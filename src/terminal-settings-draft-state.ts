export type TerminalSettingsDraftState = {
  prefs: unknown;
  syncSettings: unknown;
  workspaceKeymap: unknown;
};

export function terminalSettingsDraftHasUnsavedChanges(
  draft: TerminalSettingsDraftState,
  saved: TerminalSettingsDraftState,
): boolean {
  return !sameStructuredValue(draft.prefs, saved.prefs)
    || !sameStructuredValue(draft.syncSettings, saved.syncSettings)
    || !sameStructuredValue(draft.workspaceKeymap, saved.workspaceKeymap);
}

function sameStructuredValue(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left)
      && Array.isArray(right)
      && left.length === right.length
      && left.every((value, index) => sameStructuredValue(value, right[index]));
  }
  if (!isRecord(left) || !isRecord(right)) return false;
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  return leftKeys.length === rightKeys.length
    && leftKeys.every((key) => Object.hasOwn(right, key) && sameStructuredValue(left[key], right[key]));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object";
}
