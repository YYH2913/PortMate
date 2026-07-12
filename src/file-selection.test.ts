import { describe, expect, it } from "vitest";
import { updateFileSelection } from "./file-selection";

const entries = ["alpha", "beta", "gamma", "delta"].map((path) => ({ path }));
const plain = { shiftKey: false, ctrlKey: false, metaKey: false };

describe("updateFileSelection", () => {
  it("replaces selection on a plain click", () => {
    const result = updateFileSelection(entries, [entries[0]], entries[2], entries[0].path, plain);
    expect(result.selected).toEqual([entries[2]]);
    expect(result.anchorPath).toBe("gamma");
  });

  it("toggles entries with Ctrl or Command", () => {
    const added = updateFileSelection(entries, [entries[0]], entries[2], entries[0].path, { ...plain, ctrlKey: true });
    expect(added.selected).toEqual([entries[0], entries[2]]);
    const removed = updateFileSelection(entries, added.selected, entries[0], added.anchorPath, { ...plain, metaKey: true });
    expect(removed.selected).toEqual([entries[2]]);
  });

  it("selects an inclusive range in either direction", () => {
    const forward = updateFileSelection(entries, [], entries[3], "beta", { ...plain, shiftKey: true });
    expect(forward.selected).toEqual(entries.slice(1, 4));
    const backward = updateFileSelection(entries, [], entries[0], "gamma", { ...plain, shiftKey: true });
    expect(backward.selected).toEqual(entries.slice(0, 3));
  });

  it("falls back to a plain selection when the anchor is stale", () => {
    const result = updateFileSelection(entries, [entries[0]], entries[1], "missing", { ...plain, shiftKey: true });
    expect(result).toEqual({ selected: [entries[1]], anchorPath: "beta" });
  });
});
