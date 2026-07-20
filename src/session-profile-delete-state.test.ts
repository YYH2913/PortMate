import { describe, expect, it } from "vitest";
import { deleteSessionProfileFromClientState } from "./session-profile-delete-state";
import type { HostKeyStore, McpGrant, OneKeySummary, SessionSummary } from "./types";

const sessions = [
  { profile: { id: "deleted" } },
  { profile: { id: "retained" } },
] as SessionSummary[];

describe("deleteSessionProfileFromClientState", () => {
  it("removes the profile and detaches OneKey bindings and source identities", () => {
    const oneKeys = [{
      id: "one-key",
      sessionIds: ["deleted", "retained"],
      identity: { sourceProfileId: "deleted" },
    }] as OneKeySummary[];

    const result = deleteSessionProfileFromClientState("deleted", {
      sessions,
      oneKeys,
      hostKeys: { keys: [] },
      grants: [],
    });

    expect(result.sessions.map((session) => session.profile.id)).toEqual(["retained"]);
    expect(result.oneKeys[0].sessionIds).toEqual(["retained"]);
    expect(result.oneKeys[0].identity).toBeNull();
    expect(result.oneKeys[0].updatedAt).toBeTruthy();
  });

  it("revokes the last scoped grant without changing global grants", () => {
    const grants = [
      { clientId: "only", allowedSessions: ["deleted"], revokedAt: null },
      { clientId: "mixed", allowedSessions: ["deleted", "retained"], revokedAt: null },
      { clientId: "global", allowedSessions: [], revokedAt: null },
    ] as McpGrant[];

    const result = deleteSessionProfileFromClientState("deleted", {
      sessions,
      oneKeys: [],
      hostKeys: { keys: [] },
      grants,
    }, "2026-07-20T00:00:00.000Z");

    expect(result.grants.find((grant) => grant.clientId === "only")).toMatchObject({
      allowedSessions: [],
      revokedAt: "2026-07-20T00:00:00.000Z",
    });
    expect(result.grants.find((grant) => grant.clientId === "mixed")).toMatchObject({
      allowedSessions: ["retained"],
      revokedAt: null,
    });
    expect(result.grants.find((grant) => grant.clientId === "global")).toMatchObject({
      allowedSessions: [],
      revokedAt: null,
    });
  });

  it("removes profile trust while retaining project trust without a stale owner", () => {
    const hostKeys = { keys: [
      { id: "profile", profileId: "deleted", scope: "profile" },
      { id: "project", profileId: "deleted", scope: "project" },
      { id: "other", profileId: "retained", scope: "profile" },
    ] } as HostKeyStore;

    const result = deleteSessionProfileFromClientState("deleted", {
      sessions,
      oneKeys: [],
      hostKeys,
      grants: [],
    });

    expect(result.hostKeys.keys.map((key) => key.id)).toEqual(["project", "other"]);
    expect(result.hostKeys.keys[0].profileId).toBeNull();
  });
});
