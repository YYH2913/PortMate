import { describe, expect, it } from "vitest";
import { defaultEasyTierConfig, easyTierConfigValid, easyTierEndpoints, easyTierSecretValid } from "../../src/easytier-state";

describe("EasyTier network configuration", () => {
  it("normalizes endpoint lines without interpreting shell syntax", () => {
    expect(easyTierEndpoints("  udp://peer:11010\r\n\ntcp://[::1]:11010 \n")).toEqual(["udp://peer:11010", "tcp://[::1]:11010"]);
    expect(easyTierConfigValid({ ...defaultEasyTierConfig(), peers: ["tcp://user:password@host:11010"] })).toBe(false);
    expect(easyTierConfigValid({ ...defaultEasyTierConfig(), peers: ["udp://0.0.0.0:11010"] })).toBe(false);
    expect(easyTierConfigValid({ ...defaultEasyTierConfig(), peers: ["tcp://[::1]:11010"] })).toBe(true);
  });
  it("rejects expansion and invalid virtual addresses before sending a request", () => {
    expect(easyTierConfigValid({ ...defaultEasyTierConfig(), networkName: "${HOME}" })).toBe(false);
    for (const virtualIpv4 of ["127.0.0.1", "224.0.0.1", "0.0.0.0", "10.1.1.300", "10.1.1.2/24"]) {
      expect(easyTierConfigValid({ ...defaultEasyTierConfig(), addressMode: "static", virtualIpv4 })).toBe(false);
    }
    expect(easyTierConfigValid({ ...defaultEasyTierConfig(), addressMode: "static", virtualIpv4: "10.1.1.2" })).toBe(true);
    expect(easyTierConfigValid({ ...defaultEasyTierConfig(), addressMode: "relay" })).toBe(true);
  });
  it("accepts literal Unicode passwords but rejects empty, multiline, or expanded secrets", () => {
    expect(easyTierSecretValid('中文-quote"-backslash\\')).toBe(true);
    for (const secret of ["", " ", "abc\ndef", "${SECRET}"]) expect(easyTierSecretValid(secret)).toBe(false);
    expect(Object.keys(defaultEasyTierConfig())).not.toContain("networkSecret");
  });
});
