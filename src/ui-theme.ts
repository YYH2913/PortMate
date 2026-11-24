import { useSyncExternalStore } from "react";

export const UI_APPEARANCE_STORAGE_KEY = "portmate.uiAppearance.v1";

export const uiThemePresets = ["studio-light", "carbon-islands", "obsidian-linear"] as const;
export type UiThemePreset = typeof uiThemePresets[number];

export const uiLayoutStyles = ["docked", "islands"] as const;
export type UiLayoutStyle = typeof uiLayoutStyles[number];

export const uiAccentColors = ["default", "indigo", "emerald", "amber", "rose"] as const;
export type UiAccentColor = typeof uiAccentColors[number];

export type UiAppearanceState = Readonly<{
  theme: UiThemePreset;
  layout: UiLayoutStyle;
  accent: UiAccentColor;
}>;

export const DEFAULT_UI_APPEARANCE: UiAppearanceState = Object.freeze({
  theme: "studio-light",
  layout: "docked",
  accent: "default",
});

const listeners = new Set<() => void>();
let state: UiAppearanceState = DEFAULT_UI_APPEARANCE;
let previewState: UiAppearanceState | null = null;
let initialized = false;

export function normalizeUiThemePreset(value: unknown): UiThemePreset {
  return typeof value === "string" && (uiThemePresets as readonly string[]).includes(value)
    ? value as UiThemePreset
    : DEFAULT_UI_APPEARANCE.theme;
}

export function normalizeUiLayoutStyle(value: unknown): UiLayoutStyle {
  return typeof value === "string" && (uiLayoutStyles as readonly string[]).includes(value)
    ? value as UiLayoutStyle
    : DEFAULT_UI_APPEARANCE.layout;
}

export function normalizeUiAccentColor(value: unknown): UiAccentColor {
  return typeof value === "string" && (uiAccentColors as readonly string[]).includes(value)
    ? value as UiAccentColor
    : DEFAULT_UI_APPEARANCE.accent;
}

export function normalizeUiAppearance(value: unknown): UiAppearanceState {
  if (!value || typeof value !== "object") return DEFAULT_UI_APPEARANCE;
  const candidate = value as Record<string, unknown>;
  return {
    theme: normalizeUiThemePreset(candidate.theme),
    layout: normalizeUiLayoutStyle(candidate.layout),
    accent: normalizeUiAccentColor(candidate.accent),
  };
}

export function uiAppearanceEquals(left: UiAppearanceState, right: UiAppearanceState): boolean {
  return left.theme === right.theme && left.layout === right.layout && left.accent === right.accent;
}

function applyToDocument(next: UiAppearanceState) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.dataset.uiTheme = next.theme;
  root.dataset.uiLayout = next.layout;
  root.dataset.uiAccent = next.accent;
}

function publish(next: UiAppearanceState) {
  const normalized = normalizeUiAppearance(next);
  applyToDocument(previewState ?? normalized);
  if (uiAppearanceEquals(state, normalized)) return;
  state = normalized;
  for (const listener of listeners) listener();
}

export function getUiAppearance(): UiAppearanceState {
  return state;
}

export function subscribeUiAppearance(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function useUiAppearance(): UiAppearanceState {
  return useSyncExternalStore(subscribeUiAppearance, getUiAppearance, getUiAppearance);
}

/** Temporarily preview theme/layout/accent on documentElement without persisting. */
export function setUiAppearancePreview(next: UiAppearanceState | null) {
  previewState = next ? normalizeUiAppearance(next) : null;
  applyToDocument(previewState ?? state);
}

export function setUiAppearance(patch: Partial<UiAppearanceState> | UiAppearanceState) {
  const next = normalizeUiAppearance({ ...state, ...patch });
  previewState = null;
  try {
    window.localStorage.setItem(UI_APPEARANCE_STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Keep in-memory appearance usable if storage is unavailable.
  }
  publish(next);
}

function readStoredAppearance(): UiAppearanceState {
  try {
    const raw = window.localStorage.getItem(UI_APPEARANCE_STORAGE_KEY);
    if (!raw) return DEFAULT_UI_APPEARANCE;
    return normalizeUiAppearance(JSON.parse(raw));
  } catch {
    return DEFAULT_UI_APPEARANCE;
  }
}

/** Called synchronously before initial React mount so there is no theme flash. */
export function initializeUiTheme(): UiAppearanceState {
  const loaded = typeof window === "undefined" ? DEFAULT_UI_APPEARANCE : readStoredAppearance();
  publish(loaded);
  if (initialized || typeof window === "undefined") return state;
  initialized = true;
  window.addEventListener("storage", (event) => {
    if (event.key === UI_APPEARANCE_STORAGE_KEY || event.key === null) {
      publish(readStoredAppearance());
    }
  });
  return state;
}
