import ipaddr from "ipaddr.js";
import type { LocalNetworkAddress } from "./types";

export const MCP_HTTP_ADDRESS_SELECTION_KEY = "portmate.mcpHttpAddressSelection.v1";
export const MCP_HTTP_CUSTOM_ADDRESS = "custom";
export type McpHttpAddressSelection =
  | { mode: "interface"; interfaceId: string; family: "ipv4" | "ipv6"; address: string }
  | { mode: "custom"; address: string };

export function normalizeMcpHttpAddressSelection(value: unknown): McpHttpAddressSelection | null {
  if (!value || typeof value !== "object") return null;
  const entry = value as Record<string, unknown>;
  if (typeof entry.address !== "string" || entry.address.length > 253) return null;
  if (entry.mode === "custom") return { mode: "custom", address: entry.address };
  if (entry.mode !== "interface" || typeof entry.interfaceId !== "string" || !entry.interfaceId
    || entry.interfaceId.length > 512 || !["ipv4", "ipv6"].includes(entry.family as string)) return null;
  return { mode: "interface", interfaceId: entry.interfaceId, family: entry.family as "ipv4" | "ipv6", address: entry.address };
}

export function normalizeLocalNetworkAddresses(value: unknown): LocalNetworkAddress[] {
  if (!Array.isArray(value)) return [];
  const unique = new Map<string, LocalNetworkAddress>();
  for (const item of value.slice(0, 1024)) {
    if (!item || typeof item !== "object" || typeof item.interfaceId !== "string" || !item.interfaceId
      || typeof item.interfaceName !== "string" || typeof item.address !== "string") continue;
    try {
      const ip = ipaddr.parse(item.address);
      const range = ip.range();
      if (!['ipv4', 'ipv6'].includes(item.family) || item.family !== ip.kind()
        || range === 'unspecified' || range === 'multicast'
        || ip.kind() === 'ipv6' && range === 'linkLocal') continue;
      const entry = { interfaceId: item.interfaceId, interfaceName: item.interfaceName,
        address: ip.toString(), family: item.family, loopback: ip.range() === 'loopback' } as LocalNetworkAddress;
      unique.set(networkAddressOptionValue(entry), entry);
    } catch { /* Ignore malformed native/legacy payloads. */ }
  }
  return [...unique.values()];
}

export function networkAddressOptionValue(entry: Pick<LocalNetworkAddress, "interfaceId" | "family" | "address">): string {
  return JSON.stringify([entry.interfaceId, entry.family, entry.address]);
}

/** Retain the adapter identity across DHCP changes; never substitute another adapter. */
export function rememberedNetworkAddress(addresses: readonly LocalNetworkAddress[], selection: McpHttpAddressSelection): LocalNetworkAddress | null {
  if (selection.mode !== "interface") return null;
  const matches = addresses.filter(entry => entry.interfaceId === selection.interfaceId && entry.family === selection.family);
  return matches.find(entry => entry.address === selection.address) ?? matches[0] ?? null;
}

export function initialMcpHttpAddressSelection(addresses: readonly LocalNetworkAddress[], remembered: McpHttpAddressSelection | null,
  currentHost: string): McpHttpAddressSelection {
  if (remembered) return remembered;
  let host = currentHost.trim().replace(/^\[|\]$/g, "");
  try { host = ipaddr.parse(host).toString(); } catch { /* Preserve DNS/custom addresses. */ }
  const matched = addresses.find(entry => entry.address === host);
  if (matched) return { mode: "interface", interfaceId: matched.interfaceId, family: matched.family, address: matched.address };
  if (!host) {
    const first = addresses.find(entry => entry.family === "ipv4" && !entry.loopback) ?? addresses[0];
    if (first) return { mode: "interface", interfaceId: first.interfaceId, family: first.family, address: first.address };
  }
  return { mode: "custom", address: currentHost };
}
