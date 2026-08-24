import type { ConnectionConfig, SessionProfile } from "./types";

export const PORTMATE_PROFILE_TRANSFER_FORMAT = "portmate-profile-transfer";
export const PORTMATE_PROFILE_TRANSFER_VERSION = 1;

export type PortMateProfileTransferDocument = {
  format: typeof PORTMATE_PROFILE_TRANSFER_FORMAT;
  version: typeof PORTMATE_PROFILE_TRANSFER_VERSION;
  exportedAt: string;
  profiles: SessionProfile[];
  warnings: string[];
};

export type PortMateProfileTransferResult = {
  profiles: SessionProfile[];
  warnings: string[];
};

export function createPortMateProfileTransfer(
  profiles: readonly SessionProfile[],
  exportedAt = new Date().toISOString(),
): PortMateProfileTransferDocument {
  const warnings = new Set<string>();
  const safeProfiles = profiles.map((profile) => sanitizeProfileForTransfer(profile, warnings));
  return {
    format: PORTMATE_PROFILE_TRANSFER_FORMAT,
    version: PORTMATE_PROFILE_TRANSFER_VERSION,
    exportedAt,
    profiles: safeProfiles,
    warnings: [...warnings],
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
    throw new Error("PortMate Profile 文件必须是 JSON 对象");
  }
  const record = document as Record<string, unknown>;
  if (record.format !== PORTMATE_PROFILE_TRANSFER_FORMAT || record.version !== PORTMATE_PROFILE_TRANSFER_VERSION) {
    throw new Error("不支持的 PortMate Profile 文件版本");
  }
  if (!Array.isArray(record.profiles) || !record.profiles.length) {
    throw new Error("PortMate Profile 文件没有可导入的 Profile");
  }
  const warnings = Array.isArray(record.warnings)
    ? record.warnings.filter((warning): warning is string => typeof warning === "string").slice(0, 64)
    : [];
  const warningSet = new Set(warnings);
  const profiles = record.profiles.map((profile, index) => (
    sanitizeProfileForTransfer(normalizeImportedProfile(profile, index), warningSet)
  ));
  return { profiles, warnings: [...warningSet] };
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

function sanitizeProfileForTransfer(profile: SessionProfile, warnings: Set<string>): SessionProfile {
  const connection = sanitizeConnectionForTransfer(profile.connection, warnings, profile.name);
  return JSON.parse(JSON.stringify({ ...profile, connection })) as SessionProfile;
}

function sanitizeConnectionForTransfer(
  connection: ConnectionConfig,
  warnings: Set<string>,
  profileName: string,
): ConnectionConfig {
  if (connection.kind === "ssh" || connection.kind === "tmux") {
    if (connection.passwordSecretRef || connection.passphraseSecretRef || connection.proxy.passwordSecretRef
      || connection.identityRefs.some((identity) => Boolean(identity.secretRef))
      || connection.jumps.some((jump) => Boolean(jump.passwordSecretRef || jump.passphraseSecretRef))) {
      warnings.add(profileName + ": 密码、私钥口令和代理凭据不会导出，导入后需要重新保存");
    }
    const identities = connection.identityRefs.map((identity) => {
      if (identity.source === "profile-vault") {
        warnings.add(profileName + ": Profile Vault 私钥不会导出，导入后需要重新导入私钥");
        return {
          ...identity,
          source: "public-key-only" as const,
          path: null,
          secretRef: null,
          label: identity.label + "（需重新导入私钥）",
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
      warnings.add(profileName + ": 代理凭据不会导出，导入后需要重新保存");
    }
    return { ...connection, proxy: { ...connection.proxy, passwordSecretRef: null } };
  }
  return connection;
}

function normalizeImportedProfile(value: unknown, index: number): SessionProfile {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("第 " + (index + 1) + " 个 Profile 不是对象");
  }
  const profile = value as Partial<SessionProfile>;
  if (typeof profile.name !== "string" || !profile.name.trim()
    || typeof profile.kind !== "string" || !profile.connection
    || !profile.terminal || !profile.logging || !profile.transfer) {
    throw new Error("第 " + (index + 1) + " 个 Profile 缺少必要配置项");
  }
  return JSON.parse(JSON.stringify(profile)) as SessionProfile;
}

function parseJson(value: string): unknown {
  try {
    return JSON.parse(value);
  } catch {
    throw new Error("Profile 文件不是有效 JSON");
  }
}
