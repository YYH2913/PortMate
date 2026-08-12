export interface SerialAnalyzerRequest {
  windowId: string;
  ownerWindowId: string;
  sessionId: string;
}

export function buildSerialAnalyzerPath(request: SerialAnalyzerRequest): string {
  const params = new URLSearchParams({
    serialAnalyzer: "1",
    windowId: request.windowId,
    ownerWindowId: request.ownerWindowId,
    sessionId: request.sessionId,
  });
  return `/?${params.toString()}`;
}

export function parseSerialAnalyzerRequest(search: string): SerialAnalyzerRequest | null {
  const params = new URLSearchParams(search);
  if (params.get("serialAnalyzer") !== "1") return null;
  const windowId = params.get("windowId") ?? "";
  const ownerWindowId = params.get("ownerWindowId") ?? "main";
  const sessionId = cleanIdentifier(params.get("sessionId"), 256);
  return /^[A-Za-z0-9_-]{1,128}$/.test(windowId)
    && /^[A-Za-z0-9_-]{1,128}$/.test(ownerWindowId)
    && sessionId
    ? { windowId, ownerWindowId, sessionId }
    : null;
}

function cleanIdentifier(value: unknown, maximum: number): string {
  if (typeof value !== "string" || /[\u0000-\u001f\u007f]/.test(value)) return "";
  const clean = value.trim();
  return clean && clean.length <= maximum ? clean : "";
}
