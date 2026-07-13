import type { SessionSummary } from "./types";

export type SecretStorage = "native" | "portable";
export type ProfileSecretCleanupStatus =
  | "deleted"
  | "retained-by-request"
  | "retained-shared"
  | "retained-in-use"
  | "failed";

export type ProfileSecretMigrationRequest = {
  targetStorage: SecretStorage;
  profileIds: string[];
  cleanupSource: boolean;
};

export type ProfileSecretMigrationPreview = {
  planToken: string;
  targetStorage: SecretStorage;
  selectedProfileCount: number;
  affectedProfileCount: number;
  eligibleReferenceCount: number;
  eligibleSecretCount: number;
  retainedSharedSecretCount: number;
  retainedInFlightSecretCount: number;
  alreadyTargetReferenceCount: number;
  excludedReservedReferenceCount: number;
};

export type ProfileSecretMigrationItem = {
  sourceRef: string;
  targetRef: string;
  referenceCount: number;
  remainingSourceReferences: number;
  cleanupStatus: ProfileSecretCleanupStatus;
  cleanupWarning?: string | null;
};

export type ProfileSecretMigrationResponse = {
  targetStorage: SecretStorage;
  selectedProfileCount: number;
  migratedProfileCount: number;
  migratedReferenceCount: number;
  migratedSecretCount: number;
  summaries: SessionSummary[];
  items: ProfileSecretMigrationItem[];
  warnings: string[];
  portableVaultRequiresReunlock: boolean;
};

export type MigrationCleanupSummary = Record<ProfileSecretCleanupStatus, number>;

export const PROFILE_SECRET_MIGRATION_RESTART_REQUIRED = "PORTMATE_MIGRATION_RESTART_REQUIRED:";

export function isProfileSecretMigrationRestartRequired(message: string): boolean {
  return message.includes(PROFILE_SECRET_MIGRATION_RESTART_REQUIRED);
}

export function profileSecretMigrationErrorMessage(message: string): string {
  return message.replace(PROFILE_SECRET_MIGRATION_RESTART_REQUIRED, "").trim();
}

export function buildProfileSecretMigrationRequest(
  targetStorage: SecretStorage,
  scopeProfileId: "all" | string,
  availableProfileIds: string[],
  cleanupSource: boolean,
): ProfileSecretMigrationRequest {
  const profileIds = scopeProfileId === "all"
    ? Array.from(new Set(availableProfileIds.map((id) => id.trim()).filter(Boolean)))
    : [scopeProfileId.trim()].filter(Boolean);
  if (!profileIds.length) {
    throw new Error("凭据迁移必须选择至少一个 SSH/Tmux Profile");
  }
  return { targetStorage, profileIds, cleanupSource };
}

export function sameProfileSecretMigrationRequest(
  left: ProfileSecretMigrationRequest,
  right: ProfileSecretMigrationRequest,
): boolean {
  return left.targetStorage === right.targetStorage
    && left.cleanupSource === right.cleanupSource
    && left.profileIds.length === right.profileIds.length
    && left.profileIds.every((profileId, index) => profileId === right.profileIds[index]);
}

export function canExecuteProfileSecretMigration(
  preview: ProfileSecretMigrationPreview | null,
  portableVaultUnlocked: boolean,
  busy: boolean,
): boolean {
  return portableVaultUnlocked && !busy && Boolean(preview?.eligibleSecretCount);
}

export function summarizeProfileSecretCleanup(
  items: ProfileSecretMigrationItem[],
): MigrationCleanupSummary {
  const summary: MigrationCleanupSummary = {
    deleted: 0,
    "retained-by-request": 0,
    "retained-shared": 0,
    "retained-in-use": 0,
    failed: 0,
  };
  for (const item of items) {
    summary[item.cleanupStatus] += 1;
  }
  return summary;
}
