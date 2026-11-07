import { afterEach, describe, expect, it, vi } from "vitest";
import { TerminalTaskScheduler } from "../../src/terminal-task-scheduler";

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });
describe("terminal task scheduler", () => {
  it("coalesces work and yields before running or rescheduling it", async () => {
    const order: string[] = [];
    let runs = 0;
    await new Promise<void>(resolve => {
      const queue = new TerminalTaskScheduler(() => {
        expect(queue.pending).toBe(false);
        order.push("run");
        if (++runs === 2) { queue.dispose(); resolve(); }
        else queue.schedule();
      });
      queue.schedule();
      queue.schedule();
      expect(queue.pending).toBe(true);
      order.push("caller");
    });
    expect(order).toEqual(["caller", "run", "run"]);
  });
  it("uses cancellable timers only when MessageChannel is unavailable", async () => {
    vi.stubGlobal("MessageChannel", undefined);
    vi.useFakeTimers();
    const callback = vi.fn();
    const queue = new TerminalTaskScheduler(callback);
    queue.schedule();
    queue.schedule();
    expect(vi.getTimerCount()).toBe(1);
    await vi.runAllTimersAsync();
    expect(callback).toHaveBeenCalledOnce();
    queue.schedule();
    queue.dispose();
    queue.schedule();
    await vi.runAllTimersAsync();
    expect(callback).toHaveBeenCalledOnce();
    expect(queue.pending).toBe(false);
  });
});
