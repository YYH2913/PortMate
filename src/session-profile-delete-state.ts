import type { DeleteSessionProfileResponse, HostKeyStore, McpGrant, OneKeySummary, SessionSummary } from "./types";

export function deleteSessionProfileFromClientState(
  profileId: string,
  state: {
    sessions: SessionSummary[];
    oneKeys: OneKeySummary[];
    hostKeys: HostKeyStore;
    grants: McpGrant[];
  },
  revokedAt = new Date().toISOString(),
): DeleteSessionProfileResponse {
  return {
    deletedProfileId: profileId,
    sessions: state.sessions.filter((session) => session.profile.id !== profileId),
    oneKeys: state.oneKeys.map((oneKey) => {
      const sessionIds = oneKey.sessionIds.filter((sessionId) => sessionId !== profileId);
      const identity = oneKey.identity?.sourceProfileId === profileId ? null : oneKey.identity;
      const changed = sessionIds.length !== oneKey.sessionIds.length || identity !== oneKey.identity;
      return { ...oneKey, sessionIds, identity, updatedAt: changed ? revokedAt : oneKey.updatedAt };
    }),
    hostKeys: {
      keys: state.hostKeys.keys
        .filter((key) => !(key.scope === "profile" && key.profileId === profileId))
        .map((key) => key.profileId === profileId ? { ...key, profileId: null } : key),
    },
    grants: state.grants.map((grant) => {
      if (!grant.allowedSessions.length || !grant.allowedSessions.includes(profileId)) return grant;
      const allowedSessions = grant.allowedSessions.filter((sessionId) => sessionId !== profileId);
      return {
        ...grant,
        allowedSessions,
        revokedAt: allowedSessions.length ? grant.revokedAt : grant.revokedAt ?? revokedAt,
      };
    }),
  };
}
