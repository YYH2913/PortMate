import { describe, expect, it } from "vitest";
import {
  canAddTunnel,
  isValidTunnelHostInput,
  MAX_TUNNELS_PER_PROFILE,
  MAX_TUNNEL_HOST_CHARACTERS,
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
});
