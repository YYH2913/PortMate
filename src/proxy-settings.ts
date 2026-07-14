import type { ProxyConfig } from "./types";

export type ProxyPasswordUpdate =
  | { action: "set"; password: string }
  | { action: "clear" }
  | null;

export const proxyDefaults: ProxyConfig = {
  enabled: false,
  kind: "socks5",
  host: "127.0.0.1",
  port: 1080,
  username: "",
  passwordSecretRef: null,
};

export function normalizeProxyConfig(proxy?: Partial<ProxyConfig> | null): ProxyConfig {
  const kind = proxy?.kind === "http-connect" ? "http-connect" : "socks5";
  const port = typeof proxy?.port === "number" && Number.isFinite(proxy.port)
    ? Math.min(65_535, Math.max(0, Math.trunc(proxy.port)))
    : proxyDefaults.port;
  return {
    enabled: typeof proxy?.enabled === "boolean" ? proxy.enabled : false,
    kind,
    host: typeof proxy?.host === "string" ? proxy.host.trim() : proxyDefaults.host,
    port,
    username: typeof proxy?.username === "string" ? proxy.username.trim() : proxyDefaults.username,
    passwordSecretRef: typeof proxy?.passwordSecretRef === "string"
      ? proxy.passwordSecretRef.trim() || null
      : null,
  };
}
