import { invokeBackend } from "./api";
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
  migrationId: string | null;
  recoveryPending: boolean;
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

export type ProfileSecretMigrationJournalState =
  | "target-write-pending"
  | "targets-verified"
  | "profiles-committed"
  | "source-cleanup-pending"
  | "target-cleanup-pending"
  | "needs-resolution";

export type ProfileSecretMigrationRecoveryDisposition =
  | "not-committed"
  | "committed"
  | "conflict";

export type ProfileSecretMigrationRecoverySummary = {
  migrationId: string;
  state: ProfileSecretMigrationJournalState;
  disposition: ProfileSecretMigrationRecoveryDisposition;
  targetStorage: SecretStorage;
  cleanupSource: boolean;
  profileCount: number;
  secretCount: number;
  requiresPortableVaultUnlock: boolean;
  canRecover: boolean;
  message: string;
  createdAt: string;
  updatedAt: string;
};

export type ProfileSecretMigrationRecoveryRequest = {
  migrationId: string;
};

export type ProfileSecretMigrationRecoveryResponse = {
  migrationId: string;
  resolved: boolean;
  action: string;
  warnings: string[];
  pending: ProfileSecretMigrationRecoverySummary | null;
};

export type ProfileSecretMigrationDiagnosticExportResult = {
  path: string;
  checksumPath: string;
  sha256: string;
  size: number;
  migrationId: string | null;
  journalValid: boolean;
  warnings: string[];
};

export type ProfileSecretMigrationRecoveryBlockReason =
  | "no-pending-migration"
  | "operation-busy"
  | "manual-resolution-required"
  | "portable-vault-locked"
  | "recovery-unavailable";

export type MigrationCleanupSummary = Record<ProfileSecretCleanupStatus, number>;

export const PROFILE_SECRET_MIGRATION_RESTART_REQUIRED = "PORTMATE_MIGRATION_RESTART_REQUIRED:";

export function isProfileSecretMigrationRestartRequired(message: string): boolean {
  return message.includes(PROFILE_SECRET_MIGRATION_RESTART_REQUIRED);
}

export function profileSecretMigrationErrorMessage(message: string): string {
  return message.replace(PROFILE_SECRET_MIGRATION_RESTART_REQUIRED, "").trim();
}

export function buildProfileSecretMigrationRequest(
  scopeProfileId: "all" | string,
  availableProfileIds: string[],
  cleanupSource: boolean,
): ProfileSecretMigrationRequest {
  const profileIds = scopeProfileId === "all"
    ? Array.from(new Set(availableProfileIds.map((id) => id.trim()).filter(Boolean)))
    : [scopeProfileId.trim()].filter(Boolean);
  if (!profileIds.length) {
    throw new Error("凭据迁移必须选择至少一个支持凭据的 Profile");
  }
  return { targetStorage: "portable", profileIds, cleanupSource };
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
  recoveryPending = false,
): boolean {
  return portableVaultUnlocked && !busy && !recoveryPending && Boolean(preview?.eligibleSecretCount);
}

export function buildProfileSecretMigrationRecoveryRequest(
  migrationId: string,
): ProfileSecretMigrationRecoveryRequest {
  const normalizedMigrationId = migrationId.trim();
  if (!normalizedMigrationId) {
    throw new Error("凭据迁移恢复记录 ID 不能为空");
  }
  return { migrationId: normalizedMigrationId };
}

export function profileSecretMigrationRecoveryBlockReason(
  pending: ProfileSecretMigrationRecoverySummary | null,
  portableVaultUnlocked: boolean,
  busy: boolean,
): ProfileSecretMigrationRecoveryBlockReason | null {
  if (!pending) return "no-pending-migration";
  if (busy) return "operation-busy";
  if (pending.disposition === "conflict" || pending.state === "needs-resolution") {
    return "manual-resolution-required";
  }
  if (pending.requiresPortableVaultUnlock || !portableVaultUnlocked) {
    return "portable-vault-locked";
  }
  if (!pending.canRecover) return "recovery-unavailable";
  return null;
}

export function canRecoverProfileSecretMigration(
  pending: ProfileSecretMigrationRecoverySummary | null,
  portableVaultUnlocked: boolean,
  busy: boolean,
): boolean {
  return profileSecretMigrationRecoveryBlockReason(pending, portableVaultUnlocked, busy) === null;
}

export function getProfileSecretMigrationRecovery(): Promise<ProfileSecretMigrationRecoverySummary | null> {
  return invokeBackend<ProfileSecretMigrationRecoverySummary | null>(
    "get_profile_secret_migration_recovery",
    {},
  );
}

export function recoverProfileSecretMigration(
  migrationId: string,
): Promise<ProfileSecretMigrationRecoveryResponse> {
  const request = buildProfileSecretMigrationRecoveryRequest(migrationId);
  return invokeBackend<ProfileSecretMigrationRecoveryResponse>(
    "recover_profile_secret_migration",
    { request },
  );
}

export function exportProfileSecretMigrationDiagnostics(): Promise<ProfileSecretMigrationDiagnosticExportResult> {
  return invokeBackend<ProfileSecretMigrationDiagnosticExportResult>(
    "export_profile_secret_migration_diagnostics",
    {},
  );
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
