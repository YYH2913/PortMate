import { describe, expect, it } from "vitest";
import { identityStableKey, mergeAgentIdentities } from "./client-identity-state";
import type { IdentityRef } from "./types";

function identity(overrides: Partial<IdentityRef> = {}): IdentityRef {
  return {
    id: "agent-0",
    label: "Agent key",
    source: "agent",
    fingerprintSha256: "SHA256:agent-key",
    path: "agent-comment",
    secretRef: null,
    ...overrides,
  };
}

describe("client identity state", () => {
  it("keeps an existing agent identity id stable when discovery metadata is refreshed", () => {
    const existing = identity({ id: "jump-agent-key", label: "Old label", path: "old-comment" });
    const discovered = identity({ id: "agent-7", label: "New label", path: "new-comment " });

    const result = mergeAgentIdentities([existing], [discovered], () => "unused");

    expect(result).toEqual({
      identities: [{ ...discovered, id: "jump-agent-key", source: "agent", secretRef: null }],
      added: 0,
      updated: 1,
    });
  });

  it("assigns a unique id when a new agent identity collides with another key id", () => {
    const existing = identity({
      id: "agent:SHA256:agent-key",
      source: "public-key-only",
      fingerprintSha256: "SHA256:different-key",
    });

    const result = mergeAgentIdentities([existing], [identity()], () => "generated");

    expect(result.identities.map((item) => item.id)).toEqual([
      "agent:SHA256:agent-key:generated",
      "agent:SHA256:agent-key",
    ]);
    expect(result.added).toBe(1);
    expect(result.updated).toBe(0);
  });

  it("does not duplicate a key already provided by a non-agent source", () => {
    const existing = identity({ id: "vault-key", source: "profile-vault", secretRef: "keychain:key" });
    const result = mergeAgentIdentities([existing], [identity()], () => "unused");

    expect(result).toEqual({ identities: [existing], added: 0, updated: 0 });
  });

  it("uses exact agent comments when a fingerprint is unavailable", () => {
    expect(identityStableKey(identity({ fingerprintSha256: null, path: "key " })))
      .toBe("path:agent:key ");
  });
});
