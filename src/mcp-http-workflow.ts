import type { McpHttpAccessResponse, McpHttpConfig, McpHttpConfigRequest, McpHttpRuntimeStatus, McpHttpTokenResponse } from "./types";

export type McpInvoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

/** Save before generating a token: the backend invalidates the old token on rebind. */
export async function prepareMcpHttpAccess(
  invoke: McpInvoke,
  settings: McpHttpConfigRequest,
  save: boolean,
  generateToken: boolean,
  isCurrent: () => boolean = () => true,
): Promise<McpHttpAccessResponse | null> {
  if (!isCurrent()) return null;
  if (save) {
    await invoke<McpHttpConfig>("save_mcp_http_settings", { settings });
    if (!isCurrent()) return null;
  }
  // Always read canonical access, not a token cached by a different window.
  let access = await invoke<McpHttpAccessResponse>("mcp_http_access_config", {});
  if (!isCurrent()) return null;
  if (generateToken && !access.token) {
    access = await invoke<McpHttpTokenResponse>("rotate_mcp_http_token", {});
    if (!isCurrent()) return null;
  }
  return access;
}

export function mcpHttpRuntimeLabel(status: McpHttpRuntimeStatus | null): string {
  switch (status?.phase) {
    case "starting": return "正在启动";
    case "running": return "运行中";
    case "failed": return "启动失败";
    case "stopped": return "未运行";
    default: return "读取状态";
  }
}
