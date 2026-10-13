import { describe, expect, it } from "vitest";
import { initialMcpHttpAddressSelection, normalizeLocalNetworkAddresses, normalizeMcpHttpAddressSelection,
  rememberedNetworkAddress } from "../../src/mcp-http-address-selection";
import type { LocalNetworkAddress } from "../../src/types";

const nic: LocalNetworkAddress = { interfaceId: "adapter-guid", interfaceName: "Ethernet", address: "192.0.2.42", family: "ipv4", loopback: false };
const loopback: LocalNetworkAddress = { interfaceId: "lo", interfaceName: "lo", address: "127.0.0.1", family: "ipv4", loopback: true };

describe("MCP HTTP access address selection", () => {
  it("uses a real adapter entry for the saved/default local address", () => {
    expect(initialMcpHttpAddressSelection([nic, loopback], null, "127.0.0.1")).toEqual({ mode: "interface", interfaceId: "lo", family: "ipv4", address: "127.0.0.1" });
    expect(initialMcpHttpAddressSelection([nic, loopback], null, "")).toEqual({ mode: "interface", interfaceId: nic.interfaceId, family: "ipv4", address: nic.address });
  });

  it("restores the adapter identity after DHCP changes and preserves address family", () => {
    const remembered = { mode: "interface", interfaceId: nic.interfaceId, family: "ipv4", address: "192.0.2.7" } as const;
    expect(initialMcpHttpAddressSelection([nic], remembered, "127.0.0.1")).toEqual(remembered);
    expect(rememberedNetworkAddress([nic, { ...nic, address: "2001:db8::42", family: "ipv6" }], remembered)).toEqual(nic);
    expect(rememberedNetworkAddress([{ ...nic, interfaceId: "another-adapter" }], remembered)).toBeNull();
  });

  it("keeps an exact address when the adapter has multiple addresses", () => {
    const extra = { ...nic, address: "192.0.2.43" };
    expect(rememberedNetworkAddress([nic, extra], { mode: "interface", interfaceId: nic.interfaceId, family: "ipv4", address: extra.address })).toEqual(extra);
  });

  it("preserves legacy DNS/custom settings and restores explicit Custom selection", () => {
    expect(initialMcpHttpAddressSelection([nic], null, "mcp.example.test")).toEqual({ mode: "custom", address: "mcp.example.test" });
    expect(initialMcpHttpAddressSelection([nic], { mode: "custom", address: nic.address }, "127.0.0.1")).toEqual({ mode: "custom", address: nic.address });
  });

  it("normalizes persistence without accepting incomplete interface identities", () => {
    expect(normalizeMcpHttpAddressSelection({ mode: "interface", address: nic.address })).toBeNull();
    expect(normalizeMcpHttpAddressSelection({ mode: "interface", interfaceId: "a", family: "localized IPv4", address: nic.address })).toBeNull();
    expect(normalizeMcpHttpAddressSelection({ mode: "custom", address: "" })).toEqual({ mode: "custom", address: "" });
  });

  it("filters invalid and URL-incompatible addresses while retaining IPv4 and global IPv6", () => {
    expect(normalizeLocalNetworkAddresses([nic, nic, loopback, { ...nic, address: "0.0.0.0" }, { ...nic, address: "224.0.0.1" },
      { ...nic, family: "ipv6", address: "fe80::1" }, { ...nic, family: "ipv6", address: "2001:0db8::42" },
      { ...nic, address: "garbage" }])).toEqual([nic, loopback, { ...nic, family: "ipv6", address: "2001:db8::42" }]);
  });
});
