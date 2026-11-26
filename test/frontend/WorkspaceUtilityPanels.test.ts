import { describe, expect, it } from "vitest";
import type { CommandHistoryEntry } from "../../src/command-history-state";
import type { SessionSummary } from "../../src/types";
import { DEFAULT_SEND_COUNT, DEFAULT_SEND_INTERVAL_MS, DEFAULT_SEND_LINE_ENDING, sendDraftSnapshotsEqual, shouldConfirmHistorySenderReplace } from "../../src/send-panel-state";
import { filterWorkspaceSessions } from "../../src/session-search-state";
import { filterCommandHistory, formatCommandHistoryTimestamp, getCommandHistoryScope } from "../../src/WorkspaceUtilityPanels";

const sessions = [
  session("router", "Edge Router", "Network", ["production"], {
    kind: "ssh",
    endpoint: { host: "10.0.0.1", port: 2222 },
    username: "admin",
  }),
  session("serial", "Bench UART", "Lab", ["hardware"], {
    kind: "serial",
    port: "/dev/ttyUSB0",
    baudRate: 115200,
  }),
  session("tmux", "Ops Multiplexer", "Network", ["persistent"], {
    kind: "tmux",
    endpoint: { host: "tmux.internal", port: 2200 },
    username: "ops",
  }),
  session("tcp", "Telemetry Feed", "Services", ["raw"], {
    kind: "tcp",
    host: "telemetry.local",
    port: 9000,
  }),
  session("telnet", "Legacy Console", "Services", ["legacy"], {
    kind: "telnet",
    host: "legacy.local",
    port: 23,
  }),
  session("shell", "Local Fish", "Local", ["development"], {
    kind: "shell",
    program: "/usr/bin/fish",
    args: ["--login"],
    cwd: "/srv/portmate",
  }),
];

describe("workspace utility filters", () => {
  it("matches visible session identity, grouping, tags, status, and endpoint fields", () => {
    expect(filterWorkspaceSessions(sessions, "edge").map(id)).toEqual(["router"]);
    expect(filterWorkspaceSessions(sessions, "PRODUCTION").map(id)).toEqual(["router"]);
    expect(filterWorkspaceSessions(sessions, "10.0.0.1").map(id)).toEqual(["router"]);
    expect(filterWorkspaceSessions(sessions, "connected").map(id)).toEqual(["router", "serial", "tmux", "tcp", "telnet", "shell"]);
    expect(filterWorkspaceSessions(sessions, "ttyusb0").map(id)).toEqual(["serial"]);
    expect(filterWorkspaceSessions(sessions, "tmux.internal").map(id)).toEqual(["tmux"]);
    expect(filterWorkspaceSessions(sessions, "telemetry.local 9000").map(id)).toEqual(["tcp"]);
    expect(filterWorkspaceSessions(sessions, "legacy.local 23").map(id)).toEqual(["telnet"]);
    expect(filterWorkspaceSessions(sessions, "/usr/bin/fish --login /srv").map(id)).toEqual(["shell"]);
    expect(filterWorkspaceSessions(sessions, "missing")).toEqual([]);
    expect(filterWorkspaceSessions(sessions, "  ")).toEqual(sessions);
  });

  it("filters normalized multi-line command labels without changing stored commands", () => {
    const history = [
      { command: "git status --short", recordedAt: 3, sessionId: "router" },
      { command: "docker compose\nup -d", recordedAt: 2, sessionId: "shell" },
      { command: "cargo test", recordedAt: 1, sessionId: null },
    ];
    const labels = new Map([["router", "Edge Router"], ["shell", "Local Fish"]]);
    expect(filterCommandHistory(history, "COMPOSE UP", labels)).toEqual([history[1]]);
    expect(filterCommandHistory(history, "docker   compose\nup -d", labels)).toEqual([history[1]]);
    expect(filterCommandHistory(history, " local fish ", labels)).toEqual([history[1]]);
    expect(filterCommandHistory(history, " test ", labels)).toEqual([history[2]]);
    expect(filterCommandHistory(history, "")).toEqual(history);
  });

  it("counts both scopes before filtering and shows All when there is no active session", () => {
    const entries: CommandHistoryEntry[] = [
      { command: "pwd", recordedAt: 3, sessionId: "router" },
      { command: "whoami", recordedAt: 2, sessionId: "shell" },
      { command: "date", recordedAt: 1, sessionId: null },
    ];

    expect(getCommandHistoryScope(entries, "router", "session")).toEqual({
      scope: "session",
      entries: [entries[0]],
      sessionCount: 1,
      allCount: 3,
    });
    expect(getCommandHistoryScope(entries, "router", "all")).toEqual({
      scope: "all",
      entries,
      sessionCount: 1,
      allCount: 3,
    });
    expect(getCommandHistoryScope(entries, "", "session")).toEqual({
      scope: "all",
      entries,
      sessionCount: 0,
      allCount: 3,
    });
  });

  it("formats valid history times and tolerates dates outside the JavaScript date range", () => {
    const formats = {
      compact: new Intl.DateTimeFormat("en-US", { timeZone: "UTC", month: "short", day: "numeric" }),
      full: new Intl.DateTimeFormat("en-US", { timeZone: "UTC", dateStyle: "long", timeStyle: "short" }),
    };
    const recordedAt = Date.UTC(2025, 5, 3, 16, 30);

    expect(formatCommandHistoryTimestamp(recordedAt, formats)).toEqual({
      dateTime: "2025-06-03T16:30:00.000Z",
      compact: formats.compact.format(recordedAt),
      full: formats.full.format(recordedAt),
    });
    expect(formatCommandHistoryTimestamp(Number.NaN, formats)).toBeNull();
    expect(formatCommandHistoryTimestamp(8.64e15 + 1, formats)).toBeNull();
  });

  it("confirms replacing a Send draft when text differs or Send options are non-default", () => {
    const base = {
      text: "",
      mode: "text",
      target: "active",
      count: DEFAULT_SEND_COUNT,
      intervalMs: DEFAULT_SEND_INTERVAL_MS,
      lineEnding: DEFAULT_SEND_LINE_ENDING,
    };

    expect(shouldConfirmHistorySenderReplace(base, "ls -la")).toBe(false);
    expect(shouldConfirmHistorySenderReplace({ ...base, text: "ls -la" }, "ls -la")).toBe(false);
    expect(shouldConfirmHistorySenderReplace({ ...base, text: "pwd" }, "ls -la")).toBe(true);
    expect(shouldConfirmHistorySenderReplace({ ...base, text: "31 32", mode: "hex" }, "31 32")).toBe(true);
    expect(shouldConfirmHistorySenderReplace({ ...base, mode: "hex" }, "ls -la")).toBe(true);
    expect(shouldConfirmHistorySenderReplace({ ...base, text: "ls -la", target: "all" }, "ls -la")).toBe(true);
    expect(shouldConfirmHistorySenderReplace({ ...base, text: "ls -la", count: 3 }, "ls -la")).toBe(true);
    expect(shouldConfirmHistorySenderReplace({ ...base, text: "ls -la", intervalMs: 250 }, "ls -la")).toBe(true);
    expect(shouldConfirmHistorySenderReplace({ ...base, text: "ls -la", lineEnding: "crlf" }, "ls -la")).toBe(true);

    expect(sendDraftSnapshotsEqual(base, { ...base })).toBe(true);
    expect(sendDraftSnapshotsEqual(base, { ...base, lineEnding: "lf" })).toBe(false);
  });
});

function id(value: SessionSummary): string {
  return value.profile.id;
}

function session(
  id: string,
  name: string,
  group: string,
  tags: string[],
  connection: Record<string, unknown>,
): SessionSummary {
  return {
    profile: { id, name, group, tags, kind: connection.kind, connection },
    runtime: { status: "connected" },
  } as unknown as SessionSummary;
}
