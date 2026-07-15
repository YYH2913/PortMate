import type { OneKeySummary } from "./types";

export function sshOneKeysForSession(oneKeys: readonly OneKeySummary[], sessionId: string): OneKeySummary[] {
  return oneKeys.filter((oneKey) => (
    oneKey.kind === "ssh" && oneKey.sessionIds.includes(sessionId)
  ));
}

export function selectedSshOneKey(
  oneKeys: readonly OneKeySummary[],
  oneKeyId: string,
): OneKeySummary | null {
  return oneKeys.find((oneKey) => oneKey.id === oneKeyId) ?? null;
}
