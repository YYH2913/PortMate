import { afterEach, describe, expect, it, vi } from "vitest";
import { BackgroundConnectionQueue } from "../../src/background-connection-queue";

afterEach(() => vi.useRealTimers());
describe("background startup connections", () => {
  it("yields first, connects concurrently with one shared limit and does not duplicate targets", async () => {
    vi.useFakeTimers();
    const queue = new BackgroundConnectionQueue(4);
    const releases = new Map<string, () => void>();
    const started: string[] = [];
    const connect = (id: string) => () => new Promise<void>(resolve => { started.push(id); releases.set(id, resolve); });
    for (let index = 0; index < 8; index++) queue.enqueue(String(index), connect(String(index)));
    expect(started).toEqual([]);
    await vi.advanceTimersByTimeAsync(0);
    expect(started).toEqual(["0", "1", "2", "3"]);
    queue.enqueue("0", connect("0"));
    queue.enqueue("8", connect("8"));
    releases.get("1")!();
    await vi.advanceTimersByTimeAsync(1);
    expect(started).toEqual(["0", "1", "2", "3", "4"]);
    queue.dispose();
    releases.forEach(release => release());
    await vi.runAllTimersAsync();
    expect(started).toHaveLength(5);
  });
  it("continues after failures and cancels queued sessions without reconnecting them", async () => {
    vi.useFakeTimers();
    const queue = new BackgroundConnectionQueue(2);
    const started: string[] = [];
    queue.enqueue("bad", async () => { started.push("bad"); throw Error("silent failure"); });
    queue.enqueue("good", async () => { started.push("good"); });
    queue.enqueue("cancelled", async () => { started.push("cancelled"); });
    queue.cancel("cancelled");
    queue.enqueue("after", async () => { started.push("after"); });
    await vi.runAllTimersAsync();
    expect(started).toEqual(["bad", "good", "after"]);
    queue.dispose();
  });
});
