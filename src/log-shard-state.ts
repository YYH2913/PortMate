import type { LogShardInfo } from "./types";

export function filterLogShards(
  shards: LogShardInfo[],
  query: string,
  format: LogShardInfo["format"] | "all",
) {
  const normalizedQuery = query.trim().toLowerCase();
  return shards.filter((shard) =>
    (format === "all" || shard.format === format)
      && (!normalizedQuery || shard.path.toLowerCase().includes(normalizedQuery)),
  );
}

export function selectVisibleLogShards(selected: string[], visible: LogShardInfo[]) {
  return Array.from(new Set([...selected, ...visible.map((shard) => shard.path)]));
}
