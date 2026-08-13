import { describe, expect, it } from "vitest";
import {
  parseSwiftVersion,
  swiftVersionIsAtLeast,
} from "./swift-toolchain-version.mjs";

describe("Swift toolchain version", () => {
  it("parses upstream and Apple version output", () => {
    expect(parseSwiftVersion("Swift version 6.3.3 (swift-6.3.3-RELEASE)"))
      .toEqual([6, 3, 3]);
    expect(parseSwiftVersion("Apple Swift version 6.1.2 effective-5.10"))
      .toEqual([6, 1, 2]);
    expect(parseSwiftVersion("Swift version 6.1"))
      .toEqual([6, 1, 0]);
  });

  it("compares complete semantic versions without accepting malformed output", () => {
    expect(swiftVersionIsAtLeast("Apple Swift version 6.1.2", "6.1.0")).toBe(true);
    expect(swiftVersionIsAtLeast("Swift version 6.0.3", "6.1.0")).toBe(false);
    expect(swiftVersionIsAtLeast("Swift version 7.0.0", "6.1.0")).toBe(true);
    expect(swiftVersionIsAtLeast("Swift development snapshot", "6.1.0")).toBe(false);
    expect(swiftVersionIsAtLeast("Swift version 6.3.3", "6.1")).toBe(false);
  });
});
