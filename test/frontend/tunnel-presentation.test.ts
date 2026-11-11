import { describe, expect, it } from "vitest";
import { formatTunnelEndpoint, tunnelModePresentation } from "../../src/tunnel-presentation";
import en from "../../src/locales/en.json";
import zh from "../../src/locales/zh.json";
import ar from "../../src/locales/ar.json";
import es from "../../src/locales/es.json";
import fr from "../../src/locales/fr.json";
import ru from "../../src/locales/ru.json";

describe("tunnel presentation", () => {
  it("keeps local and remote entry and target sides distinct", () => {
    expect(tunnelModePresentation.local).toMatchObject({
      bindLabel: "tunnel-bind-local",
      bindHint: "tunnel-bind-local-hint",
      entrySide: "tunnel-side-local-entry",
      targetHint: "tunnel-target-ssh-hint",
      targetSide: "tunnel-side-ssh-target",
    });
    expect(tunnelModePresentation.remote).toMatchObject({
      bindLabel: "tunnel-bind-remote",
      bindHint: "tunnel-bind-remote-hint",
      entrySide: "tunnel-side-remote-entry",
      targetHint: "tunnel-target-local-hint",
      targetSide: "tunnel-side-local-target",
    });
    expect(tunnelModePresentation.dynamic).toMatchObject({
      bindLabel: "tunnel-bind-local",
      entrySide: "tunnel-side-proxy-entry",
      targetSide: "tunnel-side-dynamic-target",
    });
  });

  it.each([
    ["127.0.0.1", "22", "127.0.0.1:22"],
    [" service.example ", 443, "service.example:443"],
    ["::1", "1080", "[::1]:1080"],
    ["2001:db8::1", 22, "[2001:db8::1]:22"],
    ["[::1]", 22, "[::1]:22"],
    ["0.0.0.0", "0", "0.0.0.0:自动分配"],
    ["::", 0, "[::]:自动分配"],
    ["", "", "…:…"],
    ["  ", "  ", "…:…"],
  ])("formats the display endpoint %s / %s", (host, port, expected) => {
    expect(formatTunnelEndpoint(host, port, "自动分配")).toBe(expected);
  });

  it("does not describe a zero target or SSH port as automatic", () => {
    expect(formatTunnelEndpoint("127.0.0.1", "0")).toBe("127.0.0.1:0");
    expect(formatTunnelEndpoint("::1", 0)).toBe("[::1]:0");
  });

  it("provides all tunnel hints and presentation keys in every supported language", () => {
    const keys = Object.keys(en).filter(key => key.startsWith("tunnel-"));
    const modeKeys = Object.values(tunnelModePresentation).flatMap(mode => Object.values(mode));
    expect(keys.length).toBeGreaterThanOrEqual(34);
    for (const locale of [en, zh, ar, es, fr, ru]) {
      const messages: Record<string, string> = locale;
      expect(Object.keys(messages).filter(key => key.startsWith("tunnel-")).sort()).toEqual([...keys].sort());
      for (const key of [...keys, ...modeKeys]) {
        expect(messages[key]?.trim(), key).toBeTruthy();
        expect(messages[key], key).not.toBe(key);
      }
      for (const key of ["tunnel-bind-local-hint", "tunnel-bind-remote-hint", "tunnel-target-ssh-hint", "tunnel-target-local-hint"]) {
        expect(messages[key], key).toContain("127.0.0.1");
      }
    }
  });
});
