import type { OneKeySummary, SessionEvent } from "./types";

export const MAX_ONE_KEY_PROMPT_BUFFER_CHARACTERS = 1_024;

export type OneKeyPromptField = "username" | "password";

export interface OneKeyTerminalPrompt {
  eventId: string;
  field: OneKeyPromptField;
  line: string;
  usernameHint: string | null;
}

export interface OneKeyPromptDetectionState {
  raw: string;
  prompt: OneKeyTerminalPrompt | null;
}

export const emptyOneKeyPromptDetectionState = (): OneKeyPromptDetectionState => ({
  raw: "",
  prompt: null,
});

export function reduceOneKeyPromptDetection(
  state: OneKeyPromptDetectionState,
  event: SessionEvent,
): OneKeyPromptDetectionState {
  if (event.direction === "outbound") return emptyOneKeyPromptDetectionState();
  if (
    event.direction !== "inbound"
    || (event.stream !== "stdout" && event.stream !== "stderr")
    || !event.text
  ) {
    return state;
  }
  const raw = trimPromptBuffer(state.raw + event.text);
  return {
    raw,
    prompt: detectOneKeyTerminalPrompt(raw, event.id),
  };
}

export function oneKeyPromptStateFromEvents(
  events: readonly SessionEvent[],
  sessionId: string,
): OneKeyPromptDetectionState {
  return events
    .filter((event) => event.sessionId === sessionId)
    .reduce(reduceOneKeyPromptDetection, emptyOneKeyPromptDetectionState());
}

export function detectOneKeyTerminalPrompt(
  raw: string,
  eventId: string,
): OneKeyTerminalPrompt | null {
  const display = sanitizeTerminalPromptText(raw)
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n");
  const line = display.split("\n").at(-1)?.trimEnd().slice(-160) ?? "";
  if (!line) return null;
  if (/\b(?:new|retype|repeat|confirm)\s+(?:new\s+)?password(?:\s+for\s+\S+)?\s*:\s*$/i.test(line)) {
    return null;
  }

  const passwordFor = line.match(/\bpassword\s+for\s+([^\s:]+)\s*:\s*$/i);
  if (passwordFor) {
    return { eventId, field: "password", line, usernameHint: passwordFor[1] };
  }
  const opensshPassword = line.match(/(?:^|\s)([^\s@]+)@\S+(?:'s)?\s+password\s*:\s*$/i);
  if (opensshPassword) {
    return { eventId, field: "password", line, usernameHint: opensshPassword[1] };
  }
  if (/\bpassword\s*:\s*$/i.test(line)) {
    return { eventId, field: "password", line, usernameHint: null };
  }
  if (/\b(?:username|login)\s*:\s*$/i.test(line)) {
    return { eventId, field: "username", line, usernameHint: null };
  }
  return null;
}

export function oneKeyPromptCandidates(
  oneKeys: readonly OneKeySummary[],
  sessionId: string,
  prompt: OneKeyTerminalPrompt,
): OneKeySummary[] {
  const candidates = oneKeys.filter((oneKey) => (
    oneKey.sessionIds.includes(sessionId)
    && (prompt.field === "username" || oneKey.hasPassword)
  ));
  if (prompt.field !== "password" || !prompt.usernameHint) return candidates;
  return candidates.filter((oneKey) => oneKey.username === prompt.usernameHint);
}

export function sanitizeTerminalPromptText(raw: string): string {
  const output: string[] = [];
  for (let index = 0; index < raw.length; index += 1) {
    const character = raw[index];
    if (character === "\x1b") {
      const introducer = raw[index + 1];
      if (introducer === "[") {
        index += 2;
        while (index < raw.length && !/[\x40-\x7e]/.test(raw[index])) index += 1;
        continue;
      }
      if (introducer === "]" || introducer === "P" || introducer === "^" || introducer === "_") {
        index += 2;
        while (index < raw.length) {
          if (raw[index] === "\x07") break;
          if (raw[index] === "\x1b" && raw[index + 1] === "\\") {
            index += 1;
            break;
          }
          index += 1;
        }
        continue;
      }
      index += introducer ? 1 : 0;
      continue;
    }
    if (character === "\b" || character === "\x7f") {
      if (output.length && output.at(-1) !== "\n" && output.at(-1) !== "\r") output.pop();
      continue;
    }
    if (character === "\n" || character === "\r" || character === "\t" || character >= " ") {
      output.push(character);
    }
  }
  return output.join("");
}

function trimPromptBuffer(raw: string): string {
  return [...raw].slice(-MAX_ONE_KEY_PROMPT_BUFFER_CHARACTERS).join("");
}
