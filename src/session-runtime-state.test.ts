import { describe, expect, it } from "vitest";
import {
  sessionConnectionAction,
  sessionRuntimeDisconnectDescription,
  sessionRuntimeHealthDescription,
  sessionRuntimeStatusLabel,
  transitionSessionRuntimeStatus,
} from "./session-runtime-state";
import type { SessionRuntime, SessionStatus } from "./types";

describe("session runtime state", () => {
  it("allows active and pending transports to be cancelled", () => {
    expect(sessionConnectionAction("connected")).toBe("disconnect");
    expect(sessionConnectionAction("connecting")).toBe("disconnect");
    expect(sessionConnectionAction("reconnecting")).toBe("disconnect");
  });

  it("offers connection for inactive terminal states", () => {
    expect(sessionConnectionAction("disconnected")).toBe("connect");
    expect(sessionConnectionAction("blocked")).toBe("connect");
    expect(sessionConnectionAction("error")).toBe("connect");
  });

  it("labels every runtime state for compact diagnostics", () => {
    const labels = ([
      "disconnected",
      "connecting",
      "connected",
      "reconnecting",
      "blocked",
      "error",
    ] as SessionStatus[]).map(sessionRuntimeStatusLabel);
    expect(labels).toEqual(["已断开", "正在连接", "已连接", "正在重连", "连接已阻止", "连接错误"]);
  });

  it("includes the previous disconnect timestamp and reason after reconnecting", () => {
    const runtime = createRuntime({
      status: "connected",
      lastDisconnect: "2026-07-22T00:00:00.000Z",
      lastDisconnectReason: "SSH keepalive timeout",
    });
    expect(sessionRuntimeHealthDescription(runtime, (value) => value)).toBe(
      "已连接 · 上次断开 2026-07-22T00:00:00.000Z · 原因: SSH keepalive timeout",
    );
  });

  it("omits invalid timestamps and normalizes multiline reasons", () => {
    const runtime = createRuntime({
      status: "reconnecting",
      lastDisconnect: "invalid",
      lastDisconnectReason: " socket\n  closed ",
    });
    expect(sessionRuntimeHealthDescription(runtime)).toBe("正在重连 · 原因: socket closed");
  });

  it("bounds untrusted disconnect diagnostics", () => {
    const runtime = createRuntime({
      status: "error",
      lastDisconnectReason: "x".repeat(300),
    });
    const description = sessionRuntimeHealthDescription(runtime);
    expect(description.endsWith("...")).toBe(true);
    expect(Array.from(description.split("原因: ")[1]).length).toBe(256);
  });

  it("provides bounded disconnect details without duplicating the status", () => {
    const runtime = createRuntime({
      status: "connected",
      lastDisconnect: "invalid",
      lastDisconnectReason: ` serial\n  cable ${"x".repeat(300)} `,
    });
    const description = sessionRuntimeDisconnectDescription(runtime);
    expect(description.startsWith("原因: serial cable ")).toBe(true);
    expect(description).not.toContain("Invalid Date");
    expect(description).not.toContain("已连接");
    expect(description).not.toContain("\n");
    expect(description.endsWith("...")).toBe(true);
    expect(Array.from(description.slice("原因: ".length)).length).toBe(256);
  });

  it("omits disconnect details when no valid diagnostics exist", () => {
    expect(sessionRuntimeDisconnectDescription(createRuntime({ status: "connected" }))).toBe("");
    expect(sessionRuntimeDisconnectDescription(createRuntime({
      status: "connected",
      lastDisconnect: "invalid",
      lastDisconnectReason: "  ",
    }))).toBe("");
  });

  it("does not invent a disconnect during the first successful connection", () => {
    const initial = createRuntime({ status: "disconnected" });
    const connecting = transitionSessionRuntimeStatus(
      { ...initial, lastDisconnect: null, lastDisconnectReason: null },
      "connecting",
      "2026-07-22T00:00:01.000Z",
    );
    expect(connecting.lastDisconnect).toBeNull();
    expect(connecting.lastDisconnectReason).toBeNull();

    const connected = transitionSessionRuntimeStatus(
      connecting,
      "connected",
      "2026-07-22T00:00:02.000Z",
    );
    expect(connected.lastDisconnect).toBeNull();
    expect(connected.lastDisconnectReason).toBeNull();
    expect(connected.connectedSince).toBe("2026-07-22T00:00:02.000Z");
  });

  it("preserves the first timestamp while an outage reason evolves", () => {
    const first = transitionSessionRuntimeStatus(
      createRuntime({ status: "connected" }),
      "reconnecting",
      "2026-07-22T00:01:00.000Z",
      "socket closed",
    );
    const retried = transitionSessionRuntimeStatus(
      first,
      "reconnecting",
      "2026-07-22T00:02:00.000Z",
      "reconnect refused",
    );
    const stopped = transitionSessionRuntimeStatus(
      retried,
      "disconnected",
      "2026-07-22T00:03:00.000Z",
      "automatic reconnect disabled",
    );
    expect(stopped.lastDisconnect).toBe("2026-07-22T00:01:00.000Z");
    expect(stopped.lastDisconnectReason).toBe("automatic reconnect disabled");

    const recovered = transitionSessionRuntimeStatus(
      transitionSessionRuntimeStatus(stopped, "connecting", "2026-07-22T00:04:00.000Z"),
      "connected",
      "2026-07-22T00:05:00.000Z",
    );
    const nextOutage = transitionSessionRuntimeStatus(
      recovered,
      "error",
      "2026-07-22T00:06:00.000Z",
      "new transport failure",
    );
    expect(nextOutage.lastDisconnect).toBe("2026-07-22T00:06:00.000Z");
    expect(nextOutage.lastDisconnectReason).toBe("new transport failure");
  });

  it("records the actual reason when a connection attempt fails", () => {
    const failed = transitionSessionRuntimeStatus(
      createRuntime({ status: "connecting", lastDisconnect: null, lastDisconnectReason: null }),
      "error",
      "2026-07-22T00:07:00.000Z",
      "proxy authentication failed",
    );
    expect(failed.lastDisconnect).toBe("2026-07-22T00:07:00.000Z");
    expect(failed.lastDisconnectReason).toBe("proxy authentication failed");
  });

  it("normalizes and bounds fallback reasons before storing runtime state", () => {
    const failed = transitionSessionRuntimeStatus(
      createRuntime({ status: "connecting", lastDisconnect: null, lastDisconnectReason: null }),
      "error",
      "2026-07-22T00:08:00.000Z",
      `  proxy\n\tauth ${"界".repeat(300)}  `,
    );
    expect(failed.lastDisconnectReason?.startsWith("proxy auth 界")).toBe(true);
    expect(failed.lastDisconnectReason?.endsWith("...")).toBe(true);
    expect(failed.lastDisconnectReason).not.toContain("\n");
    expect(Array.from(failed.lastDisconnectReason ?? "")).toHaveLength(256);

    const whitespaceOnly = transitionSessionRuntimeStatus(
      failed,
      "error",
      "2026-07-22T00:09:00.000Z",
      " \n\t ".repeat(100_000),
    );
    expect(whitespaceOnly.lastDisconnectReason).toBe("connection error");
  });

  it("uses the native Store disconnect defaults in browser fallback", () => {
    const initial = createRuntime({ status: "connected" });
    expect(transitionSessionRuntimeStatus(
      initial,
      "disconnected",
      "2026-07-22T00:10:00.000Z",
    ).lastDisconnectReason).toBe("session disconnected");
    expect(transitionSessionRuntimeStatus(
      initial,
      "reconnecting",
      "2026-07-22T00:11:00.000Z",
    ).lastDisconnectReason).toBe("session reconnecting");
  });
});

function createRuntime(patch: Partial<SessionRuntime>): SessionRuntime {
  return {
    sessionId: "edge-router",
    paneId: "edge-router:main",
    status: "disconnected",
    title: "Edge Router",
    cwd: null,
    connectedSince: null,
    lastActivity: "2026-07-22T00:00:00.000Z",
    lastDisconnect: null,
    lastDisconnectReason: null,
    activeTransport: "ssh",
    ...patch,
  };
}
