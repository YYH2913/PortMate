import { createElement, Fragment, useSyncExternalStore } from "react";
import type { ReactNode } from "react";
import { invokeBackend, isBackendAvailable } from "./api";
import en from "./locales/en.json";
import zh from "./locales/zh.json";
import ar from "./locales/ar.json";
import fr from "./locales/fr.json";
import ru from "./locales/ru.json";
import es from "./locales/es.json";
import nativeMessages from "./locales/native-message-templates.json";
import { createDiagnosticLocalizer } from "./diagnostic-localization";

export const LANGUAGE_STORAGE_KEY = "portmate.language.v1";
export const languages = [
  { id: "ar", name: "العربية", direction: "rtl" },
  { id: "zh", name: "简体中文", direction: "ltr" },
  { id: "en", name: "English", direction: "ltr" },
  { id: "fr", name: "Français", direction: "ltr" },
  { id: "ru", name: "Русский", direction: "ltr" },
  { id: "es", name: "Español", direction: "ltr" },
] as const;
export type Locale = typeof languages[number]["id"];
export type LanguagePreference = Locale | "system";
export type LanguageState = Readonly<{ preference: LanguagePreference; locale: Locale }>;
const catalogs: Record<Locale, Readonly<Record<string, string>>> = { en, zh, ar, fr, ru, es };
const listeners = new Set<() => void>();
let systemLanguage = typeof navigator === "undefined" ? "en" : navigator.languages?.[0] || navigator.language;
let state: LanguageState = { preference: "system", locale: resolveLocale(systemLanguage) };
let initialized = false;
let systemLanguageRevision = 0;

/** Resolve the primary system language; an unsupported language always uses English. */
export function resolveLocale(value: unknown): Locale {
  if (typeof value !== "string") return "en";
  const tag = value.trim().replace(/_/g, "-").split(/[.@]/, 1)[0];
  try {
    const base = new Intl.Locale(tag).language;
    return languages.some(language => language.id === base) ? base as Locale : "en";
  } catch { return "en"; }
}

export function normalizeLanguagePreference(value: unknown): LanguagePreference {
  return value === "system" || languages.some(language => language.id === value)
    ? value as LanguagePreference : "system";
}

export function getLanguageState(): LanguageState { return state; }
export function subscribeLanguage(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function useLocale(): LanguageState {
  return useSyncExternalStore(subscribeLanguage, getLanguageState, getLanguageState);
}

function publish(preference: LanguagePreference) {
  const locale = preference === "system" ? resolveLocale(systemLanguage) : preference;
  if (typeof document !== "undefined") {
    document.documentElement.lang = locale === "zh" ? "zh-CN" : locale;
    document.documentElement.dir = locale === "ar" ? "rtl" : "ltr";
  }
  if (state.preference === preference && state.locale === locale) return;
  state = { preference, locale };
  for (const listener of listeners) listener();
}

export function setLanguagePreference(value: LanguagePreference) {
  const preference = normalizeLanguagePreference(value);
  try { window.localStorage.setItem(LANGUAGE_STORAGE_KEY, preference); } catch { /* In-memory selection remains usable. */ }
  publish(preference);
}

export function updateSystemLanguage(value: unknown) {
  if (typeof value !== "string" || !value.trim()) return;
  systemLanguageRevision += 1;
  systemLanguage = value;
  publish(state.preference);
}

async function refreshNativeSystemLanguage(): Promise<void> {
  const revision = systemLanguageRevision;
  try {
    const locale = await invokeBackend<string | null>("system_locale", {});
    // An older request must not overwrite a more recent OS/browser notification.
    if (revision === systemLanguageRevision) updateSystemLanguage(locale);
  } catch { /* Keep the browser/system fallback when the native bridge is unavailable. */ }
}

function readPreference(): LanguagePreference {
  try { return normalizeLanguagePreference(window.localStorage.getItem(LANGUAGE_STORAGE_KEY)); }
  catch { return "system"; }
}

/** Called before rendering: no terminal remounts or page reloads on language changes. */
export async function initializeI18n(): Promise<void> {
  if (initialized || typeof window === "undefined") return;
  initialized = true;
  publish(readPreference());
  window.addEventListener("storage", event => {
    if (event.key === LANGUAGE_STORAGE_KEY || event.key === null) publish(readPreference());
  });
  window.addEventListener("languagechange", () => {
    updateSystemLanguage(navigator.languages?.[0] || navigator.language);
    if (isBackendAvailable()) void refreshNativeSystemLanguage();
  });
  if (!isBackendAvailable()) return;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    // Do not leave the application blank if the native bridge is unavailable.
    await Promise.race([
      refreshNativeSystemLanguage(),
      new Promise<void>(resolve => { timer = setTimeout(resolve, 1000); }),
    ]);
  } finally { if (timer !== undefined) clearTimeout(timer); }
}

/** Only application-owned messages are translated; interpolation values stay verbatim. */
export function translate(message: string, values: readonly unknown[] = [], locale: Locale = state.locale): string {
  const english = Object.hasOwn(catalogs.en, message) ? catalogs.en[message] : message;
  const translated = Object.hasOwn(catalogs[locale], message) ? catalogs[locale][message] : english;
  return translated.replace(/\{(\d+)\}/g, (placeholder, index: string) => (
    Number(index) < values.length ? String(values[Number(index)] ?? "") : placeholder
  ));
}
export const t = translate;

let diagnosticLocalizer: ReturnType<typeof createDiagnosticLocalizer> | undefined;
/** Only call for diagnostic presentation, never for transport payloads or control flow. */
export function localizeDiagnostic(message: string | null | undefined): string {
  if (!message) return "";
  if (state.locale === "zh" || !/\p{Script=Han}/u.test(message)) return message;
  diagnosticLocalizer ??= createDiagnosticLocalizer(nativeMessages, translate);
  if (message.startsWith("Error: ")) return "Error: " + diagnosticLocalizer(message.slice(7));
  return diagnosticLocalizer(message);
}

/** Rich interpolation preserves React nodes instead of coercing icons/markup to strings. */
export function tr(message: string, values: readonly ReactNode[]): ReactNode {
  return translate(message).split(/(\{\d+\})/g).map((part, index) => {
    const match = /^\{(\d+)\}$/.exec(part);
    const value = match && Number(match[1]) < values.length ? values[Number(match[1])] : part;
    return createElement(Fragment, { key: index }, value);
  });
}

export function formatUiNumber(value: number, options?: Intl.NumberFormatOptions): string {
  return new Intl.NumberFormat(state.locale === "zh" ? "zh-CN" : state.locale, options).format(value);
}

export function formatUiDate(value: Date | number, options?: Intl.DateTimeFormatOptions): string {
  return new Intl.DateTimeFormat(state.locale === "zh" ? "zh-CN" : state.locale, options).format(value);
}
