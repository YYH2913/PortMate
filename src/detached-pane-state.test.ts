import { describe, expect, it } from "vitest";
import {
  buildDetachedPanePath,
  DETACHED_PANE_MESSAGE_TYPE,
  normalizeDetachedPaneCommand,
  normalizeDetachedPaneMessage,
  parseDetachedPaneRequest,
} from "./detached-pane-state";

describe("detached pane state", () => {
  it("round-trips an encoded detached pane route", () => {
    const request = { windowId: "pane-123", paneId: "pane/a", viewId: "view/1", sessionId: "ssh host", title: "Router Copy" };
    const path = buildDetachedPanePath(request);

    expect(path).toContain("detachedPane=1");
    expect(parseDetachedPaneRequest(new URL(path, "http://localhost").search)).toEqual(request);
  });

  it("rejects malformed labels and control characters", () => {
    expect(parseDetachedPaneRequest("?detachedPane=1&windowId=pane%2Fbad&paneId=a&sessionId=b")).toBeNull();
    expect(parseDetachedPaneRequest("?detachedPane=1&windowId=pane-ok&paneId=a%0A&viewId=v&sessionId=b&title=x")).toBeNull();
    expect(parseDetachedPaneRequest("?detachedPane=1&windowId=pane-ok&paneId=a&viewId=v&sessionId=b&title=x%0A")).toBeNull();
    expect(parseDetachedPaneRequest("?windowId=pane-ok&paneId=a&sessionId=b")).toBeNull();
  });

  it("normalizes only supported cross-window commands", () => {
    const payload = { action: "reattach", windowId: "pane-123", paneId: "pane-a", viewId: "view-a", sessionId: "session-a", title: "Router" };

    expect(normalizeDetachedPaneCommand(payload)).toEqual(payload);
    expect(normalizeDetachedPaneCommand({ ...payload, action: "remove" })).toBeNull();
    expect(normalizeDetachedPaneMessage({ type: DETACHED_PANE_MESSAGE_TYPE, payload })).toEqual({
      type: DETACHED_PANE_MESSAGE_TYPE,
      payload,
    });
    expect(normalizeDetachedPaneMessage({ type: "other", payload })).toBeNull();
  });
});
