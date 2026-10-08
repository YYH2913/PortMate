import { profileCredentialSecretRefs } from "./session-profile-helpers";
import type { SessionProfile } from "./types";
import { sshUsesNoneAuthenticationOnly } from "./ssh-connection-settings";

/** Leave vault-dependent targets pending; status-loaded is not unlocked. */
export function startupCredentialsReady(profile: SessionProfile, vaultUnlocked: boolean): boolean {
  if ((profile.connection.kind === "ssh" || profile.connection.kind === "tmux")
      && sshUsesNoneAuthenticationOnly(profile.connection)) {
    return vaultUnlocked || !profile.connection.proxy.passwordSecretRef?.startsWith("stronghold:");
  }
  return vaultUnlocked || ![...profileCredentialSecretRefs(profile)].some(ref => ref.startsWith("stronghold:"));
}
