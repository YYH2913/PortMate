import type { IdentityRef } from "./types";

export type AgentIdentityMergeResult = {
  identities: IdentityRef[];
  added: number;
  updated: number;
};

export function identityStableKey(identity: IdentityRef): string {
  if (identity.fingerprintSha256) return `fingerprint:${identity.fingerprintSha256}`;
  if (identity.secretRef) return `secret:${identity.secretRef}`;
  if (identity.path) return `path:${identity.source}:${identity.path}`;
  return `id:${identity.id}`;
}

export function mergeAgentIdentities(
  existingIdentities: readonly IdentityRef[],
  discoveredIdentities: readonly IdentityRef[],
  createIdSuffix: () => string,
): AgentIdentityMergeResult {
  let added = 0;
  let updated = 0;
  let identities = [...existingIdentities];
  const usedIds = new Set(identities.map((identity) => identity.id));

  for (const discovered of discoveredIdentities) {
    const preferredId = discovered.fingerprintSha256
      ? `agent:${discovered.fingerprintSha256}`
      : discovered.id;
    const incoming: IdentityRef = {
      ...discovered,
      id: preferredId,
      source: "agent",
      path: discovered.path ?? null,
      secretRef: null,
    };
    const stableKey = identityStableKey(incoming);
    const existingIndex = identities.findIndex((identity) => identityStableKey(identity) === stableKey);
    if (existingIndex >= 0) {
      const current = identities[existingIndex];
      if (current.source !== "agent") continue;
      identities[existingIndex] = { ...current, ...incoming, id: current.id };
      updated += 1;
      continue;
    }

    const id = uniqueIdentityId(preferredId, usedIds, createIdSuffix);
    usedIds.add(id);
    identities = [{ ...incoming, id }, ...identities];
    added += 1;
  }

  return { identities, added, updated };
}

function uniqueIdentityId(
  preferredId: string,
  usedIds: ReadonlySet<string>,
  createIdSuffix: () => string,
): string {
  if (!usedIds.has(preferredId)) return preferredId;
  const base = `${preferredId}:${createIdSuffix()}`;
  if (!usedIds.has(base)) return base;
  for (let index = 2; ; index += 1) {
    const candidate = `${base}:${index}`;
    if (!usedIds.has(candidate)) return candidate;
  }
}
