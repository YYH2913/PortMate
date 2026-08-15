import { describe, expect, it } from "vitest";
import {
  clientIdentityEditDraftHasUnsavedChanges,
  hostKeyEditDraftHasUnsavedChanges,
} from "./key-manager-draft-state";

describe("key manager draft state", () => {
  it("detects every mutable Host Key field", () => {
    const baseline = {
      profileId: "edge",
      alias: "router",
      host: "10.0.0.1",
      port: 22,
      scope: "profile" as const,
      label: "primary",
    };
    expect(hostKeyEditDraftHasUnsavedChanges({ ...baseline, baseline })).toBe(false);
    for (const patch of [
      { profileId: "" },
      { alias: "router-2" },
      { host: "10.0.0.2" },
      { port: 2222 },
      { scope: "project" as const },
      { label: "changed" },
    ]) {
      expect(hostKeyEditDraftHasUnsavedChanges({ ...baseline, ...patch, baseline })).toBe(true);
    }
  });

  it("detects Identity metadata and secret rotation actions without exposing secret values", () => {
    const saved = {
      id: "identity-1",
      label: "Router key",
      source: "profile-vault" as const,
      fingerprintSha256: "SHA256:saved",
      path: null,
      secretRef: "stronghold:identity-1",
    };
    const draft = {
      profileId: "edge",
      identityId: saved.id,
      label: saved.label,
      source: saved.source,
      fingerprintSha256: saved.fingerprintSha256,
      path: "",
      secretRef: saved.secretRef,
    };
    expect(clientIdentityEditDraftHasUnsavedChanges(draft, saved)).toBe(false);
    expect(clientIdentityEditDraftHasUnsavedChanges({ ...draft, label: "Changed" }, saved)).toBe(true);
    expect(clientIdentityEditDraftHasUnsavedChanges(draft, saved, "private-key-material")).toBe(true);
    expect(clientIdentityEditDraftHasUnsavedChanges(draft, saved, "", "passphrase")).toBe(true);
  });
});
