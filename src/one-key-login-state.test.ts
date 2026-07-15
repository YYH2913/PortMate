import { describe, expect, it } from "vitest";
import { selectedSshOneKey, sshOneKeysForSession } from "./one-key-login-state";
import type { OneKeySummary } from "./types";

const items: OneKeySummary[] = [
  {
    id: "onekey:ssh-bound",
    label: "SSH bound",
    kind: "ssh",
    username: "operator",
    hasPassword: true,
    hasPassphrase: false,
    identity: {
      sourceProfileId: "session-a",
      id: "identity-a",
      label: "Lab key",
      source: "profile-vault",
    },
    sessionIds: ["session-a"],
    createdAt: "2026-07-15T00:00:00Z",
    updatedAt: "2026-07-15T00:00:00Z",
  },
  {
    id: "onekey:ssh-other",
    label: "SSH other",
    kind: "ssh",
    username: "admin",
    hasPassword: false,
    hasPassphrase: true,
    identity: null,
    sessionIds: ["session-b"],
    createdAt: "2026-07-15T00:00:00Z",
    updatedAt: "2026-07-15T00:00:00Z",
  },
  {
    id: "onekey:account",
    label: "Account",
    kind: "account",
    username: "user",
    hasPassword: true,
    hasPassphrase: false,
    identity: null,
    sessionIds: ["session-a"],
    createdAt: "2026-07-15T00:00:00Z",
    updatedAt: "2026-07-15T00:00:00Z",
  },
];

describe("OneKey SSH login selection", () => {
  it("offers only SSH OneKeys bound to the requested session", () => {
    expect(sshOneKeysForSession(items, "session-a").map((item) => item.id))
      .toEqual(["onekey:ssh-bound"]);
    expect(sshOneKeysForSession(items, "missing")).toEqual([]);
  });

  it("resolves a selected item without inventing a stale fallback", () => {
    const compatible = sshOneKeysForSession(items, "session-a");
    expect(selectedSshOneKey(compatible, "onekey:ssh-bound")?.username).toBe("operator");
    expect(selectedSshOneKey(compatible, "onekey:ssh-other")).toBeNull();
  });
});
