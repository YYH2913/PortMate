import { normalizeTerminalKeyMode } from "./terminal-key-mode";
import type { TerminalKeyMode } from "./terminal-key-mode";
import type { SessionSummary } from "./types";

export const DETACHED_PANE_EVENT = "portmate-detached-pane-command";
export const DETACHED_PANE_MESSAGE_TYPE = "portmate:detached-pane-command";
export const SESSION_PROFILE_DELETED_EVENT = "portmate-session-profile-deleted";
export const SESSION_PROFILE_UPDATED_EVENT = "portmate-session-profile-updated";

export type DetachedPaneRequest = {
  windowId: string;
  ownerWindowId: string;
  paneId: string;
  viewId: string;
  sessionId: string;
  title: string;
  color: string;
  keyMode: TerminalKeyMode;
};

export type DetachedPaneCommand = DetachedPaneRequest & {
  action: "connect" | "disconnect" | "reattach" | "lock-screen";
};

export type DetachedPaneMessage = {
  type: typeof DETACHED_PANE_MESSAGE_TYPE;
  payload: DetachedPaneCommand;
};

const windowIdPattern = /^[A-Za-z0-9_-]{1,128}$/;
const ownerWindowIdPattern = /^(?:main|workspace-[A-Za-z0-9_-]{1,118})$/;

export function buildDetachedPanePath(request: DetachedPaneRequest): string {
  const params = new URLSearchParams({
    detachedPane: "1",
    windowId: request.windowId,
    ownerWindowId: request.ownerWindowId,
    paneId: request.paneId,
    viewId: request.viewId,
    sessionId: request.sessionId,
    title: request.title,
    color: request.color,
    keyMode: request.keyMode,
  });
  return `/?${params.toString()}`;
}

export function parseDetachedPaneRequest(search: string): DetachedPaneRequest | null {
  const params = new URLSearchParams(search);
  if (params.get("detachedPane") !== "1") return null;
  const windowId = params.get("windowId") ?? "";
  const ownerWindowId = params.get("ownerWindowId") ?? "main";
  const paneId = cleanRouteId(params.get("paneId"));
  const viewId = cleanRouteId(params.get("viewId"));
  const sessionId = cleanRouteId(params.get("sessionId"));
  const title = cleanRouteTitle(params.get("title"));
  const color = cleanRouteColor(params.get("color"));
  const keyMode = normalizeTerminalKeyMode(params.get("keyMode"));
  if (!windowIdPattern.test(windowId) || !ownerWindowIdPattern.test(ownerWindowId) || !paneId || !viewId || !sessionId || title === null || color === null) return null;
  return { windowId, ownerWindowId, paneId, viewId, sessionId, title, color, keyMode };
}

export function normalizeDetachedPaneCommand(value: unknown): DetachedPaneCommand | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  if (source.action !== "connect" && source.action !== "disconnect" && source.action !== "reattach" && source.action !== "lock-screen") return null;
  const request = parseDetachedPaneRequest(`?${new URLSearchParams({
    detachedPane: "1",
    windowId: typeof source.windowId === "string" ? source.windowId : "",
    ownerWindowId: typeof source.ownerWindowId === "string" ? source.ownerWindowId : "main",
    paneId: typeof source.paneId === "string" ? source.paneId : "",
    viewId: typeof source.viewId === "string" ? source.viewId : "",
    sessionId: typeof source.sessionId === "string" ? source.sessionId : "",
    title: typeof source.title === "string" ? source.title : "",
    color: typeof source.color === "string" ? source.color : "",
    keyMode: typeof source.keyMode === "string" ? source.keyMode : "remote",
  })}`);
  return request ? { ...request, action: source.action } : null;
}

export function normalizeDetachedPaneMessage(value: unknown): DetachedPaneMessage | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  if (source.type !== DETACHED_PANE_MESSAGE_TYPE) return null;
  const payload = normalizeDetachedPaneCommand(source.payload);
  return payload ? { type: DETACHED_PANE_MESSAGE_TYPE, payload } : null;
}

export function upsertDetachedSessionSummary(
  sessions: readonly SessionSummary[],
  updated: SessionSummary,
): SessionSummary[] {
  const index = sessions.findIndex((session) => session.profile.id === updated.profile.id);
  if (index < 0) return [...sessions, updated];
  const next = [...sessions];
  next[index] = updated;
  return next;
}

function cleanRouteId(value: string | null): string {
  const raw = value ?? "";
  if (/[\u0000-\u001f\u007f]/.test(raw)) return "";
  const clean = raw.trim();
  return clean && clean.length <= 128 ? clean : "";
}

function cleanRouteTitle(value: string | null): string | null {
  const raw = value ?? "";
  if (/[\u0000-\u001f\u007f]/.test(raw)) return null;
  const clean = raw.trim();
  return [...clean].length <= 128 ? clean : null;
}

function cleanRouteColor(value: string | null): string | null {
  if (!value) return "";
  return /^#[0-9a-f]{6}$/i.test(value) ? value.toUpperCase() : null;
}
