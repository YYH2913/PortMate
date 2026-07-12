export type SelectionModifiers = {
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
};

export function updateFileSelection<T extends { path: string }>(
  entries: readonly T[],
  selected: readonly T[],
  entry: T,
  anchorPath: string,
  modifiers: SelectionModifiers,
): { selected: T[]; anchorPath: string } {
  if (modifiers.shiftKey && anchorPath) {
    const anchorIndex = entries.findIndex((item) => item.path === anchorPath);
    const entryIndex = entries.findIndex((item) => item.path === entry.path);
    if (anchorIndex >= 0 && entryIndex >= 0) {
      const start = Math.min(anchorIndex, entryIndex);
      const end = Math.max(anchorIndex, entryIndex);
      return { selected: entries.slice(start, end + 1), anchorPath };
    }
  }

  if (modifiers.ctrlKey || modifiers.metaKey) {
    const alreadySelected = selected.some((item) => item.path === entry.path);
    return {
      selected: alreadySelected
        ? selected.filter((item) => item.path !== entry.path)
        : [...selected, entry],
      anchorPath: entry.path,
    };
  }

  return { selected: [entry], anchorPath: entry.path };
}
