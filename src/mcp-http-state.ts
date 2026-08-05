import type { McpHttpConfig, McpHttpConfigRequest } from "./types";

export const MCP_HTTP_DEFAULT_PORT = 8787;

export function defaultMcpHttpOrigins(port = MCP_HTTP_DEFAULT_PORT): string[] {
  return [`http://127.0.0.1:${port}`, `http://localhost:${port}`];
}

export function defaultMcpHttpSettings(): McpHttpConfigRequest {
  return {
    listenHost: "127.0.0.1",
    port: MCP_HTTP_DEFAULT_PORT,
    allowedOrigins: defaultMcpHttpOrigins(),
    clientId: "portmate-local",
    trusted: false,
    allowRemote: false,
  };
}

export function parseMcpHttpOrigins(value: string): string[] {
  const origins: string[] = [];
  for (const candidate of value.split(/[\n,]/)) {
    const origin = candidate.trim();
    if (origin && !origins.includes(origin)) origins.push(origin);
  }
  return origins;
}

export function formatMcpHttpOrigins(origins: readonly string[]): string {
  return origins.join("\n");
}

export function isNonLoopbackMcpHost(value: string): boolean {
  const host = value.trim().toLowerCase().replace(/^\[|\]$/g, "");
  if (!host || host === "localhost" || host === "::1") return false;
  const octets = host.split(".");
  if (octets.length === 4 && octets.every((octet) => /^\d{1,3}$/.test(octet) && Number(octet) <= 255)) {
    return Number(octets[0]) !== 127;
  }
  return true;
}

export function mcpHttpSettingsFromConfig(config: McpHttpConfig): McpHttpConfigRequest {
  return {
    listenHost: config.listenHost,
    port: config.port,
    allowedOrigins: [...config.allowedOrigins],
    clientId: config.clientId,
    trusted: config.trusted,
    allowRemote: config.allowRemote,
  };
}
