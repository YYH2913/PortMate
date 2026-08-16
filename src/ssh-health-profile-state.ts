import type { SessionProfile } from "./types";

export function sshHealthProfileRequestKey(profile: SessionProfile) {
  if (profile.connection.kind !== "ssh" && profile.connection.kind !== "tmux") {
    return `${profile.id}:${profile.connection.kind}`;
  }
  return JSON.stringify({
    id: profile.id,
    connection: {
      ...profile.connection,
      trustedHostKeys: [],
      identityPolicy: {
        ...profile.connection.identityPolicy,
        lastSuccessful: null,
      },
    },
    terminal: profile.terminal,
  });
}

export function sshHealthProfileSnapshotMatches(
  runtimeProfile: SessionProfile,
  currentProfile: SessionProfile,
) {
  return sshHealthProfileRequestKey(runtimeProfile) === sshHealthProfileRequestKey(currentProfile);
}
