import { afterEach, describe, expect, it } from "vitest";
import {
  dispatchTerminalByteEventForTests,
  resetTerminalByteEventBridgeForTests,
  subscribeTerminalByteEvents,
} from "./terminal-byte-events";
import { resetTerminalByteCacheForTests } from "./terminal-byte-state";
import type { TerminalBytesEvent } from "./types";

function frame(id: string, bytes: number[], sessionId = "session-a"): TerminalBytesEvent {
  return {
    id,
    sessionId,
    ts: "2026-08-24T08:00:00.000Z",
    direction: "inbound",
    stream: "stdout",
    bytes,
    originalLength: bytes.length,
    truncated: false,
    eventId: `event-${id}`,
  };
}

afterEach(() => {
  resetTerminalByteEventBridgeForTests();
  resetTerminalByteCacheForTests();
});

describe("terminal byte event bridge", () => {
  it("delivers valid frames synchronously and in transport order", () => {
    const received: string[] = [];
    const unsubscribe = subscribeTerminalByteEvents("session-a", (event) => received.push(event.id));

    expect(dispatchTerminalByteEventForTests(frame("first", [1, 2]))).toBe(true);
    expect(dispatchTerminalByteEventForTests(frame("second", [3, 4]))).toBe(true);
    expect(received).toEqual(["first", "second"]);
    unsubscribe();
  });

  it("replays the bounded cache when a terminal mounts after output arrives", () => {
    expect(dispatchTerminalByteEventForTests(frame("before-mount", [0xe5, 0xad]))).toBe(true);
    const received: TerminalBytesEvent[] = [];
    const unsubscribe = subscribeTerminalByteEvents("session-a", (event) => received.push(event));

    expect(received.map((event) => event.id)).toEqual(["before-mount"]);
    expect(received[0]?.bytes).toEqual([0xe5, 0xad]);
    unsubscribe();
  });

  it("rejects malformed payloads and does not redispatch duplicate frame ids", () => {
    const received: string[] = [];
    const unsubscribe = subscribeTerminalByteEvents("session-a", (event) => received.push(event.id));

    expect(dispatchTerminalByteEventForTests({ ...frame("invalid", [1]), bytes: [256] })).toBe(false);
    expect(dispatchTerminalByteEventForTests(frame("shared", [1]))).toBe(true);
    expect(dispatchTerminalByteEventForTests(frame("shared", [1]))).toBe(false);
    expect(received).toEqual(["shared"]);
    unsubscribe();
  });
});
