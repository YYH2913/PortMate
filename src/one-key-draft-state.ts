import { selectionFromOneKeyIdentity } from "./one-key-identity-state";
import type { OneKeyIdentitySelection } from "./one-key-identity-state";
import type { OneKeyIdentitySummary, OneKeyKind, OneKeySummary } from "./types";

export interface OneKeyDraftState {
  id: string | null;
  label: string;
  kind: OneKeyKind;
  username: string;
  password: string;
  passphrase: string;
  clearPassword: boolean;
  clearPassphrase: boolean;
  hasPassword: boolean;
  hasPassphrase: boolean;
  currentIdentity: OneKeyIdentitySummary | null;
  identitySelection: OneKeyIdentitySelection | null;
  sessionIds: string[];
}

export function oneKeyDraftHasUnsavedChanges(
  draft: OneKeyDraftState,
  items: readonly OneKeySummary[],
): boolean {
  const saved = draft.id ? items.find((item) => item.id === draft.id) : null;
  if (!saved) {
    return Boolean(
      draft.id
      || draft.label
      || draft.kind !== "account"
      || draft.username
      || draft.password
      || draft.passphrase
      || draft.clearPassword
      || draft.clearPassphrase
      || draft.hasPassword
      || draft.hasPassphrase
      || draft.currentIdentity
      || draft.identitySelection
      || draft.sessionIds.length,
    );
  }

  return draft.label !== saved.label
    || draft.kind !== saved.kind
    || draft.username !== saved.username
    || Boolean(draft.password)
    || Boolean(draft.passphrase)
    || draft.clearPassword
    || draft.clearPassphrase
    || draft.hasPassword !== saved.hasPassword
    || draft.hasPassphrase !== saved.hasPassphrase
    || selectionKey(draft.identitySelection) !== selectionKey(selectionFromOneKeyIdentity(saved.identity))
    || !sameStringSet(draft.sessionIds, saved.sessionIds);
}

function selectionKey(selection: OneKeyIdentitySelection | null): string {
  return selection ? JSON.stringify([selection.sourceProfileId, selection.identityId]) : "";
}

function sameStringSet(left: readonly string[], right: readonly string[]): boolean {
  if (left.length !== right.length) return false;
  const expected = new Set(right);
  return expected.size === left.length && left.every((value) => expected.has(value));
}
