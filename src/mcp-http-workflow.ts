import { t } from "./i18n";
import type { McpHttpAccessResponse, McpHttpConfig, McpHttpConfigRequest, McpHttpRuntimeStatus, McpHttpTokenResponse } from "./types";
import { mcpHttpSettingsFromConfig } from "./mcp-http-state";

export type McpInvoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

/** Save before generating a token: the backend invalidates the old token on rebind. */
export async function prepareMcpHttpAccess(
  invoke: McpInvoke,
  settings: McpHttpConfigRequest,
  save: boolean,
  generateToken: boolean,
  isCurrent: () => boolean = () => true,
  expectedSavedSettings?: McpHttpConfigRequest,
): Promise<McpHttpAccessResponse | null> {
  if (!isCurrent()) return null;
  let expected = settings;
  if (save) {
    const saved = await invoke<McpHttpConfig>("save_mcp_http_settings", { settings, ...(expectedSavedSettings ? { expectedSettings: expectedSavedSettings } : {}) });
    if (!isCurrent()) return null;
    expected = mcpHttpSettingsFromConfig(saved);
  }
  // Always read canonical access, not a token cached by a different window.
  let access = await invoke<McpHttpAccessResponse>("mcp_http_access_config", {});
  if (!isCurrent()) return null;
  assertMcpHttpSettingsUnchanged(expected, access.config);
  if (generateToken && !access.token) {
    access = await invoke<McpHttpTokenResponse>("rotate_mcp_http_token", { expectedSettings: mcpHttpSettingsFromConfig(access.config) });
    if (!isCurrent()) return null;
  }
  return access;
}

export function assertMcpHttpSettingsUnchanged(expected: McpHttpConfigRequest, actual: McpHttpConfigRequest): void {
  const values = (settings: McpHttpConfigRequest) => [
    settings.clientId, settings.listenHost, settings.clientHost, settings.port,
    settings.trusted, settings.allowRemote, settings.allowedOrigins,
  ];
  if (JSON.stringify(values(expected)) !== JSON.stringify(values(actual))) {
    throw new Error(t("http-settings-were-changed-by-another-window-refresh-and"));
  }
}

export function mcpHttpRuntimeLabel(status: McpHttpRuntimeStatus | null): string {
  switch (status?.phase) {
    case "starting": return t("starting-2");
    case "running": return t("running");
    case "failed": return t("start-failed");
    case "stopped": return t("not-running");
    default: return t("reading-status");
  }
}
