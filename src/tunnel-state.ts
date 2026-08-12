import ipaddr from "ipaddr.js";
import type { TunnelRouteRule } from "./types";

export const MAX_TUNNELS_PER_PROFILE = 64;
export const MAX_TUNNEL_ID_CHARACTERS = 128;
export const MAX_TUNNEL_LABEL_CHARACTERS = 128;
export const MAX_TUNNEL_HOST_CHARACTERS = 255;
export const MAX_TUNNEL_ROUTE_RULES = 64;

export function canAddTunnel(count: number): boolean {
  return Number.isInteger(count) && count >= 0 && count < MAX_TUNNELS_PER_PROFILE;
}

export function parseTunnelPort(value: string, allowZero: boolean): number | null {
  const normalized = value.trim();
  if (!/^\d{1,5}$/.test(normalized)) return null;
  const port = Number(normalized);
  if (!Number.isInteger(port) || port > 65_535 || (!allowZero && port === 0)) return null;
  return port;
}

export function isValidTunnelHostInput(value: string, allowEmpty = false): boolean {
  const normalized = value.trim();
  if (!allowEmpty && !normalized) return false;
  let count = 0;
  for (const character of normalized) {
    count += 1;
    if (count > MAX_TUNNEL_HOST_CHARACTERS || /\s/u.test(character)) return false;
    const codePoint = character.codePointAt(0) ?? 0;
    if (codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f)) return false;
  }
  return true;
}

export function normalizeTunnelRouteHost(value: string): string {
  if (hasControlCharacter(value)) return value;
  const normalized = value.trim().replace(/\.+$/u, "").toLowerCase();
  try {
    if (normalized.includes("/")) {
      const [address, prefix] = ipaddr.parseCIDR(normalized);
      return `${networkAddress(address, prefix).toString()}/${prefix}`;
    }
    if (ipaddr.isValid(normalized)) return ipaddr.parse(normalized).toString();
  } catch {
    return normalized;
  }
  return normalized;
}

export function isValidTunnelRouteHost(value: string): boolean {
  if (hasControlCharacter(value)) return false;
  const normalized = normalizeTunnelRouteHost(value);
  if (!normalized || normalized.length > MAX_TUNNEL_HOST_CHARACTERS || normalized !== value.trim().replace(/\.+$/u, "").toLowerCase()) {
    return false;
  }
  if (normalized.startsWith("*.")) return validDnsName(normalized.slice(2));
  if (normalized.includes("/")) {
    try {
      ipaddr.parseCIDR(normalized);
      return true;
    } catch {
      return false;
    }
  }
  return ipaddr.isValid(normalized) || validDnsName(normalized);
}

export function isValidTunnelRouteRules(rules: TunnelRouteRule[]): boolean {
  if (rules.length > MAX_TUNNEL_ROUTE_RULES) return false;
  const seen = new Set<string>();
  return rules.every((rule) => {
    if (hasControlCharacter(rule.host)) return false;
    const host = normalizeTunnelRouteHost(rule.host);
    const portValid = rule.port === null
      || (Number.isInteger(rule.port) && rule.port >= 1 && rule.port <= 65_535);
    const key = `${host}\0${rule.port ?? "*"}`;
    if (!portValid || !isValidTunnelRouteHost(host) || seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function hasControlCharacter(value: string): boolean {
  return [...value].some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f);
  });
}

function validDnsName(value: string): boolean {
  if (!value || value.length > 253 || value.startsWith(".") || value.endsWith(".")) return false;
  return value.split(".").every((label) => label.length > 0
    && label.length <= 63
    && !label.startsWith("-")
    && !label.endsWith("-")
    && /^[a-z0-9-]+$/u.test(label));
}

function networkAddress(address: ipaddr.IPv4 | ipaddr.IPv6, prefix: number): ipaddr.IPv4 | ipaddr.IPv6 {
  const bytes = address.toByteArray();
  let remaining = prefix;
  for (let index = 0; index < bytes.length; index += 1) {
    if (remaining >= 8) {
      remaining -= 8;
      continue;
    }
    bytes[index] &= remaining <= 0 ? 0 : (0xff << (8 - remaining)) & 0xff;
    remaining = 0;
  }
  return ipaddr.fromByteArray(bytes);
}
