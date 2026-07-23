export const MAX_FILE_NAVIGATION_HISTORY = 64;

export type FileNavigationHistory = {
  paths: string[];
  index: number;
};

export type FileNavigationTarget = {
  path: string;
  index: number;
};

export function createFileNavigationHistory(path: string): FileNavigationHistory {
  return { paths: [path], index: 0 };
}

export function currentFileNavigationPath(history: FileNavigationHistory): string | null {
  return history.paths[history.index] ?? null;
}

export function recordFileNavigation(history: FileNavigationHistory, path: string): FileNavigationHistory {
  if (!history.paths.length) return createFileNavigationHistory(path);
  const index = Math.max(0, Math.min(history.index, history.paths.length - 1));
  if (history.paths[index] === path) {
    return history.index === index ? history : { ...history, index };
  }
  const paths = [...history.paths.slice(0, index + 1), path];
  const start = Math.max(0, paths.length - MAX_FILE_NAVIGATION_HISTORY);
  const retained = paths.slice(start);
  return { paths: retained, index: retained.length - 1 };
}

export function fileNavigationTarget(history: FileNavigationHistory, offset: number): FileNavigationTarget | null {
  if (!Number.isInteger(offset)) return null;
  const index = history.index + offset;
  const path = history.paths[index];
  return path === undefined ? null : { path, index };
}

export function restoreFileNavigation(history: FileNavigationHistory, index: number): FileNavigationHistory {
  if (!Number.isInteger(index) || index < 0 || index >= history.paths.length || index === history.index) {
    return history;
  }
  return { ...history, index };
}
