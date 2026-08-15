import type { IdentityRef, TrustedHostKey } from "./types";

export type HostKeyEditFields = {
  profileId: string;
  alias: string;
  host: string;
  port: number;
  scope: TrustedHostKey["scope"];
  label: string;
};

export type HostKeyEditDraftState = HostKeyEditFields & {
  baseline: HostKeyEditFields;
};

export type ClientIdentityEditDraftState = {
  profileId: string;
  identityId: string;
  label: string;
  source: IdentityRef["source"];
  fingerprintSha256: string;
  path: string;
  secretRef: string;
};

export function hostKeyEditDraftHasUnsavedChanges(draft: HostKeyEditDraftState | null): boolean {
  if (!draft) return false;
  return draft.profileId !== draft.baseline.profileId
    || draft.alias !== draft.baseline.alias
    || draft.host !== draft.baseline.host
    || draft.port !== draft.baseline.port
    || draft.scope !== draft.baseline.scope
    || draft.label !== draft.baseline.label;
}

export function clientIdentityEditDraftHasUnsavedChanges(
  draft: ClientIdentityEditDraftState | null,
  saved: IdentityRef | null,
  privateKeyDraft = "",
  passphraseDraft = "",
): boolean {
  if (privateKeyDraft || passphraseDraft) return true;
  if (!draft) return false;
  if (!saved) return true;
  return draft.identityId !== saved.id
    || draft.label !== saved.label
    || draft.source !== saved.source
    || draft.fingerprintSha256 !== (saved.fingerprintSha256 ?? "")
    || draft.path !== (saved.path ?? "")
    || draft.secretRef !== (saved.secretRef ?? "");
}
