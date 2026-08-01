import { describe, expect, it } from "vitest";
import {
  compatibilityUsesCachedImages,
  filterCompatibilityEntries,
} from "./compat-docker-images.mjs";

const entries = [
  { name: "alpha-1" },
  { name: "beta.2" },
  { name: "gamma-3" },
];

describe("Docker compatibility matrix selection", () => {
  it("keeps the complete matrix when no filter is configured", () => {
    expect(filterCompatibilityEntries(entries, {})).toBe(entries);
  });

  it("selects exact names while preserving matrix order", () => {
    expect(filterCompatibilityEntries(entries, {
      PORTMATE_COMPAT_FILTER: " gamma-3,alpha-1 ",
    })).toEqual([entries[0], entries[2]]);
  });

  it.each([
    ["", "non-empty string"],
    ["alpha-1,,beta.2", "without empty entries"],
    ["alpha-1,alpha-1", "duplicate entry names"],
    ["alpha_1", "invalid entry name"],
    ["missing", "unknown entries"],
  ])("rejects invalid filter %j", (filter, message) => {
    expect(() => filterCompatibilityEntries(entries, {
      PORTMATE_COMPAT_FILTER: filter,
    })).toThrow(message);
  });

  it("supports qualified names for matrices with overlapping local names", () => {
    const qualified = [
      { name: "health.sftp-missing" },
      { name: "transfer.sftp-missing" },
    ];
    expect(filterCompatibilityEntries(qualified, {
      PORTMATE_COMPAT_FILTER: "transfer.sftp-missing",
    })).toEqual([qualified[1]]);
  });

  it("rejects duplicate names across matrix groups even without a filter", () => {
    expect(() => filterCompatibilityEntries(entries, {}, [
      ...entries,
      { name: "alpha-1" },
    ])).toThrow("duplicate entry name");
  });
});

describe("Docker compatibility cache mode", () => {
  it("accepts only explicit zero or one", () => {
    expect(compatibilityUsesCachedImages({})).toBe(false);
    expect(compatibilityUsesCachedImages({ PORTMATE_COMPAT_USE_CACHED_IMAGES: "1" })).toBe(true);
    expect(() => compatibilityUsesCachedImages({
      PORTMATE_COMPAT_USE_CACHED_IMAGES: "yes",
    })).toThrow("must be 0 or 1");
  });
});
