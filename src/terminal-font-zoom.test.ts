import { afterEach, describe, expect, it, vi } from "vitest";
import { boundedTerminalFontSize, nextTerminalFontSize, parseTerminalFontZoom, readTerminalFontZoom, subscribeTerminalFontZoom, terminalFontZoomShortcut, writeTerminalFontZoom } from "./terminal-font-zoom";

afterEach(() => vi.unstubAllGlobals());

describe("terminal font zoom", () => {
  it("bounds font size and resets to the current profile default", () => {
    expect(nextTerminalFontSize(13, 13, "increase")).toBe(14);
    expect(nextTerminalFontSize(72, 13, "increase")).toBe(72);
    expect(nextTerminalFontSize(6, 13, "decrease")).toBe(6);
    expect(nextTerminalFontSize(20, 16, "reset")).toBe(16);
    expect(boundedTerminalFontSize(NaN)).toBe(13);
    expect(boundedTerminalFontSize(Infinity)).toBe(13);
  });

  it("recognizes Ctrl/Cmd zoom and preserves remote Ctrl+underscore, AltGr and IME input", () => {
    const event = { key: "=", code: "Equal", ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, isComposing: false };
    expect(terminalFontZoomShortcut(event)).toBe("increase");
    expect(terminalFontZoomShortcut({ ...event, key: "+", shiftKey: true })).toBe("increase");
    expect(terminalFontZoomShortcut({ ...event, key: "-", code: "Minus" })).toBe("decrease");
    expect(terminalFontZoomShortcut({ ...event, key: "0", code: "Digit0", ctrlKey: false, metaKey: true })).toBe("reset");
    expect(terminalFontZoomShortcut({ ...event, key: "+", code: "NumpadAdd" })).toBe("increase");
    for (const patch of [{ ctrlKey: false }, { altKey: true }, { metaKey: true }, { isComposing: true }, { key: "_", code: "Minus", shiftKey: true }, { key: "c" }]) {
      expect(terminalFontZoomShortcut({ ...event, ...patch })).toBeNull();
    }
  });

  it("ignores invalid saved data and lets profile font edits override old zoom", () => {
    for (const value of [null, "bad-json", "[]", "null", '{"base":13,"size":"20"}', '{"base":12,"size":20}']) {
      expect(parseTerminalFontZoom(value, 13)).toBe(13);
    }
    expect(parseTerminalFontZoom('{"base":13,"size":20}', 13)).toBe(20);
    expect(parseTerminalFontZoom('{"base":13,"size":200}', 13)).toBe(72);
  });

  it("persists per-view zoom, notifies peers and works when storage is denied", () => {
    const values = new Map<string, string>();
    const window = Object.assign(new EventTarget(), { localStorage: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => { values.set(key, value); },
      removeItem: (key: string) => { values.delete(key); },
    } });
    vi.stubGlobal("window", window);
    const notify = vi.fn();
    const unsubscribe = subscribeTerminalFontZoom("view-a", notify);
    writeTerminalFontZoom("view-a", 13, 17);
    expect(readTerminalFontZoom("view-a", 13)).toBe(17);
    expect(readTerminalFontZoom("view-b", 13)).toBe(13);
    expect(notify).toHaveBeenCalledOnce();
    writeTerminalFontZoom("view-a", 13, 13);
    expect(readTerminalFontZoom("view-a", 13)).toBe(13);
    expect(values.size).toBe(0);
    unsubscribe();
    Object.defineProperty(window, "localStorage", { get: () => { throw new Error("denied"); } });
    writeTerminalFontZoom("denied-view", 13, 19);
    expect(readTerminalFontZoom("denied-view", 13)).toBe(19);
    writeTerminalFontZoom("denied-view", 13, 13);
    expect(readTerminalFontZoom("denied-view", 13)).toBe(13);
  });
});
