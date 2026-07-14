import { describe, expect, it } from "vitest";
import { normalizeProxyConfig, proxyDefaults } from "./proxy-settings";

describe("proxy settings", () => {
  it("fills safe defaults for legacy profiles", () => {
    expect(normalizeProxyConfig()).toEqual(proxyDefaults);
  });

  it("normalizes protocol, host, and port", () => {
    expect(normalizeProxyConfig({
      enabled: true,
      kind: "http-connect",
      host: "  proxy.example  ",
      port: 3128.9,
      username: "  proxy-user  ",
      passwordSecretRef: "  keychain:proxy-password  ",
    })).toEqual({
      enabled: true,
      kind: "http-connect",
      host: "proxy.example",
      port: 3128,
      username: "proxy-user",
      passwordSecretRef: "keychain:proxy-password",
    });
    expect(normalizeProxyConfig({ kind: "socks5", port: 99_999 })).toMatchObject({
      kind: "socks5",
      port: 65_535,
    });
  });

  it("keeps invalid enabled endpoints visible for backend validation", () => {
    expect(normalizeProxyConfig({ enabled: true, host: "   ", port: 0 })).toEqual({
      enabled: true,
      kind: "socks5",
      host: "",
      port: 0,
      username: "",
      passwordSecretRef: null,
    });
  });

  it("drops empty legacy authentication metadata", () => {
    expect(normalizeProxyConfig({
      username: "   ",
      passwordSecretRef: "   ",
    })).toEqual(proxyDefaults);
  });
});
