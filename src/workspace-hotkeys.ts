import type { WorkspacePaneDirection, WorkspaceSplitDirection, WorkspaceSplitPlacement } from "./workspace-state";

export const WORKSPACE_KEYMAP_STORAGE_KEY = "portmate.workspaceKeymap.v2";
export const LEGACY_WORKSPACE_KEYMAP_STORAGE_KEY = "portmate.workspaceKeymap.v1";
export const WORKSPACE_KEY_CHORD_TIMEOUT_MS = 1_200;

export type WorkspaceHotkeyAction =
  | { kind: "focus"; direction: WorkspacePaneDirection }
  | { kind: "split"; direction: WorkspaceSplitDirection; placement: WorkspaceSplitPlacement }
  | { kind: "close" }
  | { kind: "zoom" }
  | { kind: "one-keys" };

export type WorkspaceHotkeyCommandId =
  | "focus-up"
  | "focus-down"
  | "focus-left"
  | "focus-right"
  | "split-up"
  | "split-down"
  | "split-left"
  | "split-right"
  | "close-pane"
  | "zoom-pane"
  | "manage-one-keys";

export type WorkspaceKeymap = Record<WorkspaceHotkeyCommandId, string>;

export type WorkspaceKeymapConflict = {
  binding: string;
  commandIds: WorkspaceHotkeyCommandId[];
  kind: "duplicate" | "prefix";
};

export type WorkspaceHotkeyResolution =
  | { kind: "action"; action: WorkspaceHotkeyAction }
  | { kind: "pending"; prefix: string }
  | { kind: "none" };

type WorkspaceHotkeyInput = {
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  code: string;
};

type WorkspaceHotkeyCommand = {
  id: WorkspaceHotkeyCommandId;
  label: string;
  defaultBinding: string;
  requiresMultiplePanes: boolean;
  action: WorkspaceHotkeyAction;
};

const primaryChordModifier = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform)
  ? "Meta"
  : "Ctrl";

export const workspaceHotkeyCommands: readonly WorkspaceHotkeyCommand[] = [
  { id: "focus-up", label: "焦点向上", defaultBinding: "Alt+ArrowUp", requiresMultiplePanes: true, action: { kind: "focus", direction: "up" } },
  { id: "focus-down", label: "焦点向下", defaultBinding: "Alt+ArrowDown", requiresMultiplePanes: true, action: { kind: "focus", direction: "down" } },
  { id: "focus-left", label: "焦点向左", defaultBinding: "Alt+ArrowLeft", requiresMultiplePanes: true, action: { kind: "focus", direction: "left" } },
  { id: "focus-right", label: "焦点向右", defaultBinding: "Alt+ArrowRight", requiresMultiplePanes: true, action: { kind: "focus", direction: "right" } },
  { id: "split-up", label: "向上拆分", defaultBinding: "Alt+Shift+Minus", requiresMultiplePanes: false, action: { kind: "split", direction: "horizontal", placement: "first" } },
  { id: "split-down", label: "向下拆分", defaultBinding: "Alt+Minus", requiresMultiplePanes: false, action: { kind: "split", direction: "horizontal", placement: "second" } },
  { id: "split-left", label: "向左拆分", defaultBinding: "Alt+Shift+Backslash", requiresMultiplePanes: false, action: { kind: "split", direction: "vertical", placement: "first" } },
  { id: "split-right", label: "向右拆分", defaultBinding: "Alt+Backslash", requiresMultiplePanes: false, action: { kind: "split", direction: "vertical", placement: "second" } },
  { id: "close-pane", label: "关闭窗格", defaultBinding: "Alt+KeyX", requiresMultiplePanes: true, action: { kind: "close" } },
  { id: "zoom-pane", label: "切换窗格缩放", defaultBinding: "Alt+KeyZ", requiresMultiplePanes: true, action: { kind: "zoom" } },
  { id: "manage-one-keys", label: "打开 OneKeys", defaultBinding: `${primaryChordModifier}+KeyT ${primaryChordModifier}+KeyK`, requiresMultiplePanes: false, action: { kind: "one-keys" } },
];

export const defaultWorkspaceKeymap = Object.fromEntries(
  workspaceHotkeyCommands.map((command) => [command.id, command.defaultBinding]),
) as WorkspaceKeymap;

const modifierOrder = ["Ctrl", "Alt", "Shift", "Meta"] as const;
const supportedCodes = /^(Arrow(Up|Down|Left|Right)|Backquote|Backslash|Backspace|Bracket(Left|Right)|Comma|Delete|Digit[0-9]|End|Enter|Equal|Escape|F([1-9]|1[0-2])|Home|Insert|Key[A-Z]|Minus|Page(Up|Down)|Period|Quote|Semicolon|Slash|Space|Tab)$/;

export function normalizeWorkspaceKeymap(value: unknown): WorkspaceKeymap {
  const source = value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
  return Object.fromEntries(workspaceHotkeyCommands.map((command) => {
    if (!(command.id in source)) return [command.id, command.defaultBinding];
    if (source[command.id] === "") return [command.id, ""];
    return [command.id, normalizeWorkspaceKeyBinding(source[command.id]) ?? command.defaultBinding];
  })) as WorkspaceKeymap;
}

export function workspaceKeymapConflicts(keymap: WorkspaceKeymap): WorkspaceKeymapConflict[] {
  const commandIdsByBinding = new Map<string, WorkspaceHotkeyCommandId[]>();
  for (const command of workspaceHotkeyCommands) {
    const binding = keymap[command.id];
    if (!binding) continue;
    const commandIds = commandIdsByBinding.get(binding) ?? [];
    commandIds.push(command.id);
    commandIdsByBinding.set(binding, commandIds);
  }
  const conflicts = [...commandIdsByBinding.entries()]
    .filter(([, commandIds]) => commandIds.length > 1)
    .map(([binding, commandIds]): WorkspaceKeymapConflict => ({ binding, commandIds, kind: "duplicate" }));
  for (let index = 0; index < workspaceHotkeyCommands.length; index += 1) {
    const first = workspaceHotkeyCommands[index];
    const firstBinding = keymap[first.id];
    if (!firstBinding) continue;
    for (let otherIndex = index + 1; otherIndex < workspaceHotkeyCommands.length; otherIndex += 1) {
      const second = workspaceHotkeyCommands[otherIndex];
      const secondBinding = keymap[second.id];
      if (!secondBinding || firstBinding === secondBinding) continue;
      const prefix = firstBinding.length < secondBinding.length ? firstBinding : secondBinding;
      const longer = prefix === firstBinding ? secondBinding : firstBinding;
      if (longer.startsWith(`${prefix} `)) {
        conflicts.push({ binding: prefix, commandIds: [first.id, second.id], kind: "prefix" });
      }
    }
  }
  return conflicts;
}

export function workspaceKeyBindingFromEvent(input: WorkspaceHotkeyInput): string | null {
  if (!supportedCodes.test(input.code)) return null;
  const modifiers = [
    input.ctrlKey ? "Ctrl" : "",
    input.altKey ? "Alt" : "",
    input.shiftKey ? "Shift" : "",
    input.metaKey ? "Meta" : "",
  ].filter(Boolean);
  return modifiers.length ? [...modifiers, input.code].join("+") : null;
}

export function formatWorkspaceKeyBinding(binding: string): string {
  if (!binding) return "未绑定";
  const labels: Record<string, string> = {
    ArrowUp: "↑",
    ArrowDown: "↓",
    ArrowLeft: "←",
    ArrowRight: "→",
    Backslash: "\\",
    Minus: "-",
    Equal: "=",
    Space: "Space",
  };
  return binding.split(" ").map((stroke) => (
    stroke.split("+").map((part) => labels[part] ?? part.replace(/^Key/, "").replace(/^Digit/, "")).join(" + ")
  )).join("  →  ");
}

export function resolveWorkspaceHotkey(
  input: WorkspaceHotkeyInput,
  paneCount: number,
  keymap: WorkspaceKeymap = defaultWorkspaceKeymap,
): WorkspaceHotkeyAction | null {
  const resolution = resolveWorkspaceHotkeySequence(input, paneCount, keymap);
  return resolution.kind === "action" ? resolution.action : null;
}

export function resolveWorkspaceHotkeySequence(
  input: WorkspaceHotkeyInput,
  paneCount: number,
  keymap: WorkspaceKeymap = defaultWorkspaceKeymap,
  prefix = "",
): WorkspaceHotkeyResolution {
  const stroke = workspaceKeyBindingFromEvent(input);
  if (!stroke) return { kind: "none" };
  const binding = prefix ? `${prefix} ${stroke}` : stroke;
  const eligibleCommands = workspaceHotkeyCommands.filter((command) => !command.requiresMultiplePanes || paneCount > 1);
  const exactMatches = eligibleCommands.filter((command) => keymap[command.id] === binding);
  const longerMatches = eligibleCommands.filter((command) => keymap[command.id].startsWith(`${binding} `));
  if (exactMatches.length === 1 && !longerMatches.length) {
    return { kind: "action", action: exactMatches[0].action };
  }
  if (!exactMatches.length && longerMatches.length) return { kind: "pending", prefix: binding };
  return { kind: "none" };
}

function normalizeWorkspaceKeyBinding(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const strokes = value.trim().split(/\s+/);
  if (!strokes.length || strokes.length > 2) return null;
  const normalized = strokes.map(normalizeWorkspaceKeyStroke);
  return normalized.every((stroke): stroke is string => Boolean(stroke)) ? normalized.join(" ") : null;
}

function normalizeWorkspaceKeyStroke(value: string): string | null {
  const parts = value.split("+").map((part) => part.trim()).filter(Boolean);
  const code = parts.at(-1) ?? "";
  if (!supportedCodes.test(code)) return null;
  const modifiers = new Set(parts.slice(0, -1));
  if (!modifiers.size || modifiers.size !== parts.length - 1) return null;
  if ([...modifiers].some((modifier) => !modifierOrder.includes(modifier as (typeof modifierOrder)[number]))) return null;
  return [...modifierOrder.filter((modifier) => modifiers.has(modifier)), code].join("+");
}
