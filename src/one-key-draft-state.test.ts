import { describe, expect, it } from "vitest";
import { oneKeyDraftHasUnsavedChanges } from "./one-key-draft-state";
import type { OneKeyDraftState } from "./one-key-draft-state";
import type { OneKeySummary } from "./types";

const saved: OneKeySummary = {
  id: "onekey-a",
  label: "Router login",
  kind: "ssh",
  username: "operator",
  hasPassword: true,
  hasPassphrase: false,
  identity: {
    sourceProfileId: "session-a",
    id: "identity-a",
    label: "Router key",
    source: "profile-vault",
  },
  sessionIds: ["session-a", "session-b"],
  createdAt: "2026-08-15T00:00:00Z",
  updatedAt: "2026-08-15T00:00:00Z",
};

function draft(): OneKeyDraftState {
  return {
    id: saved.id,
    label: saved.label,
    kind: saved.kind,
    username: saved.username,
    password: "",
    passphrase: "",
    clearPassword: false,
    clearPassphrase: false,
    hasPassword: saved.hasPassword,
    hasPassphrase: saved.hasPassphrase,
    currentIdentity: saved.identity ?? null,
    identitySelection: { sourceProfileId: "session-a", identityId: "identity-a" },
    sessionIds: [...saved.sessionIds].reverse(),
  };
}

describe("OneKey draft state", () => {
  it("treats an unchanged saved draft as clean without depending on binding order", () => {
    expect(oneKeyDraftHasUnsavedChanges(draft(), [saved])).toBe(false);
  });

  it("detects metadata, identity, binding, clear, and secret updates", () => {
    for (const changed of [
      { label: "Changed" },
      { username: "root" },
      { kind: "account" as const },
      { password: "new-secret" },
      { passphrase: "new-passphrase" },
      { clearPassword: true },
      { clearPassphrase: true },
      { identitySelection: null },
      { sessionIds: ["session-a"] },
    ]) {
      expect(oneKeyDraftHasUnsavedChanges({ ...draft(), ...changed }, [saved])).toBe(true);
    }
  });

  it("keeps a new empty draft clean but detects any entered credential data", () => {
    const empty: OneKeyDraftState = {
      ...draft(),
      id: null,
      label: "",
      kind: "account",
      username: "",
      hasPassword: false,
      currentIdentity: null,
      identitySelection: null,
      sessionIds: [],
    };
    expect(oneKeyDraftHasUnsavedChanges(empty, [])).toBe(false);
    expect(oneKeyDraftHasUnsavedChanges({ ...empty, kind: "ssh" }, [])).toBe(true);
    expect(oneKeyDraftHasUnsavedChanges({ ...empty, password: "new-secret" }, [])).toBe(true);
    expect(oneKeyDraftHasUnsavedChanges({ ...empty, sessionIds: ["session-a"] }, [])).toBe(true);
  });

  it("fails closed when the selected saved item disappeared", () => {
    expect(oneKeyDraftHasUnsavedChanges(draft(), [])).toBe(true);
  });
});
