import { describe, expect, it } from "vitest";
import { reconcileSessionSummaries } from "../../src/session-summary-identity";
import type { SessionSummary } from "../../src/types";

describe("session-summary identity", () => {
  it("does not replace unchanged profiles and connection state on status polling", () => {
    const old = { profile: { id: "one", name: "One", terminal: { cols: 80 } }, runtime: { status: "connected", lastActivity: "old" }, logLines: 1, lastLine: "old" } as SessionSummary;
    const current = [old];
    expect(reconcileSessionSummaries(current, structuredClone(current))).toBe(current);
    const next = structuredClone(current);
    next[0].runtime.lastActivity = "new";
    next[0].logLines = 10;
    const reconciled = reconcileSessionSummaries(current, next);
    expect(reconciled[0].profile).toBe(old.profile);
    expect(reconciled[0].runtime.lastActivity).toBe("new");
    next[0].profile.name = "New";
    expect(reconcileSessionSummaries(current, next)[0].profile).not.toBe(old.profile);
    expect(reconcileSessionSummaries(current, [])).toEqual([]);
  });
});
