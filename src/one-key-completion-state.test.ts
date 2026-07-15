import { describe, expect, it } from "vitest";
import {
  detectOneKeyTerminalPrompt,
  emptyOneKeyPromptDetectionState,
  oneKeyPromptCandidates,
  oneKeyPromptStateFromEvents,
  reduceOneKeyPromptDetection,
  sanitizeTerminalPromptText,
} from "./one-key-completion-state";
import type { OneKeySummary, SessionEvent } from "./types";

function event(
  id: string,
  text: string | null,
  direction: SessionEvent["direction"] = "inbound",
  stream: SessionEvent["stream"] = "stdout",
): SessionEvent {
  return {
    id,
    sessionId: "session-a",
    paneId: "session-a:main",
    ts: "2026-07-15T00:00:00Z",
    direction,
    stream,
    text,
    annotations: {},
  };
}

const oneKeys: OneKeySummary[] = [
  {
    id: "onekey:root",
    label: "Root",
    kind: "account",
    username: "root",
    hasPassword: true,
    hasPassphrase: false,
    sessionIds: ["session-a"],
    createdAt: "2026-07-15T00:00:00Z",
    updatedAt: "2026-07-15T00:00:00Z",
  },
  {
    id: "onekey:operator",
    label: "Operator",
    kind: "ssh",
    username: "operator",
    hasPassword: false,
    hasPassphrase: true,
    sessionIds: ["session-a"],
    createdAt: "2026-07-15T00:00:00Z",
    updatedAt: "2026-07-15T00:00:00Z",
  },
  {
    id: "onekey:other",
    label: "Other",
    kind: "account",
    username: "root",
    hasPassword: true,
    hasPassphrase: false,
    sessionIds: ["session-b"],
    createdAt: "2026-07-15T00:00:00Z",
    updatedAt: "2026-07-15T00:00:00Z",
  },
];

describe("OneKey terminal prompt completion", () => {
  it("detects fragmented ANSI username and password prompts at the current line end", () => {
    let state = reduceOneKeyPromptDetection(emptyOneKeyPromptDetectionState(), event("a", "\x1b[33mPass"));
    expect(state.prompt).toBeNull();
    state = reduceOneKeyPromptDetection(state, event("b", "word:\x1b[0m"));
    expect(state.prompt).toMatchObject({ eventId: "b", field: "password", usernameHint: null });

    expect(detectOneKeyTerminalPrompt("device login:", "c"))
      .toMatchObject({ field: "username", line: "device login:" });
    expect(detectOneKeyTerminalPrompt("Password:\r\n", "d")).toBeNull();
  });

  it("extracts WindTerm username hints and handles terminal erasure", () => {
    expect(detectOneKeyTerminalPrompt("password for root:", "a")?.usernameHint).toBe("root");
    expect(detectOneKeyTerminalPrompt("root@router's password:", "b")?.usernameHint).toBe("root");
    expect(detectOneKeyTerminalPrompt("New password:", "c")).toBeNull();
    expect(detectOneKeyTerminalPrompt("Retype new password:", "d")).toBeNull();
    expect(detectOneKeyTerminalPrompt("Confirm password for root:", "e")).toBeNull();
    expect(sanitizeTerminalPromptText("PasswX\bord:")).toBe("Password:");
    expect(sanitizeTerminalPromptText("\x1b]0;password:\x07Login:")).toBe("Login:");
  });

  it("clears a pending prompt after any outbound input and replays current state", () => {
    const events = [event("a", "Pass"), event("b", "word:"), event("c", null, "system", "control")];
    expect(oneKeyPromptStateFromEvents(events, "session-a").prompt?.eventId).toBe("b");
    expect(oneKeyPromptStateFromEvents([...events, event("d", null, "outbound", "control")], "session-a").prompt)
      .toBeNull();
  });

  it("limits candidates to the bound session, required field, and exact username hint", () => {
    const usernamePrompt = detectOneKeyTerminalPrompt("login:", "u")!;
    expect(oneKeyPromptCandidates(oneKeys, "session-a", usernamePrompt).map((item) => item.id))
      .toEqual(["onekey:root", "onekey:operator"]);

    const genericPassword = detectOneKeyTerminalPrompt("password:", "p")!;
    expect(oneKeyPromptCandidates(oneKeys, "session-a", genericPassword).map((item) => item.id))
      .toEqual(["onekey:root"]);

    const hintedPassword = detectOneKeyTerminalPrompt("password for operator:", "h")!;
    expect(oneKeyPromptCandidates(oneKeys, "session-a", hintedPassword)).toEqual([]);
  });
});
