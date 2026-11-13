import { profileCredentialSecretRefs } from "./session-profile-helpers";
import type { SessionProfile } from "./types";
import { sshUsesNoneAuthenticationOnly } from "./ssh-connection-settings";

/** Leave vault-dependent targets pending; status-loaded is not unlocked. */
export function startupCredentialsReady(profile: SessionProfile, vaultUnlocked: boolean): boolean {
  // Privileged shells require a fresh, explicit desktop action, even after unlock.
  if (profile.connection.kind === "shell" && profile.connection.elevated) return false;
  if ((profile.connection.kind === "ssh" || profile.connection.kind === "tmux")
      && sshUsesNoneAuthenticationOnly(profile.connection)) {
    return vaultUnlocked || !profile.connection.proxy.passwordSecretRef?.startsWith("stronghold:");
  }
  return vaultUnlocked || ![...profileCredentialSecretRefs(profile)].some(ref => ref.startsWith("stronghold:"));
}
