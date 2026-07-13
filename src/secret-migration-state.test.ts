import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeBackend } from "./api";
import {
  buildProfileSecretMigrationRecoveryRequest,
  buildProfileSecretMigrationRequest,
  canRecoverProfileSecretMigration,
  canExecuteProfileSecretMigration,
  getProfileSecretMigrationRecovery,
  isProfileSecretMigrationRestartRequired,
  profileSecretMigrationErrorMessage,
  profileSecretMigrationRecoveryBlockReason,
  recoverProfileSecretMigration,
  sameProfileSecretMigrationRequest,
  summarizeProfileSecretCleanup,
} from "./secret-migration-state";
import type {
  ProfileSecretMigrationItem,
  ProfileSecretMigrationPreview,
  ProfileSecretMigrationRecoveryResponse,
  ProfileSecretMigrationRecoverySummary,
} from "./secret-migration-state";

vi.mock("./api", () => ({ invokeBackend: vi.fn() }));

const preview: ProfileSecretMigrationPreview = {
  planToken: "plan-1",
  targetStorage: "portable",
  selectedProfileCount: 2,
  affectedProfileCount: 1,
  eligibleReferenceCount: 3,
  eligibleSecretCount: 2,
  retainedSharedSecretCount: 1,
  retainedInFlightSecretCount: 0,
  alreadyTargetReferenceCount: 1,
  excludedReservedReferenceCount: 0,
};

const pendingRecovery: ProfileSecretMigrationRecoverySummary = {
  migrationId: "89b35790-6b62-4ca1-a81f-678c30bf8428",
  state: "target-cleanup-pending",
  disposition: "not-committed",
  targetStorage: "portable",
  cleanupSource: true,
  profileCount: 2,
  secretCount: 3,
  requiresPortableVaultUnlock: true,
  canRecover: true,
  message: "Profile 仍使用迁移前引用，可以清理目标副本",
  createdAt: "2026-07-13T01:02:03Z",
  updatedAt: "2026-07-13T01:03:04Z",
};

describe("profile secret migration state", () => {
  beforeEach(() => {
    vi.mocked(invokeBackend).mockReset();
  });

  it("builds explicit all-profile and single-profile requests", () => {
    expect(buildProfileSecretMigrationRequest("portable", "all", [" a ", "b", "a"], true)).toEqual({
      targetStorage: "portable",
      profileIds: ["a", "b"],
      cleanupSource: true,
    });
    expect(buildProfileSecretMigrationRequest("native", " b ", ["a", "b"], false)).toEqual({
      targetStorage: "native",
      profileIds: ["b"],
      cleanupSource: false,
    });
  });

  it("rejects an empty migration scope", () => {
    expect(() => buildProfileSecretMigrationRequest("portable", "all", [], true)).toThrow("至少一个");
    expect(() => buildProfileSecretMigrationRequest("portable", " ", ["a"], true)).toThrow("至少一个");
  });

  it("invalidates a preview when direction, scope, or cleanup changes", () => {
    const request = buildProfileSecretMigrationRequest("portable", "all", ["a", "b"], true);
    expect(sameProfileSecretMigrationRequest(request, { ...request })).toBe(true);
    expect(sameProfileSecretMigrationRequest(request, { ...request, targetStorage: "native" })).toBe(false);
    expect(sameProfileSecretMigrationRequest(request, { ...request, profileIds: ["a"] })).toBe(false);
    expect(sameProfileSecretMigrationRequest(request, { ...request, cleanupSource: false })).toBe(false);
  });

  it("requires an unlocked vault, an idle operation, and eligible secrets", () => {
    expect(canExecuteProfileSecretMigration(preview, true, false)).toBe(true);
    expect(canExecuteProfileSecretMigration(preview, false, false)).toBe(false);
    expect(canExecuteProfileSecretMigration(preview, true, true)).toBe(false);
    expect(canExecuteProfileSecretMigration(preview, true, false, true)).toBe(false);
    expect(canExecuteProfileSecretMigration({ ...preview, eligibleSecretCount: 0 }, true, false)).toBe(false);
  });

  it("builds a normalized recovery request and rejects an empty journal ID", () => {
    expect(buildProfileSecretMigrationRecoveryRequest(`  ${pendingRecovery.migrationId}  `)).toEqual({
      migrationId: pendingRecovery.migrationId,
    });
    expect(() => buildProfileSecretMigrationRecoveryRequest("  ")).toThrow("ID 不能为空");
  });

  it("explains why a pending migration cannot be recovered", () => {
    expect(profileSecretMigrationRecoveryBlockReason(null, true, false)).toBe("no-pending-migration");
    expect(profileSecretMigrationRecoveryBlockReason(pendingRecovery, true, true)).toBe("operation-busy");
    expect(profileSecretMigrationRecoveryBlockReason(pendingRecovery, false, false)).toBe("portable-vault-locked");
    expect(profileSecretMigrationRecoveryBlockReason({
      ...pendingRecovery,
      state: "needs-resolution",
      disposition: "conflict",
      canRecover: false,
    }, true, false)).toBe("manual-resolution-required");
    expect(profileSecretMigrationRecoveryBlockReason({
      ...pendingRecovery,
      requiresPortableVaultUnlock: false,
      canRecover: false,
    }, true, false)).toBe("recovery-unavailable");
  });

  it("only enables automatic recovery when provider access and journal state allow it", () => {
    expect(canRecoverProfileSecretMigration({
      ...pendingRecovery,
      requiresPortableVaultUnlock: false,
    }, true, false)).toBe(true);
    expect(canRecoverProfileSecretMigration(pendingRecovery, true, false)).toBe(false);
    expect(canRecoverProfileSecretMigration(pendingRecovery, false, false)).toBe(false);
    expect(canRecoverProfileSecretMigration(null, true, false)).toBe(false);
  });

  it("loads the active recovery journal through the dedicated backend command", async () => {
    vi.mocked(invokeBackend).mockResolvedValueOnce(pendingRecovery);

    await expect(getProfileSecretMigrationRecovery()).resolves.toEqual(pendingRecovery);
    expect(invokeBackend).toHaveBeenCalledWith("get_profile_secret_migration_recovery", {});
  });

  it("submits the normalized migration ID to recovery", async () => {
    const response: ProfileSecretMigrationRecoveryResponse = {
      migrationId: pendingRecovery.migrationId,
      resolved: true,
      action: "target-cleanup-completed",
      warnings: [],
      pending: null,
    };
    vi.mocked(invokeBackend).mockResolvedValueOnce(response);

    await expect(recoverProfileSecretMigration(` ${pendingRecovery.migrationId} `)).resolves.toEqual(response);
    expect(invokeBackend).toHaveBeenCalledWith("recover_profile_secret_migration", {
      request: { migrationId: pendingRecovery.migrationId },
    });
  });

  it("recognizes restart-required migration failures by their stable code", () => {
    const error = "PORTMATE_MIGRATION_RESTART_REQUIRED: verify store";
    expect(isProfileSecretMigrationRestartRequired(error)).toBe(true);
    expect(profileSecretMigrationErrorMessage(error)).toBe("verify store");
    expect(isProfileSecretMigrationRestartRequired("凭据迁移预检已过期")).toBe(false);
  });

  it("summarizes every cleanup outcome without treating warnings as migration failure", () => {
    const item = (cleanupStatus: ProfileSecretMigrationItem["cleanupStatus"]): ProfileSecretMigrationItem => ({
      sourceRef: `keychain:${cleanupStatus}`,
      targetRef: `stronghold:${cleanupStatus}`,
      referenceCount: 1,
      remainingSourceReferences: cleanupStatus === "retained-shared" ? 1 : 0,
      cleanupStatus,
      cleanupWarning: cleanupStatus === "failed" ? "keyring locked" : null,
    });
    expect(summarizeProfileSecretCleanup([
      item("deleted"),
      item("deleted"),
      item("retained-by-request"),
      item("retained-shared"),
      item("retained-in-use"),
      item("failed"),
    ])).toEqual({
      deleted: 2,
      "retained-by-request": 1,
      "retained-shared": 1,
      "retained-in-use": 1,
      failed: 1,
    });
  });
});
