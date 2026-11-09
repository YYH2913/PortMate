import { afterEach, describe, expect, it, vi } from "vitest";
import { appendTerminalByteCacheEvents, clearTerminalByteCache, resetTerminalByteCacheForTests, subscribeTerminalBytePresentation, terminalByteCacheSnapshot, terminalBytePresentationSnapshot } from "../../src/terminal-byte-state";
import type { TerminalBytesEvent } from "../../src/types";

function frame(id: string): TerminalBytesEvent {
  return { id, sessionId: "s", ts: "2026-11-08T00:00:00Z", direction: "inbound", stream: "stdout", bytes: [13, 10], originalLength: 2, truncated: false };
}
afterEach(() => { resetTerminalByteCacheForTests(); vi.useRealTimers(); });
describe("RX/TX presentation isolation", () => {
  it("captures every frame immediately but coalesces only auxiliary UI notifications", async () => {
    vi.useFakeTimers();
    const notify = vi.fn();
    const stop = subscribeTerminalBytePresentation("s", notify);
    const initial = terminalBytePresentationSnapshot("s");
    for (let index = 0; index < 100; index++) appendTerminalByteCacheEvents([frame(String(index))]);
    expect(terminalByteCacheSnapshot("s").frames).toHaveLength(100);
    expect(terminalBytePresentationSnapshot("s")).toBe(initial);
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(100);
    expect(notify).toHaveBeenCalledOnce();
    expect(terminalBytePresentationSnapshot("s")).toBe(terminalByteCacheSnapshot("s"));
    stop();
  });
  it("clears immediately and cannot resurrect a scheduled old snapshot", async () => {
    vi.useFakeTimers();
    const notify = vi.fn();
    const stop = subscribeTerminalBytePresentation("s", notify);
    appendTerminalByteCacheEvents([frame("old")]);
    clearTerminalByteCache("s");
    expect(notify).toHaveBeenCalledOnce();
    expect(terminalBytePresentationSnapshot("s").capturedBytes).toBe(0);
    await vi.runAllTimersAsync();
    expect(notify).toHaveBeenCalledOnce();
    appendTerminalByteCacheEvents([frame("new")]);
    stop();
    expect(vi.getTimerCount()).toBe(0);
    expect(terminalBytePresentationSnapshot("s")).toBe(terminalByteCacheSnapshot("s"));
  });
});
