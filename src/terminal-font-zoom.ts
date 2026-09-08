import { TERMINAL_PROFILE_BOUNDS } from "./terminal-settings-state";

export type TerminalFontZoomAction = "increase" | "decrease" | "reset";
export const TERMINAL_FONT_ZOOM_PREFIX = "portmate.terminalFontZoom.v1:";
const CHANGE_EVENT = "portmate-terminal-font-zoom";
const transient = new Map<string, string | null>();

export function boundedTerminalFontSize(value: number): number {
  const { min, max, fallback } = TERMINAL_PROFILE_BOUNDS.fontSize;
  return Math.max(min, Math.min(max, Number.isFinite(value) ? Math.round(value) : fallback));
}

export function nextTerminalFontSize(current: number, base: number, action: TerminalFontZoomAction): number {
  return boundedTerminalFontSize(action === "reset" ? base : current + (action === "increase" ? 1 : -1));
}

export function terminalFontZoomShortcut(event: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "isComposing">): TerminalFontZoomAction | null {
  if (event.isComposing || event.altKey || event.ctrlKey === event.metaKey) return null;
  if (event.key === "+" || event.key === "=" || event.code === "NumpadAdd") return "increase";
  if (!event.shiftKey && (event.key === "-" || event.code === "NumpadSubtract")) return "decrease";
  if (!event.shiftKey && (event.key === "0" || event.code === "Numpad0")) return "reset";
  return null;
}

export function parseTerminalFontZoom(raw: string | null, base: number): number {
  const normalizedBase = boundedTerminalFontSize(base);
  try {
    const saved: unknown = JSON.parse(raw ?? "null");
    if (!saved || typeof saved !== "object" || Array.isArray(saved)) return normalizedBase;
    const value = saved as Record<string, unknown>;
    // Profile font changes take precedence over an older view override.
    if (value.base !== normalizedBase || typeof value.size !== "number" || !Number.isFinite(value.size)) return normalizedBase;
    return boundedTerminalFontSize(value.size);
  } catch { return normalizedBase; }
}

export function readTerminalFontZoom(viewKey: string, base: number): number {
  const key = TERMINAL_FONT_ZOOM_PREFIX + viewKey;
  if (transient.has(key)) return parseTerminalFontZoom(transient.get(key) ?? null, base);
  try { return parseTerminalFontZoom(window.localStorage.getItem(key), base); }
  catch { return boundedTerminalFontSize(base); }
}

export function writeTerminalFontZoom(viewKey: string, base: number, size: number): void {
  const key = TERMINAL_FONT_ZOOM_PREFIX + viewKey;
  const value = size === base ? null : JSON.stringify({ base: boundedTerminalFontSize(base), size: boundedTerminalFontSize(size) });
  try {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
    transient.delete(key);
  } catch {
    // Storage denial must not disable zoom for this window.
    transient.delete(key);
    transient.set(key, value);
    if (transient.size > 128) transient.delete(transient.keys().next().value!);
  }
  window.dispatchEvent(new CustomEvent(CHANGE_EVENT, { detail: key }));
}

export function subscribeTerminalFontZoom(viewKey: string, notify: () => void): () => void {
  const key = TERMINAL_FONT_ZOOM_PREFIX + viewKey;
  const local = (event: Event) => { if ((event as CustomEvent).detail === key) notify(); };
  const remote = (event: StorageEvent) => {
    if (event.key === key || event.key === null) { transient.delete(key); notify(); }
  };
  window.addEventListener(CHANGE_EVENT, local);
  window.addEventListener("storage", remote);
  return () => { window.removeEventListener(CHANGE_EVENT, local); window.removeEventListener("storage", remote); };
}
