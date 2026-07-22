import { describe, expect, it } from "vitest";
import { formatBytes, formatDuration, formatEventClock } from "./display-formatters";

describe("display formatters", () => {
  it("formats byte boundaries without changing units early", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1024)).toBe("1.0 KiB");
    expect(formatBytes(1024 * 1024)).toBe("1.0 MiB");
    expect(formatBytes(1024 * 1024 * 1024)).toBe("1.0 GiB");
  });

  it("formats millisecond, second and minute transfer durations", () => {
    const start = "2026-07-22T00:00:00.000Z";
    expect(formatDuration(start, "2026-07-22T00:00:00.500Z")).toBe("500 ms");
    expect(formatDuration(start, "2026-07-22T00:00:01.500Z")).toBe("1.5 s");
    expect(formatDuration(start, "2026-07-22T00:01:05.000Z")).toBe("1m 5s");
    expect(formatDuration("invalid", "invalid")).toBe("");
  });

  it("uses a stable placeholder for an invalid event timestamp", () => {
    expect(formatEventClock("invalid")).toBe("--:--:--");
  });
});
