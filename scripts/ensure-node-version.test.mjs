import { describe, expect, it } from "vitest";
import {
  MINIMUM_NODE_VERSION,
  assertSupportedNodeVersion,
  parseNodeVersion,
  supportsNodeVersion,
} from "./ensure-node-version.mjs";

describe("Node version guard", () => {
  it("parses complete Node semantic versions without accepting malformed input", () => {
    expect(parseNodeVersion("v22.20.0")).toEqual({ major: 22, minor: 20, patch: 0 });
    expect(parseNodeVersion("23.0.0-pre.1")).toEqual({ major: 23, minor: 0, patch: 0 });
    expect(parseNodeVersion("22.12")).toBeNull();
    expect(parseNodeVersion("v22.12.x")).toBeNull();
    expect(parseNodeVersion(null)).toBeNull();
  });

  it("accepts the supported boundary and every newer major release", () => {
    expect(supportsNodeVersion("22.11.99")).toBe(false);
    expect(supportsNodeVersion("22.12.0")).toBe(true);
    expect(supportsNodeVersion("22.12.1")).toBe(true);
    expect(supportsNodeVersion("23.0.0")).toBe(true);
    expect(supportsNodeVersion("21.99.99")).toBe(false);
    expect(supportsNodeVersion({ major: 22, minor: 12, patch: 0 }, MINIMUM_NODE_VERSION)).toBe(true);
  });

  it("reports an actionable error before a dependent tool starts", () => {
    expect(() => assertSupportedNodeVersion("18.19.1")).toThrow(
      "PortMate requires Node >=22.12.0; current runtime is 18.19.1. Run `nvm use` before running this command.",
    );
  });
});
