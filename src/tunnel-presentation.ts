import type { TunnelSpec } from "./types";

export const tunnelModePresentation = {
  local: {
    option: "tunnel-mode-local",
    description: "tunnel-help-local",
    bindLabel: "tunnel-bind-local",
    bindHint: "tunnel-bind-local-hint",
    targetHint: "tunnel-target-ssh-hint",
    entrySide: "tunnel-side-local-entry",
    targetSide: "tunnel-side-ssh-target",
  },
  remote: {
    option: "tunnel-mode-remote",
    description: "tunnel-help-remote",
    bindLabel: "tunnel-bind-remote",
    bindHint: "tunnel-bind-remote-hint",
    targetHint: "tunnel-target-local-hint",
    entrySide: "tunnel-side-remote-entry",
    targetSide: "tunnel-side-local-target",
  },
  dynamic: {
    option: "tunnel-mode-dynamic",
    description: "tunnel-help-dynamic",
    bindLabel: "tunnel-bind-local",
    bindHint: "tunnel-bind-local-hint",
    targetHint: "tunnel-target-ssh-hint",
    entrySide: "tunnel-side-proxy-entry",
    targetSide: "tunnel-side-dynamic-target",
  },
} as const satisfies Record<TunnelSpec["mode"], Record<string, string>>;

/** Display only; submitted hosts/ports retain their existing validation. */
export function formatTunnelEndpoint(host: string, port: string | number, automaticPort?: string): string {
  const trimmed = host.trim();
  const address = trimmed.includes(":") && !trimmed.startsWith("[") ? `[${trimmed}]` : trimmed || "…";
  const portText = String(port).trim();
  return `${address}:${portText === "0" && automaticPort ? automaticPort : portText || "…"}`;
}
