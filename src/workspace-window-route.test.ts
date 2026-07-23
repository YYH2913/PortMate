import { describe, expect, it } from "vitest";
import { buildWorkspaceWindowPath, parseWorkspaceWindowRequest } from "./workspace-window-route";

describe("workspace window route", () => {
  it("round-trips a restricted workspace window label", () => {
    const request = { windowId: "workspace-1f608dc1-6b1d-45df-a221-0f3f7be64c9a" };
    const path = buildWorkspaceWindowPath(request);

    expect(path).toContain("workspaceWindow=1");
    expect(parseWorkspaceWindowRequest(new URL(path, "http://localhost").search)).toEqual(request);
  });

  it("rejects missing, foreign, and unsafe labels", () => {
    expect(parseWorkspaceWindowRequest("?workspaceWindow=1")).toBeNull();
    expect(parseWorkspaceWindowRequest("?workspaceWindow=1&windowId=main")).toBeNull();
    expect(parseWorkspaceWindowRequest("?workspaceWindow=1&windowId=workspace-bad%2Flabel")).toBeNull();
    expect(parseWorkspaceWindowRequest("?workspaceWindow=1&windowId=workspace-bad%0Alabel")).toBeNull();
    expect(parseWorkspaceWindowRequest("?windowId=workspace-123")).toBeNull();
  });
});
