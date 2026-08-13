import { describe, expect, it } from "vitest";
import {
  compatibilityUsesCachedImages,
  filterCompatibilityEntries,
  prepareCompatibilityImage,
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

  it("inspects cached images without attempting a build", async () => {
    const calls = [];
    await prepareCompatibilityImage({
      run: (command, args, options) => {
        calls.push({ command, args, options });
        return { status: 0 };
      },
      image: "portmate-compat-cached:local",
      buildArgs: ["build", "."],
      useCachedImages: true,
    });

    expect(calls).toEqual([{
      command: "docker",
      args: ["image", "inspect", "portmate-compat-cached:local"],
      options: { quiet: true, allowFailure: true },
    }]);
  });

  it("retries image builds with bounded backoff and reports the image", async () => {
    const calls = [];
    const waits = [];
    await expect(prepareCompatibilityImage({
      run: (command, args, options) => {
        calls.push({ command, args, options });
        throw new Error(`network failure ${calls.length}`);
      },
      image: "portmate-compat-fedora:local",
      buildArgs: ["build", "--tag", "portmate-compat-fedora:local", "."],
      buildOptions: { timeout: 600_000 },
      useCachedImages: false,
      retryDelayMs: 5_000,
      wait: async (durationMs) => waits.push(durationMs),
    })).rejects.toThrow(
      "failed to prepare compatibility image portmate-compat-fedora:local after 3 attempts: network failure 3",
    );

    expect(calls).toHaveLength(3);
    expect(calls.every(({ options }) => options.timeout === 600_000)).toBe(true);
    expect(waits).toEqual([5_000, 10_000]);
  });

  it("rejects unsafe retry bounds before invoking Docker", async () => {
    await expect(prepareCompatibilityImage({
      run: () => {
        throw new Error("must not run");
      },
      image: "portmate-compat-invalid:local",
      buildArgs: ["build", "."],
      useCachedImages: false,
      attempts: 0,
    })).rejects.toThrow("attempts must be an integer from 1 to 10");
  });
});
