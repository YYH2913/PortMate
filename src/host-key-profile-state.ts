import type { SessionProfile } from "./types";

export function hostKeyProfileRequestKey(profile: SessionProfile) {
  if (profile.connection.kind !== "ssh" && profile.connection.kind !== "tmux") {
    return `${profile.id}:${profile.connection.kind}`;
  }
  return JSON.stringify({
    id: profile.id,
    kind: profile.kind,
    connection: { ...profile.connection, trustedHostKeys: [] },
  });
}

export function hostKeyProfileSnapshotMatches(
  scannedProfile: SessionProfile,
  currentProfile: SessionProfile,
) {
  return hostKeyProfileRequestKey(scannedProfile) === hostKeyProfileRequestKey(currentProfile);
}
