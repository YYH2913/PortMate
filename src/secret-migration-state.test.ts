import { describe, expect, it } from "vitest";
import {
  buildProfileSecretMigrationRequest,
  canExecuteProfileSecretMigration,
  isProfileSecretMigrationRestartRequired,
  profileSecretMigrationErrorMessage,
  sameProfileSecretMigrationRequest,
  summarizeProfileSecretCleanup,
} from "./secret-migration-state";
import type { ProfileSecretMigrationItem, ProfileSecretMigrationPreview } from "./secret-migration-state";

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

describe("profile secret migration state", () => {
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
    expect(canExecuteProfileSecretMigration({ ...preview, eligibleSecretCount: 0 }, true, false)).toBe(false);
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
