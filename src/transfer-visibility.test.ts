import { describe, expect, it } from "vitest";
import {
  addDismissedTransferId,
  COMPLETED_TRANSFER_AUTO_DISMISS_MS,
  completedTransferDismissDeadline,
} from "./transfer-visibility";

describe("completed transfer visibility", () => {
  const observedAt = Date.parse("2026-07-31T02:00:00.000Z");

  it("starts the dismissal window at the recorded completion time", () => {
    const finishedAt = "2026-07-31T01:59:58.000Z";
    expect(completedTransferDismissDeadline(finishedAt, observedAt)).toBe(
      Date.parse(finishedAt) + COMPLETED_TRANSFER_AUTO_DISMISS_MS,
    );
  });

  it("expires restored transfers that completed before the window", () => {
    expect(completedTransferDismissDeadline("2026-07-31T01:00:00.000Z", observedAt)).toBeLessThan(
      observedAt,
    );
  });

  it("does not extend the window for a future-skewed timestamp", () => {
    expect(completedTransferDismissDeadline("2026-08-01T02:00:00.000Z", observedAt)).toBe(
      observedAt + COMPLETED_TRANSFER_AUTO_DISMISS_MS,
    );
  });

  it.each([null, undefined, "invalid"])("falls back for %s completion time", (finishedAt) => {
    expect(completedTransferDismissDeadline(finishedAt, observedAt)).toBe(
      observedAt + COMPLETED_TRANSFER_AUTO_DISMISS_MS,
    );
  });

  it("shares idempotent dismissals through a bounded insertion-ordered set", () => {
    const first = addDismissedTransferId(new Set(), "first", 2);
    expect(addDismissedTransferId(first, "first", 2)).toBe(first);
    const second = addDismissedTransferId(first, "second", 2);
    const third = addDismissedTransferId(second, "third", 2);
    expect([...third]).toEqual(["second", "third"]);
  });
});
