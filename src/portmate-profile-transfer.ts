import { t } from "./i18n";
import type { ConnectionConfig, SessionProfile } from "./types";

export const PORTMATE_PROFILE_TRANSFER_FORMAT = "portmate-profile-transfer";
export const PORTMATE_PROFILE_TRANSFER_VERSION = 2;

const transferWarningMessages = {
  "credentials-omitted": "passwords-private-key-passphrases-and-proxy-credentials-are-not",
  "private-key-omitted": "profile-vault-private-keys-are-not-exported-re-import",
  "proxy-credentials-omitted": "proxy-credentials-are-not-exported-save-them-again-after",
} as const;
export type ProfileTransferWarning = {
  code: keyof typeof transferWarningMessages;
  profileName: string;
};

/** Localized presentation is never written into a transfer document or user label. */
export function profileTransferWarningLabel(warning: ProfileTransferWarning): string {
  return warning.profileName + t(transferWarningMessages[warning.code]);
}

function uniqueWarnings(warnings: ProfileTransferWarning[]): ProfileTransferWarning[] {
  return [...new Map(warnings.map(warning => [JSON.stringify([warning.code, warning.profileName]), warning])).values()];
}

export type PortMateProfileTransferDocument = {
  format: typeof PORTMATE_PROFILE_TRANSFER_FORMAT;
  version: typeof PORTMATE_PROFILE_TRANSFER_VERSION;
  exportedAt: string;
  profiles: SessionProfile[];
  warnings: ProfileTransferWarning[];
};

export type PortMateProfileTransferResult = {
  profiles: SessionProfile[];
  warnings: ProfileTransferWarning[];
};

export function createPortMateProfileTransfer(
  profiles: readonly SessionProfile[],
  exportedAt = new Date().toISOString(),
): PortMateProfileTransferDocument {
  const warnings: ProfileTransferWarning[] = [];
  const safeProfiles = profiles.map((profile) => sanitizeProfileForTransfer(profile, warnings));
  return {
    format: PORTMATE_PROFILE_TRANSFER_FORMAT,
    version: PORTMATE_PROFILE_TRANSFER_VERSION,
    exportedAt,
    profiles: safeProfiles,
    warnings: uniqueWarnings(warnings),
  };
}

export function serializePortMateProfileTransfer(
  profiles: readonly SessionProfile[],
  exportedAt = new Date().toISOString(),
): string {
  return JSON.stringify(createPortMateProfileTransfer(profiles, exportedAt), null, 2);
}

export function parsePortMateProfileTransfer(value: unknown): PortMateProfileTransferResult {
  const document = typeof value === "string" ? parseJson(value) : value;
  if (!document || typeof document !== "object" || Array.isArray(document)) {
    throw new Error(t("portmate-profile-file-must-be-a-json-object"));
  }
  const record = document as Record<string, unknown>;
  if (record.format !== PORTMATE_PROFILE_TRANSFER_FORMAT || record.version !== PORTMATE_PROFILE_TRANSFER_VERSION) {
    throw new Error(t("unsupported-portmate-profile-file-version"));
  }
  if (!Array.isArray(record.profiles) || !record.profiles.length) {
    throw new Error(t("portmate-profile-file-contains-no-profiles-to-import"));
  }
  const warnings: ProfileTransferWarning[] = Array.isArray(record.warnings)
    ? record.warnings.slice(0, 64).flatMap((value: unknown) => {
      if (!value || typeof value !== "object" || Array.isArray(value)) return [];
      const warning = value as Record<string, unknown>;
      if (typeof warning.code !== "string" || !Object.hasOwn(transferWarningMessages, warning.code)
        || typeof warning.profileName !== "string") return [];
      return [{ code: warning.code as ProfileTransferWarning["code"], profileName: warning.profileName }];
    })
    : [];
  const profiles = record.profiles.map((profile, index) => (
    sanitizeProfileForTransfer(normalizeImportedProfile(profile, index), warnings)
  ));
  return { profiles, warnings: uniqueWarnings(warnings) };
}

export function cloneImportedProfile(profile: SessionProfile, createId: () => string): SessionProfile {
  const imported = JSON.parse(JSON.stringify(profile)) as SessionProfile;
  if (imported.connection.kind === "ssh" || imported.connection.kind === "tmux") {
    const identityIds = new Map<string, string>();
    const identityRefs = imported.connection.identityRefs.map((identity) => {
      const nextId = "identity-" + createId();
      identityIds.set(identity.id, nextId);
      return { ...identity, id: nextId };
    });
    imported.connection = {
      ...imported.connection,
      identityRefs,
      jumps: imported.connection.jumps.map((jump) => ({
        ...jump,
        identityRef: jump.identityRef ? identityIds.get(jump.identityRef) ?? null : null,
      })),
    };
  }
  return { ...imported, id: createId(), name: imported.name || "Imported Profile" };
}

function sanitizeProfileForTransfer(profile: SessionProfile, warnings: ProfileTransferWarning[]): SessionProfile {
  const connection = sanitizeConnectionForTransfer(profile.connection, warnings, profile.name);
  return JSON.parse(JSON.stringify({ ...profile, connection })) as SessionProfile;
}

function sanitizeConnectionForTransfer(
  connection: ConnectionConfig,
  warnings: ProfileTransferWarning[],
  profileName: string,
): ConnectionConfig {
  if (connection.kind === "ssh" || connection.kind === "tmux") {
    if (connection.passwordSecretRef || connection.passphraseSecretRef || connection.proxy.passwordSecretRef
      || connection.identityRefs.some((identity) => Boolean(identity.secretRef))
      || connection.jumps.some((jump) => Boolean(jump.passwordSecretRef || jump.passphraseSecretRef))) {
      warnings.push({ code: "credentials-omitted", profileName });
    }
    const identities = connection.identityRefs.map((identity) => {
      if (identity.source === "profile-vault") {
        warnings.push({ code: "private-key-omitted", profileName });
        return {
          ...identity,
          source: "public-key-only" as const,
          path: null,
          secretRef: null,
        };
      }
      return {
        ...identity,
        path: identity.source === "system-file" ? identity.path : null,
        secretRef: null,
      };
    });
    const identityIds = new Set(identities.map((identity) => identity.id));
    return {
      ...connection,
      passwordSecretRef: null,
      passphraseSecretRef: null,
      proxy: { ...connection.proxy, passwordSecretRef: null },
      identityRefs: identities,
      jumps: connection.jumps.map((jump) => ({
        ...jump,
        passwordSecretRef: null,
        passphraseSecretRef: null,
        identityRef: jump.identityRef && identityIds.has(jump.identityRef) ? jump.identityRef : null,
      })),
    };
  }
  if (connection.kind === "tcp" || connection.kind === "telnet") {
    if (connection.proxy.passwordSecretRef) {
      warnings.push({ code: "proxy-credentials-omitted", profileName });
    }
    return { ...connection, proxy: { ...connection.proxy, passwordSecretRef: null } };
  }
  return connection;
}

function normalizeImportedProfile(value: unknown, index: number): SessionProfile {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(t("entry") + (index + 1) + t("is-not-a-profile-object"));
  }
  const profile = value as Partial<SessionProfile>;
  if (typeof profile.name !== "string" || !profile.name.trim()
    || typeof profile.kind !== "string" || !profile.connection
    || !profile.terminal || !profile.logging || !profile.transfer) {
    throw new Error(t("entry") + (index + 1) + t("is-missing-required-profile-settings"));
  }
  return JSON.parse(JSON.stringify(profile)) as SessionProfile;
}

function parseJson(value: string): unknown {
  try {
    return JSON.parse(value);
  } catch {
    throw new Error(t("profile-file-is-not-valid-json"));
  }
}
