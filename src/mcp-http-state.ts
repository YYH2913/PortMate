import type { McpHttpConfig, McpHttpConfigRequest } from "./types";

export const MCP_HTTP_DEFAULT_PORT = 8787;
export const CC_SWITCH_DEFAULT_SERVER_ID = "portmate";
export const CC_SWITCH_DEFAULT_TOOL_TIMEOUT_SECONDS = 180;
export const MCP_HTTP_LISTEN_PRESETS = ["127.0.0.1", "0.0.0.0", "::1", "::"] as const;
export const MCP_HTTP_CUSTOM_LISTEN_PRESET = "custom" as const;

export type McpHttpListenPreset = typeof MCP_HTTP_LISTEN_PRESETS[number]
  | typeof MCP_HTTP_CUSTOM_LISTEN_PRESET;

export function defaultMcpHttpOrigins(port = MCP_HTTP_DEFAULT_PORT): string[] {
  return [`http://127.0.0.1:${port}`, `http://localhost:${port}`];
}

export function defaultMcpHttpSettings(): McpHttpConfigRequest {
  return {
    listenHost: "127.0.0.1",
    clientHost: "127.0.0.1",
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

export function mcpHttpListenPreset(value: string): McpHttpListenPreset {
  const host = value.trim().replace(/^\[|\]$/g, "");
  return MCP_HTTP_LISTEN_PRESETS.find((preset) => preset === host)
    ?? MCP_HTTP_CUSTOM_LISTEN_PRESET;
}

export function mcpHttpSettingsFromConfig(config: McpHttpConfig): McpHttpConfigRequest {
  return {
    listenHost: config.listenHost,
    clientHost: config.clientHost,
    port: config.port,
    allowedOrigins: [...config.allowedOrigins],
    clientId: config.clientId,
    trusted: config.trusted,
    allowRemote: config.allowRemote,
  };
}

export function mcpHttpClientEndpoint(settings: Pick<McpHttpConfigRequest, "clientHost" | "port">): string | null {
  const host = normalizeMcpHttpClientHost(settings.clientHost);
  if (!host || !Number.isInteger(settings.port) || settings.port < 1 || settings.port > 65_535) return null;
  const urlHost = host.includes(":") ? `[${host}]` : host;
  return `http://${urlHost}:${settings.port}/mcp`;
}

export function formatCcSwitchMcpJson(
  settings: Pick<McpHttpConfigRequest, "clientHost" | "port">,
  options: {
    serverId?: string;
    token?: string;
    toolTimeoutSeconds?: number;
  } = {},
): string {
  const serverId = (options.serverId ?? CC_SWITCH_DEFAULT_SERVER_ID).trim();
  const token = (options.token ?? "").trim();
  const toolTimeoutSeconds = options.toolTimeoutSeconds ?? CC_SWITCH_DEFAULT_TOOL_TIMEOUT_SECONDS;
  const url = mcpHttpClientEndpoint(settings);
  if (!url
    || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(serverId)
    || !token
    || token.length > 4_096
    || !Number.isInteger(toolTimeoutSeconds)
    || toolTimeoutSeconds < 1
    || toolTimeoutSeconds > 3_600) return "";
  return JSON.stringify({
    [serverId]: {
      type: "http",
      url,
      headers: {
        Authorization: `Bearer ${token}`,
      },
      tool_timeout_sec: toolTimeoutSeconds,
    },
  }, null, 2);
}

export function ccSwitchServerIdForGrant(clientId: string): string {
  const suffix = clientId
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^[._-]+|[._-]+$/g, "");
  if (!suffix) return CC_SWITCH_DEFAULT_SERVER_ID;
  return `${CC_SWITCH_DEFAULT_SERVER_ID}-${suffix}`
    .slice(0, 64)
    .replace(/[._-]+$/g, "") || CC_SWITCH_DEFAULT_SERVER_ID;
}

function normalizeMcpHttpClientHost(value: string): string | null {
  const host = value.trim().replace(/^\[|\]$/g, "");
  if (!host || /[\s/?#@]/.test(host)) return null;
  try {
    const parsed = new URL(`http://${host.includes(":") ? `[${host}]` : host}`);
    const normalized = parsed.hostname.replace(/^\[|\]$/g, "");
    return normalized === "0.0.0.0" || normalized === "::" ? null : normalized;
  } catch {
    return null;
  }
}
