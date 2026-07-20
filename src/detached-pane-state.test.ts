import { describe, expect, it } from "vitest";
import {
  buildDetachedPanePath,
  DETACHED_PANE_MESSAGE_TYPE,
  normalizeDetachedPaneCommand,
  normalizeDetachedPaneMessage,
  parseDetachedPaneRequest,
  upsertDetachedSessionSummary,
} from "./detached-pane-state";
import type { SessionSummary } from "./types";

describe("detached pane state", () => {
  it("round-trips an encoded detached pane route", () => {
    const request = { windowId: "pane-123", paneId: "pane/a", viewId: "view/1", sessionId: "ssh host", title: "Router Copy", color: "#228B22", keyMode: "command" as const };
    const path = buildDetachedPanePath(request);

    expect(path).toContain("detachedPane=1");
    expect(parseDetachedPaneRequest(new URL(path, "http://localhost").search)).toEqual(request);
    expect(parseDetachedPaneRequest("?detachedPane=1&windowId=pane-old&paneId=a&viewId=v&sessionId=b&title=Old")).toEqual({
      windowId: "pane-old",
      paneId: "a",
      viewId: "v",
      sessionId: "b",
      title: "Old",
      color: "",
      keyMode: "remote",
    });
  });

  it("rejects malformed labels and control characters", () => {
    expect(parseDetachedPaneRequest("?detachedPane=1&windowId=pane%2Fbad&paneId=a&sessionId=b")).toBeNull();
    expect(parseDetachedPaneRequest("?detachedPane=1&windowId=pane-ok&paneId=a%0A&viewId=v&sessionId=b&title=x")).toBeNull();
    expect(parseDetachedPaneRequest("?detachedPane=1&windowId=pane-ok&paneId=a&viewId=v&sessionId=b&title=x%0A")).toBeNull();
    expect(parseDetachedPaneRequest("?detachedPane=1&windowId=pane-ok&paneId=a&viewId=v&sessionId=b&title=x&color=red")).toBeNull();
    expect(parseDetachedPaneRequest("?windowId=pane-ok&paneId=a&sessionId=b")).toBeNull();
  });

  it("normalizes only supported cross-window commands", () => {
    const payload = { action: "reattach", windowId: "pane-123", paneId: "pane-a", viewId: "view-a", sessionId: "session-a", title: "Router", color: "#4169E1", keyMode: "local" };

    expect(normalizeDetachedPaneCommand(payload)).toEqual(payload);
    expect(normalizeDetachedPaneCommand({ ...payload, keyMode: "invalid" })).toMatchObject({ keyMode: "remote" });
    expect(normalizeDetachedPaneCommand({ ...payload, action: "remove" })).toBeNull();
    expect(normalizeDetachedPaneMessage({ type: DETACHED_PANE_MESSAGE_TYPE, payload })).toEqual({
      type: DETACHED_PANE_MESSAGE_TYPE,
      payload,
    });
    expect(normalizeDetachedPaneMessage({ type: "other", payload })).toBeNull();
  });

  it("accepts a global lock request from a detached window", () => {
    const payload = { action: "lock-screen", windowId: "pane-123", paneId: "pane-a", viewId: "view-a", sessionId: "session-a", title: "Router", color: "#4169E1", keyMode: "remote" };

    expect(normalizeDetachedPaneCommand(payload)).toEqual(payload);
    expect(normalizeDetachedPaneMessage({ type: DETACHED_PANE_MESSAGE_TYPE, payload })).toEqual({
      type: DETACHED_PANE_MESSAGE_TYPE,
      payload,
    });
  });

  it("upserts a detached session after a profile update", () => {
    const first = { profile: { id: "session-a", name: "Before" } } as SessionSummary;
    const second = { profile: { id: "session-b", name: "Other" } } as SessionSummary;
    const updated = { profile: { id: "session-a", name: "After" } } as SessionSummary;
    const sessions = [first, second];

    const next = upsertDetachedSessionSummary(sessions, updated);
    expect(next).toEqual([updated, second]);
    expect(next).not.toBe(sessions);

    const unknown = { profile: { id: "session-c", name: "Unknown" } } as SessionSummary;
    expect(upsertDetachedSessionSummary(sessions, unknown)).toEqual([first, second, unknown]);
  });
});
