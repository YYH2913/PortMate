export type WorkspaceWindowRequest = {
  windowId: string;
};

const windowIdPattern = /^workspace-[A-Za-z0-9_-]{1,118}$/;

export function buildWorkspaceWindowPath(request: WorkspaceWindowRequest): string {
  const params = new URLSearchParams({
    workspaceWindow: "1",
    windowId: request.windowId,
  });
  return `/?${params.toString()}`;
}

export function parseWorkspaceWindowRequest(search: string): WorkspaceWindowRequest | null {
  const params = new URLSearchParams(search);
  if (params.get("workspaceWindow") !== "1") return null;
  const windowId = params.get("windowId") ?? "";
  return windowIdPattern.test(windowId) ? { windowId } : null;
}
