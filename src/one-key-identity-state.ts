import type {
  IdentityRef,
  OneKeyIdentitySummary,
  OneKeyIdentityUpdate,
  OneKeyKind,
  SessionSummary,
} from "./types";

export interface OneKeyIdentityCandidate {
  sourceProfileId: string;
  sourceProfileName: string;
  identity: IdentityRef;
}

export interface OneKeyIdentitySelection {
  sourceProfileId: string;
  identityId: string;
}

export function oneKeyIdentityCandidates(
  sessions: readonly SessionSummary[],
  sessionIds: readonly string[],
): OneKeyIdentityCandidate[] {
  const bound = new Set(sessionIds);
  return sessions.flatMap((session) => {
    const { profile } = session;
    if (!bound.has(profile.id) || (profile.kind !== "ssh" && profile.kind !== "tmux")) {
      return [];
    }
    if (profile.connection.kind !== "ssh" && profile.connection.kind !== "tmux") {
      return [];
    }
    return profile.connection.identityRefs
      .filter((identity) => identity.source !== "public-key-only")
      .map((identity) => ({
        sourceProfileId: profile.id,
        sourceProfileName: profile.name,
        identity,
      }));
  });
}

export function oneKeyIdentitySelectionKey(selection: OneKeyIdentitySelection): string {
  return JSON.stringify([selection.sourceProfileId, selection.identityId]);
}

export function selectionFromOneKeyIdentity(
  identity: OneKeyIdentitySummary | null | undefined,
): OneKeyIdentitySelection | null {
  return identity
    ? { sourceProfileId: identity.sourceProfileId, identityId: identity.id }
    : null;
}

export function oneKeyIdentityUpdate(
  kind: OneKeyKind,
  current: OneKeyIdentitySummary | null | undefined,
  selection: OneKeyIdentitySelection | null,
): OneKeyIdentityUpdate {
  if (kind !== "ssh" || !selection) return { action: "clear" };
  if (
    current
    && current.sourceProfileId === selection.sourceProfileId
    && current.id === selection.identityId
  ) {
    return { action: "preserve" };
  }
  return {
    action: "set",
    sourceProfileId: selection.sourceProfileId,
    identityId: selection.identityId,
  };
}
