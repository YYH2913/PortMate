import { afterEach, describe, expect, it, vi } from "vitest";
import { TerminalRawCorrelator } from "../../src/terminal-raw-correlator";

afterEach(() => vi.useRealTimers());
describe("terminal raw packet correlation", () => {
  it("releases a large text-only burst with one timer and callback, in order", async () => {
    vi.useFakeTimers();
    const expired = vi.fn();
    const queue = new TerminalRawCorrelator<number>(32, expired);
    for (let i = 0; i < 10_000; i++) queue.add(String(i), i);
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(31);
    expect(expired).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(expired).toHaveBeenCalledExactlyOnceWith(Array.from({ length: 10_000 }, (_, i) => i));
    expect(vi.getTimerCount()).toBe(0);
  });
  it("never expires a matched frame and respects later arrival deadlines", async () => {
    vi.useFakeTimers();
    const expired = vi.fn();
    const queue = new TerminalRawCorrelator<string>(32, expired);
    queue.add("a", "first");
    await vi.advanceTimersByTimeAsync(10);
    queue.add("b", "matched");
    queue.add("c", "last");
    expect(queue.take("b")).toBe("matched");
    expect(queue.take("b")).toBeUndefined();
    await vi.advanceTimersByTimeAsync(22);
    expect(expired.mock.calls).toEqual([[["first"]]]);
    await vi.advanceTimersByTimeAsync(10);
    expect(expired.mock.calls).toEqual([[["first"]], [["last"]]]);
  });
  it("cancels pending work on disposal or when the last packet matches", async () => {
    vi.useFakeTimers();
    const expired = vi.fn();
    const queue = new TerminalRawCorrelator<string>(32, expired);
    queue.add("a", "a");
    expect(queue.take("a")).toBe("a");
    expect(vi.getTimerCount()).toBe(0);
    queue.add("b", "b");
    queue.dispose();
    await vi.runAllTimersAsync();
    expect(expired).not.toHaveBeenCalled();
  });
});
