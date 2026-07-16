import type { LogShardInfo } from "./types";

export const MAX_BUNDLE_ATTACHMENTS = 32;
export const MAX_BUNDLE_ATTACHMENT_BYTES = 16 * 1024 * 1024;
export const MAX_BUNDLE_ATTACHMENT_TOTAL_BYTES = 32 * 1024 * 1024;

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

export function summarizeBundleAttachmentSelection(
  shards: LogShardInfo[],
  selected: string[],
) {
  const selectedPaths = new Set(selected);
  const selectedShards = shards.filter((shard) => selectedPaths.has(shard.path));
  const bytes = selectedShards.reduce((sum, shard) => sum + shard.size, 0);
  return {
    count: selectedShards.length,
    bytes,
    withinLimits: selectedShards.length <= MAX_BUNDLE_ATTACHMENTS
      && bytes <= MAX_BUNDLE_ATTACHMENT_TOTAL_BYTES
      && selectedShards.every((shard) => shard.size <= MAX_BUNDLE_ATTACHMENT_BYTES),
  };
}
