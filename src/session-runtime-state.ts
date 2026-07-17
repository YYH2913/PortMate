import type { SessionStatus } from "./types";

export function sessionConnectionAction(status: SessionStatus): "connect" | "disconnect" {
  return status === "connected" || status === "connecting" || status === "reconnecting"
    ? "disconnect"
    : "connect";
}
