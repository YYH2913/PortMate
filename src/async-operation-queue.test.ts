import { describe, expect, it } from "vitest";
import { AsyncOperationQueue } from "./async-operation-queue";

describe("async operation queue", () => {
  it("runs operations in FIFO order and preserves each result", async () => {
    let releaseFirst: (() => void) | undefined;
    const firstBlocked = new Promise<void>((resolve) => { releaseFirst = resolve; });
    const calls: string[] = [];
    const queue = new AsyncOperationQueue();
    const first = queue.enqueue(async () => {
      calls.push("first");
      await firstBlocked;
      return 1;
    });
    const second = queue.enqueue(async () => {
      calls.push("second");
      return 2;
    });

    await Promise.resolve();
    expect(calls).toEqual(["first"]);
    releaseFirst?.();
    await expect(Promise.all([first, second])).resolves.toEqual([1, 2]);
    expect(calls).toEqual(["first", "second"]);
  });

  it("continues after a rejected operation", async () => {
    const queue = new AsyncOperationQueue();
    const failed = queue.enqueue(async () => { throw new Error("failed"); });
    const recovered = queue.enqueue(async () => "recovered");

    await expect(failed).rejects.toThrow("failed");
    await expect(recovered).resolves.toBe("recovered");
  });
});
