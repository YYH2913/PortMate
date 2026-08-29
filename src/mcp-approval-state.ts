import type { McpApprovalRequest, McpScope } from "./types";

export const MCP_APPROVAL_EVENT = "portmate-mcp-approval";
export const MAX_MCP_APPROVAL_QUEUE = 32;

const approvalActionScopes: Record<string, McpScope> = {
  send_text: "write-input",
  send_bytes: "write-input",
  send_key: "write-input",
  serial_send_break: "write-input",
  run_command: "write-input",
  run_local_command: "write-input",
  run_custom_script: "run-scripts",
  attach_tmux: "write-input",
  start_transfer: "transfer",
  cancel_transfer: "transfer",
  retry_transfer: "transfer",
  create_tunnel: "tunnel",
  stop_tunnel: "tunnel",
  tunnel_request: "tunnel",
  udp_request: "tunnel",
  restart_mcp_http: "manage-mcp",
};

const approvalIdPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const controlCharacters = /[\u0000-\u001f\u007f]/;

export function normalizeMcpApproval(value: unknown): McpApprovalRequest | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  if (typeof source.id !== "string" || !approvalIdPattern.test(source.id)) return null;
  if (!validText(source.clientId, 128) || !validText(source.action, 64) || !validText(source.sessionId, 128)) return null;
  if (typeof source.scope !== "string" || approvalActionScopes[source.action] !== source.scope) return null;
  if (typeof source.createdAt !== "string" || typeof source.expiresAt !== "string") return null;
  const createdAt = Date.parse(source.createdAt);
  const expiresAt = Date.parse(source.expiresAt);
  if (!Number.isFinite(createdAt) || !Number.isFinite(expiresAt) || expiresAt <= createdAt || expiresAt - createdAt > 65_000) return null;
  const target = normalizeApprovalTarget(source.action, source.target);
  if (target === null) return null;
  return {
    id: source.id.toLowerCase(),
    clientId: source.clientId,
    action: source.action,
    sessionId: source.sessionId,
    scope: source.scope as McpScope,
    ...(target ? { target } : {}),
    createdAt: new Date(createdAt).toISOString(),
    expiresAt: new Date(expiresAt).toISOString(),
  };
}

function normalizeApprovalTarget(action: string, value: unknown): McpApprovalRequest["target"] | null {
  const expectedKind = action === "run_custom_script"
    ? "custom-script"
    : action === "create_tunnel"
      ? "portmate-host-proxy"
      : action === "tunnel_request" || action === "udp_request"
        ? "portmate-host-tunnel-request"
        : ["send_text", "send_bytes", "send_key", "run_command", "run_local_command"].includes(action)
          ? "command"
          : action === "start_transfer"
            ? "transfer"
            : ["cancel_transfer", "retry_transfer", "attach_tmux", "stop_tunnel"].includes(action)
              ? "operation"
              : null;
  if (!expectedKind) return value === undefined ? undefined : null;
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  if (source.kind !== expectedKind || !validText(source.label, 512) || !validText(source.id, 512)) return null;
  if (expectedKind === "custom-script" && !approvalIdPattern.test(source.id)) return null;
  return {
    kind: expectedKind,
    id: expectedKind === "custom-script" ? source.id.toLowerCase() : source.id,
    label: source.label,
  };
}

export function mergeMcpApprovals(
  current: readonly McpApprovalRequest[],
  incoming: unknown,
  now = Date.now(),
  resolvedIds: ReadonlySet<string> = new Set(),
  retainedIds: ReadonlySet<string> = new Set(),
): McpApprovalRequest[] {
  const merged = new Map<string, McpApprovalRequest>();
  const candidates = Array.isArray(incoming) ? incoming : [];
  for (const value of [...current, ...candidates]) {
    const request = normalizeMcpApproval(value);
    if (request
      && !resolvedIds.has(request.id)
      && (Date.parse(request.expiresAt) > now || retainedIds.has(request.id))) {
      merged.set(request.id, request);
    }
  }
  const next = [...merged.values()]
    .sort((left, right) => Date.parse(left.createdAt) - Date.parse(right.createdAt) || left.id.localeCompare(right.id))
    .slice(0, MAX_MCP_APPROVAL_QUEUE);
  if (next.length === current.length && next.every((item, index) => sameApproval(item, current[index]))) {
    // The one-second expiry sweep is intentionally cheap when nothing changed.
    // Reusing the array prevents the whole workspace from rendering again.
    return current as McpApprovalRequest[];
  }
  return next;
}

function sameApproval(left: McpApprovalRequest, right: McpApprovalRequest | undefined): boolean {
  if (!right) return false;
  return left.id === right.id
    && left.clientId === right.clientId
    && left.action === right.action
    && left.sessionId === right.sessionId
    && left.scope === right.scope
    && left.createdAt === right.createdAt
    && left.expiresAt === right.expiresAt
    && JSON.stringify(left.target ?? null) === JSON.stringify(right.target ?? null);
}

function validText(value: unknown, maxBytes: number): value is string {
  return typeof value === "string"
    && value.length > 0
    && new TextEncoder().encode(value).length <= maxBytes
    && !controlCharacters.test(value);
}
