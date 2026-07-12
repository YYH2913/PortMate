import { describe, expect, it } from "vitest";
import type { LogShardInfo } from "./types";
import { filterLogShards, selectVisibleLogShards } from "./log-shard-state";

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
});
