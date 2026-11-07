import { describe, expect, it, vi } from "vitest";
import { TerminalTimestampViewportStore } from "../../src/terminal-timestamp-viewport";

describe("terminal timestamp viewport", () => {
  it("notifies only the gutter, and only for a different visible viewport", () => {
    const store = new TerminalTimestampViewportStore();
    const notify = vi.fn();
    const unsubscribe = store.subscribe(notify);
    const viewport = { bufferType: "normal" as const, screenTop: 0, cellHeight: 18, entries: [{ row: 0, line: 0, ts: "first" }] };
    store.publish(viewport);
    store.publish(structuredClone(viewport));
    expect(notify).toHaveBeenCalledTimes(1);
    store.publish({ ...viewport, entries: [{ row: 0, line: 0, ts: "second" }] });
    expect(notify).toHaveBeenCalledTimes(2);
    unsubscribe();
    store.publish({ ...viewport, bufferType: "alternate" });
    expect(notify).toHaveBeenCalledTimes(2);
  });
});
