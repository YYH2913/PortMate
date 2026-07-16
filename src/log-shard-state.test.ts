import { describe, expect, it } from "vitest";
import type { LogShardInfo } from "./types";
import {
  filterLogShards,
  MAX_BUNDLE_ATTACHMENT_BYTES,
  MAX_BUNDLE_ATTACHMENTS,
  MAX_BUNDLE_ATTACHMENT_TOTAL_BYTES,
  selectVisibleLogShards,
  summarizeBundleAttachmentSelection,
} from "./log-shard-state";

const shards: LogShardInfo[] = [
  { path: "Bench/2026-07-12/session.raw", format: "raw", size: 10 },
  { path: "bench/2026-07-12/session.txt", format: "txt", size: 20 },
  { path: "other/session.jsonl", format: "jsonl", size: 30 },
];

describe("log shard state", () => {
  it("filters paths case-insensitively", () => {
    expect(filterLogShards(shards, "BENCH", "all")).toHaveLength(2);
  });

  it("combines path and format filters", () => {
    expect(filterLogShards(shards, "session", "jsonl")).toEqual([shards[2]]);
  });

  it("returns all shards for an empty all-format filter", () => {
    expect(filterLogShards(shards, "  ", "all")).toEqual(shards);
  });

  it("adds visible paths without duplicating existing selection", () => {
    expect(selectVisibleLogShards([shards[0].path], shards.slice(0, 2))).toEqual([
      shards[0].path,
      shards[1].path,
    ]);
  });

  it("summarizes only existing selected shards for bundle attachments", () => {
    expect(summarizeBundleAttachmentSelection(shards, [shards[0].path, "missing.raw"])).toEqual({
      count: 1,
      bytes: 10,
      withinLimits: true,
    });
  });

  it("rejects bundle attachment selections outside count or total-size limits", () => {
    const tooMany = Array.from({ length: MAX_BUNDLE_ATTACHMENTS + 1 }, (_, index) => ({
      path: `${index}.txt`,
      format: "txt" as const,
      size: 1,
    }));
    expect(summarizeBundleAttachmentSelection(tooMany, tooMany.map((shard) => shard.path)).withinLimits).toBe(false);
    const tooLarge = [{
      path: "large.raw",
      format: "raw" as const,
      size: MAX_BUNDLE_ATTACHMENT_TOTAL_BYTES + 1,
    }];
    expect(summarizeBundleAttachmentSelection(tooLarge, ["large.raw"]).withinLimits).toBe(false);
    const oversizedSingle = [{
      path: "single.raw",
      format: "raw" as const,
      size: MAX_BUNDLE_ATTACHMENT_BYTES + 1,
    }];
    expect(summarizeBundleAttachmentSelection(oversizedSingle, ["single.raw"]).withinLimits).toBe(false);
  });
});
