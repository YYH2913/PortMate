import { describe, expect, it } from "vitest";
import { buildSessionSearchResults, filterWorkspaceSessions } from "./session-search-state";
import type { SessionEvent, SessionSummary } from "./types";

const sessions = [
  session("router", "Edge Router", "Network", ["production", "gateway"], {
    kind: "ssh",
    endpoint: { host: "10.0.0.1", port: 2222 },
    username: "admin",
  }),
  session("serial", "Bench UART", "Lab", ["hardware"], {
    kind: "serial",
    port: "/dev/ttyUSB0",
    baudRate: 115200,
  }),
];

describe("session search state", () => {
  it("matches session identity, tags, state, protocol and endpoint fields", () => {
    expect(filterWorkspaceSessions(sessions, "router").map(id)).toEqual(["router"]);
    expect(filterWorkspaceSessions(sessions, "GATEWAY").map(id)).toEqual(["router"]);
    expect(filterWorkspaceSessions(sessions, "ssh connected").map(id)).toEqual(["router"]);
    expect(filterWorkspaceSessions(sessions, "ttyUSB0 115200").map(id)).toEqual(["serial"]);
  });

  it("deduplicates duplicate hydration summaries by session ID", () => {
    const duplicate = { ...sessions[0], profile: { ...sessions[0].profile, name: "Edge Router (latest)" } };
    expect(filterWorkspaceSessions([sessions[0], duplicate], "production").map((session) => session.profile.name))
      .toEqual(["Edge Router (latest)"]);
  });

  it("builds globally sorted log results and searches session context", () => {
    const logs = {
      router: [event("old", "router", "2026-01-01T00:00:00Z", "route ready", "command-123")],
      serial: [event("new", "serial", "2026-01-02T00:00:00Z", "uart ready")],
    };
    expect(buildSessionSearchResults("logs", "ready", sessions, logs).map((result) => result.key)).toEqual([
      "log-serial-new",
      "log-router-old",
    ]);
    expect(buildSessionSearchResults("logs", "hardware", sessions, logs).map((result) => result.sessionId)).toEqual(["serial"]);
    expect(buildSessionSearchResults("logs", "command-123", sessions, logs)[0]?.detail).toContain("[命令 command-]");
  });

  it("bounds log result count and preview text", () => {
    const logs = {
      router: Array.from({ length: 90 }, (_, index) => event(
        `event-${index}`,
        "router",
        new Date(index * 1_000).toISOString(),
        index === 89 ? "x".repeat(3_000) : `line ${index}`,
      )),
    };
    const results = buildSessionSearchResults("logs", "", sessions, logs);
    expect(results).toHaveLength(80);
    expect(results[0]?.key).toBe("log-router-event-89");
    expect(Array.from(results[0]?.detail ?? "").length).toBeLessThanOrEqual(2_051);
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

function event(id: string, sessionId: string, ts: string, text: string, commandId = ""): SessionEvent {
  return {
    id,
    sessionId,
    ts,
    text,
    direction: "inbound",
    stream: "stdout",
    annotations: commandId ? { commandId } : {},
  } as SessionEvent;
}
