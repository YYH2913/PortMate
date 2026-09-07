import { describe, expect, it } from "vitest";
import {
  MINIMUM_NODE_VERSION,
  assertSupportedNodeVersion,
  parseNodeVersion,
  supportsNodeVersion,
} from "./ensure-node-version.mjs";

describe("Node version guard", () => {
  it("parses complete Node semantic versions without accepting malformed input", () => {
    expect(parseNodeVersion("v24.20.0")).toEqual({ major: 24, minor: 20, patch: 0 });
    expect(parseNodeVersion("23.0.0-pre.1")).toEqual({ major: 23, minor: 0, patch: 0 });
    expect(parseNodeVersion("22.12")).toBeNull();
    expect(parseNodeVersion("v22.12.x")).toBeNull();
    expect(parseNodeVersion(null)).toBeNull();
  });

  it("accepts the supported boundary and every newer major release", () => {
    expect(supportsNodeVersion("24.19.99")).toBe(false);
    expect(supportsNodeVersion("24.20.0")).toBe(true);
    expect(supportsNodeVersion("24.20.1")).toBe(true);
    expect(supportsNodeVersion("25.0.0")).toBe(true);
    expect(supportsNodeVersion("23.99.99")).toBe(false);
    expect(supportsNodeVersion({ major: 24, minor: 20, patch: 0 }, MINIMUM_NODE_VERSION)).toBe(true);
  });

  it("reports an actionable error before a dependent tool starts", () => {
    expect(() => assertSupportedNodeVersion("18.19.1")).toThrow(
      "PortMate requires Node >=24.20.0; current runtime is 18.19.1. Run `nvm use` before running this command.",
    );
  });

  it("checks patch versions and rejects incomplete version objects", () => {
    expect(supportsNodeVersion("24.20.0", { major: 24, minor: 20, patch: 1 })).toBe(false);
    expect(supportsNodeVersion("24.20.1", { major: 24, minor: 20, patch: 1 })).toBe(true);
    expect(supportsNodeVersion({ major: 24, minor: 20 })).toBe(false);
    expect(supportsNodeVersion({ major: 24, minor: 20, patch: -1 })).toBe(false);
  });
});
