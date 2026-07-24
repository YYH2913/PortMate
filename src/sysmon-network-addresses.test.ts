import { describe, expect, it } from "vitest";
import { formatSysmonNetworkAddresses, orderedSysmonNetworkAddresses } from "./sysmon-network-addresses";

describe("Sysmon network address display", () => {
  it("puts usable IPv4 and global IPv6 addresses before link-local and loopback addresses", () => {
    const addresses = [
      "fe80::25/64",
      "127.0.0.1/8",
      "192.168.33.121/24",
      "2001:db8::42/64",
      "169.254.8.9/16",
      "::1/128",
    ];

    expect(orderedSysmonNetworkAddresses(addresses)).toEqual([
      "192.168.33.121/24",
      "2001:db8::42/64",
      "169.254.8.9/16",
      "fe80::25/64",
      "127.0.0.1/8",
      "::1/128",
    ]);
    expect(formatSysmonNetworkAddresses(addresses)).toBe("192.168.33.121/24 · 2001:db8::42/64 +4");
  });

  it("keeps equal-priority addresses stable and ignores empty display values", () => {
    expect(orderedSysmonNetworkAddresses([" 10.0.0.2/24 ", "", "10.0.0.1/24", "   "])).toEqual([
      "10.0.0.2/24",
      "10.0.0.1/24",
    ]);
    expect(formatSysmonNetworkAddresses([])).toBe("-");
  });
});
