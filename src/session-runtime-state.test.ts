import { describe, expect, it } from "vitest";
import {
  sessionConnectionAction,
  sessionRuntimeHealthDescription,
  sessionRuntimeStatusLabel,
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
