export const TERMINAL_SEARCH_REQUEST_EVENT = "portmate-terminal-search";
export const MAX_TERMINAL_SEARCH_QUERY_LENGTH = 512;

export type TerminalSearchResult = {
  resultIndex: number;
  resultCount: number;
};

type TerminalFindKey = Pick<KeyboardEvent, "altKey" | "ctrlKey" | "key" | "metaKey">;

export function isTerminalFindShortcut(event: TerminalFindKey): boolean {
  return !event.altKey && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f";
}

export function terminalSearchSeed(selection: string): string {
  let seed = "";
  for (const character of selection.replace(/[\r\n]+/g, " ").trim()) {
    if (seed.length + character.length > MAX_TERMINAL_SEARCH_QUERY_LENGTH) break;
    seed += character;
  }
  return seed;
}

export function terminalSearchResultLabel(
  query: string,
  result: TerminalSearchResult | null,
  invalidExpression = false,
): string {
  if (invalidExpression) return "表达式无效";
  if (!query || !result || result.resultCount <= 0) return "0/0";
  if (result.resultIndex < 0) return `${result.resultCount} 个结果`;
  return `${Math.min(result.resultIndex + 1, result.resultCount)}/${result.resultCount}`;
}

export function requestTerminalSearch(target: Pick<EventTarget, "dispatchEvent"> = window): boolean {
  return target.dispatchEvent(new Event(TERMINAL_SEARCH_REQUEST_EVENT));
}
