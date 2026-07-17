import { describe, expect, it } from "vitest";
import type { SessionSummary } from "./types";
import { filterWorkspaceSessions } from "./session-search-state";
import { filterCommandHistory } from "./WorkspaceUtilityPanels";

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
    const history = ["git status --short", "docker compose\nup -d", "cargo test"];
    expect(filterCommandHistory(history, "COMPOSE UP")).toEqual(["docker compose\nup -d"]);
    expect(filterCommandHistory(history, " test ")).toEqual(["cargo test"]);
    expect(filterCommandHistory(history, "")).toEqual(history);
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
