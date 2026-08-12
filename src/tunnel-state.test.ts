import { describe, expect, it } from "vitest";
import {
  canAddTunnel,
  isValidTunnelHostInput,
  isValidTunnelRouteHost,
  isValidTunnelRouteRules,
  MAX_TUNNELS_PER_PROFILE,
  MAX_TUNNEL_HOST_CHARACTERS,
  normalizeTunnelRouteHost,
  parseTunnelPort,
} from "./tunnel-state";

describe("tunnel state", () => {
  it("parses bind and target ports without coercing ambiguous input", () => {
    expect(parseTunnelPort("0", true)).toBe(0);
    expect(parseTunnelPort("65535", false)).toBe(65_535);
    expect(parseTunnelPort("0", false)).toBeNull();
    expect(parseTunnelPort("65536", true)).toBeNull();
    expect(parseTunnelPort("1e3", true)).toBeNull();
    expect(parseTunnelPort("1.5", true)).toBeNull();
  });

  it("validates host and profile count boundaries", () => {
    expect(isValidTunnelHostInput(" 127.0.0.1 ")).toBe(true);
    expect(isValidTunnelHostInput("::1")).toBe(true);
    expect(isValidTunnelHostInput("bad host")).toBe(false);
    expect(isValidTunnelHostInput("bad\nhost")).toBe(false);
    expect(isValidTunnelHostInput("", true)).toBe(true);
    expect(isValidTunnelHostInput("x".repeat(MAX_TUNNEL_HOST_CHARACTERS + 1))).toBe(false);
    expect(canAddTunnel(MAX_TUNNELS_PER_PROFILE - 1)).toBe(true);
    expect(canAddTunnel(MAX_TUNNELS_PER_PROFILE)).toBe(false);
  });

  it("normalizes and validates domain, IP, CIDR, wildcard, and port route rules", () => {
    expect(normalizeTunnelRouteHost("  *.Example.COM. ")).toBe("*.example.com");
    expect(normalizeTunnelRouteHost("10.9.8.7/8")).toBe("10.0.0.0/8");
    expect(normalizeTunnelRouteHost("2001:0DB8::1/32")).toBe("2001:db8::/32");
    for (const host of ["example.com", "*.example.com", "10.0.0.1", "10.0.0.0/8", "2001:db8::/32"]) {
      expect(isValidTunnelRouteHost(host)).toBe(true);
    }
    for (const host of ["*", "*.bad_domain", "bad..host", "10.0.0.1/999", "bad host", "\nexample.com"]) {
      expect(isValidTunnelRouteHost(host)).toBe(false);
    }
    expect(isValidTunnelRouteRules([
      { host: "*.example.com", port: 443 },
      { host: "10.0.0.0/8", port: null },
    ])).toBe(true);
    expect(isValidTunnelRouteRules([
      { host: "example.com", port: null },
      { host: "EXAMPLE.COM", port: null },
    ])).toBe(false);
    expect(isValidTunnelRouteRules([{ host: "\texample.com", port: null }])).toBe(false);
    expect(isValidTunnelRouteRules([{ host: "example.com", port: 0 }])).toBe(false);
  });
});
