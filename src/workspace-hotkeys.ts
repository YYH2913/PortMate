import type { WorkspacePaneDirection, WorkspaceSplitDirection, WorkspaceSplitPlacement } from "./workspace-state";

export type WorkspaceHotkeyAction =
  | { kind: "focus"; direction: WorkspacePaneDirection }
  | { kind: "split"; direction: WorkspaceSplitDirection; placement: WorkspaceSplitPlacement }
  | { kind: "close" };

type WorkspaceHotkeyInput = {
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  code: string;
};

export function resolveWorkspaceHotkey(input: WorkspaceHotkeyInput, paneCount: number): WorkspaceHotkeyAction | null {
  if (!input.altKey || input.ctrlKey || input.metaKey) return null;
  if (!input.shiftKey && paneCount > 1) {
    const directionByCode: Partial<Record<string, WorkspacePaneDirection>> = {
      ArrowUp: "up",
      ArrowDown: "down",
      ArrowLeft: "left",
      ArrowRight: "right",
    };
    const direction = directionByCode[input.code];
    if (direction) return { kind: "focus", direction };
  }
  if (input.code === "Minus" || input.code === "Backslash") {
    return {
      kind: "split",
      direction: input.code === "Minus" ? "horizontal" : "vertical",
      placement: input.shiftKey ? "first" : "second",
    };
  }
  return input.code === "KeyX" && !input.shiftKey && paneCount > 1 ? { kind: "close" } : null;
}
