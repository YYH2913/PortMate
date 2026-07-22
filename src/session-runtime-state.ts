import type { SessionRuntime, SessionStatus } from "./types";

const statusLabels: Record<SessionStatus, string> = {
  disconnected: "已断开",
  connecting: "正在连接",
  connected: "已连接",
  reconnecting: "正在重连",
  blocked: "连接已阻止",
  error: "连接错误",
};

const MAX_DISCONNECT_REASON_CHARS = 256;

export function sessionConnectionAction(status: SessionStatus): "connect" | "disconnect" {
  return status === "connected" || status === "connecting" || status === "reconnecting"
    ? "disconnect"
    : "connect";
}

export function sessionRuntimeStatusLabel(status: SessionStatus): string {
  return statusLabels[status];
}

export function sessionRuntimeHealthDescription(
  runtime: SessionRuntime,
  formatTimestamp: (value: string) => string = formatRuntimeTimestamp,
): string {
  const parts = [sessionRuntimeStatusLabel(runtime.status)];
  if (runtime.lastDisconnect && Number.isFinite(Date.parse(runtime.lastDisconnect))) {
    const timestamp = formatTimestamp(runtime.lastDisconnect).trim();
    if (timestamp) parts.push(`上次断开 ${timestamp}`);
  }
  const reason = normalizeDisconnectReason(runtime.lastDisconnectReason);
  if (reason) parts.push(`原因: ${reason}`);
  return parts.join(" · ");
}

function normalizeDisconnectReason(value: string | null | undefined): string {
  const normalized = value?.replace(/\s+/g, " ").trim() ?? "";
  const characters = Array.from(normalized);
  if (characters.length <= MAX_DISCONNECT_REASON_CHARS) return normalized;
  return `${characters.slice(0, MAX_DISCONNECT_REASON_CHARS - 3).join("")}...`;
}

function formatRuntimeTimestamp(value: string): string {
  return new Date(value).toLocaleString();
}
