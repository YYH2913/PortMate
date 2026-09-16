import { afterEach, describe, expect, it, vi } from "vitest";

const backend = vi.hoisted(() => ({ invoke: vi.fn(), available: vi.fn(() => true) }));
vi.mock("./api", () => ({ invokeBackend: backend.invoke, isBackendAvailable: backend.available }));

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); vi.clearAllMocks(); });

async function setup() {
  vi.resetModules();
  const window = new EventTarget();
  const storage = new Map<string, string>();
  Object.assign(window, { localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) } });
  vi.stubGlobal("window", window);
  vi.stubGlobal("navigator", { language: "en-US", languages: ["en-US"] });
  vi.stubGlobal("document", { documentElement: { lang: "", dir: "" } });
  return { window, storage, i18n: await import("./i18n") };
}

describe("native system language startup", () => {
  it("prefers native system language and handles cross-window preference changes", async () => {
    backend.invoke.mockResolvedValue("ar-SA");
    const { i18n, window, storage } = await setup();
    await i18n.initializeI18n();
    expect(document.documentElement).toEqual({ lang: "ar", dir: "rtl" });
    storage.set("portmate.language.v1", "fr");
    window.dispatchEvent(Object.assign(new Event("storage"), { key: "portmate.language.v1" }));
    expect(i18n.getLanguageState()).toEqual({ preference: "fr", locale: "fr" });
    storage.clear();
    window.dispatchEvent(Object.assign(new Event("storage"), { key: null }));
    expect(i18n.getLanguageState().locale).toBe("ar");
  });

  it("ignores a late startup result after a newer system language notification", async () => {
    let finish!: (value: string) => void;
    backend.invoke.mockImplementationOnce(() => new Promise<string>(resolve => { finish = resolve; }));
    const { i18n } = await setup();
    const initializing = i18n.initializeI18n();
    i18n.updateSystemLanguage("fr-FR");
    finish("ar-SA");
    await initializing;
    expect(i18n.getLanguageState().locale).toBe("fr");
  });

  it("does not leave startup blocked when the native bridge never responds", async () => {
    vi.useFakeTimers();
    backend.invoke.mockImplementationOnce(() => new Promise(() => {}));
    const { i18n } = await setup();
    const initializing = i18n.initializeI18n();
    await vi.advanceTimersByTimeAsync(1000);
    await initializing;
    expect(i18n.getLanguageState().locale).toBe("en");
  });
});
