import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  AlertCircle,
  ArrowRightLeft,
  ArrowUp,
  CheckCircle2,
  Copy,
  FileText,
  KeyRound,
  Lock,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  Unlock,
  UserPlus,
  X,
} from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { identityStableKey, mergeAgentIdentities } from "./client-identity-state";
import { formatBytes } from "./display-formatters";
import { KeyedRequestGate } from "./keyed-request-gate";
import {
  buildProfileSecretMigrationRequest,
  canExecuteProfileSecretMigration,
  canRecoverProfileSecretMigration,
  exportProfileSecretMigrationDiagnostics,
  getProfileSecretMigrationRecovery,
  isProfileSecretMigrationRestartRequired,
  profileSecretMigrationErrorMessage,
  recoverProfileSecretMigration,
  sameProfileSecretMigrationRequest,
  summarizeProfileSecretCleanup,
} from "./secret-migration-state";
import type {
  ProfileSecretMigrationDiagnosticExportResult,
  ProfileSecretMigrationPreview,
  ProfileSecretMigrationRecoverySummary,
  ProfileSecretMigrationRequest,
  ProfileSecretMigrationResponse,
  SecretStorage,
} from "./secret-migration-state";
import type {
  ConnectionConfig,
  HostKeyScanResult,
  HostKeyStore,
  IdentityRef,
  SessionProfile,
  SessionSummary,
  TrustedHostKey,
} from "./types";

const migrationRecoveryStateLabels: Record<ProfileSecretMigrationRecoverySummary["state"], string> = {
  "target-write-pending": "目标写入待核对",
  "targets-verified": "目标已验证",
  "profiles-committed": "Profile 已提交",
  "source-cleanup-pending": "源清理待完成",
  "target-cleanup-pending": "目标回滚待完成",
  "needs-resolution": "需要人工核对",
};

const migrationRecoveryDispositionLabels: Record<ProfileSecretMigrationRecoverySummary["disposition"], string> = {
  "not-committed": "原引用生效",
  committed: "目标引用生效",
  conflict: "投影冲突",
};

type HostKeyEditDraft = {
  keyId: string;
  expectedKey: TrustedHostKey;
  profileId: string;
  alias: string;
  host: string;
  port: number;
  scope: TrustedHostKey["scope"];
  label: string;
};

type ClientIdentityGroupBy = "profile" | "source";

type ClientIdentityItem = {
  selectionId: string;
  profileId: string;
  profileName: string;
  identity: IdentityRef;
  jumpInUse: boolean;
};

type ClientIdentityEditDraft = {
  profileId: string;
  identityId: string;
  label: string;
  source: IdentityRef["source"];
  fingerprintSha256: string;
  path: string;
  secretRef: string;
};

type ClientIdentityMutationResponse = {
  summary: SessionSummary;
  oldSecretDeleted: boolean;
  oldSecretShared: boolean;
  cleanupWarning?: string | null;
};

type PortableVaultStatus = {
  exists: boolean;
  unlocked: boolean;
  path: string;
};

type SecretStorageChoice = "auto" | "native" | "portable";

export default function KeyManagerDialog({
  hostKeys,
  sessions,
  prepareProfile,
  onHostKeyMutationStart,
  onChange,
  onHostKeyMutationFinish,
  onProfileMutationStart,
  onProfileChange,
  onProfileMutationCurrent,
  onProfileMutationFinish,
  credentialOperationBusy,
  credentialSyncRevision,
  onCredentialOperationStart,
  onCredentialOperationFinish,
  onClose,
}: {
  hostKeys: HostKeyStore;
  sessions: SessionSummary[];
  prepareProfile: (profile: SessionProfile) => SessionProfile;
  onHostKeyMutationStart: () => number;
  onChange: (store: HostKeyStore, token: number) => boolean;
  onHostKeyMutationFinish: (token: number) => void;
  onProfileMutationStart: (profileId: string) => number;
  onProfileChange: (summary: SessionSummary, token: number, activateWorkspace?: boolean) => boolean;
  onProfileMutationCurrent: (profileId: string, token: number) => boolean;
  onProfileMutationFinish: (profileId: string, token: number, committed: boolean) => void;
  credentialOperationBusy: boolean;
  credentialSyncRevision: number;
  onCredentialOperationStart: () => number | null;
  onCredentialOperationFinish: (token: number) => void;
  onClose: () => void;
}) {
  const sshSessions = sessions.filter((session) => isSshLikeProfile(session.profile));
  const credentialSessions = sessions.filter((session) => (
    session.profile.connection.kind === "ssh"
    || session.profile.connection.kind === "tmux"
    || session.profile.connection.kind === "tcp"
    || session.profile.connection.kind === "telnet"
  ));
  const [profileId, setProfileId] = useState(sshSessions[0]?.profile.id ?? "");
  const [knownHostsText, setKnownHostsText] = useState("");
  const [exportText, setExportText] = useState("");
  const [agentKeys, setAgentKeys] = useState<IdentityRef[]>([]);
  const [clientKeyQuery, setClientKeyQuery] = useState("");
  const [clientKeySourceFilter, setClientKeySourceFilter] = useState<IdentityRef["source"] | "all">("all");
  const [clientKeyProfileFilter, setClientKeyProfileFilter] = useState("all");
  const [clientKeyGroupBy, setClientKeyGroupBy] = useState<ClientIdentityGroupBy>("profile");
  const [selectedClientKeyIds, setSelectedClientKeyIds] = useState<string[]>([]);
  const [editingClientKeyId, setEditingClientKeyId] = useState("");
  const [clientKeyEditDraft, setClientKeyEditDraft] = useState<ClientIdentityEditDraft | null>(null);
  const clientKeyEditExpectedIdentityRef = useRef<IdentityRef | null>(null);
  const [clientKeyPrivateKey, setClientKeyPrivateKey] = useState("");
  const [clientKeyPassphrase, setClientKeyPassphrase] = useState("");
  const [clientKeyStorage, setClientKeyStorage] = useState<SecretStorageChoice>("auto");
  const [clientKeyMutationBusy, setClientKeyMutationBusy] = useState(false);
  const [selectedAgentKeyIds, setSelectedAgentKeyIds] = useState<string[]>([]);
  const [privateKeyLabel, setPrivateKeyLabel] = useState("profile key");
  const [privateKeyText, setPrivateKeyText] = useState("");
  const [privateKeyStorage, setPrivateKeyStorage] = useState<SecretStorageChoice>("auto");
  const [portableVault, setPortableVault] = useState<PortableVaultStatus | null>(null);
  const [portableVaultPassword, setPortableVaultPassword] = useState("");
  const [portableVaultCurrentPassword, setPortableVaultCurrentPassword] = useState("");
  const [portableVaultNewPassword, setPortableVaultNewPassword] = useState("");
  const [portableVaultConfirmPassword, setPortableVaultConfirmPassword] = useState("");
  const [portableVaultFeedback, setPortableVaultFeedback] = useState<{ kind: "error" | "status"; message: string } | null>(null);
  const [portableVaultBusy, setPortableVaultBusy] = useState(false);
  const [migrationTarget, setMigrationTarget] = useState<SecretStorage>("portable");
  const [migrationScopeProfileId, setMigrationScopeProfileId] = useState<"all" | string>("all");
  const [migrationCleanupSource, setMigrationCleanupSource] = useState(true);
  const [migrationBusy, setMigrationBusy] = useState<"preview" | "migrate" | null>(null);
  const [migrationPreviewState, setMigrationPreviewState] = useState<{ request: ProfileSecretMigrationRequest; preview: ProfileSecretMigrationPreview } | null>(null);
  const [migrationResult, setMigrationResult] = useState<ProfileSecretMigrationResponse | null>(null);
  const [migrationError, setMigrationError] = useState("");
  const [migrationRequiresRestart, setMigrationRequiresRestart] = useState(false);
  const [migrationRecovery, setMigrationRecovery] = useState<ProfileSecretMigrationRecoverySummary | null>(null);
  const [migrationRecoveryBusy, setMigrationRecoveryBusy] = useState(false);
  const [migrationRecoveryChecking, setMigrationRecoveryChecking] = useState(isBackendAvailable);
  const [migrationRecoveryStatusError, setMigrationRecoveryStatusError] = useState("");
  const [migrationRecoveryError, setMigrationRecoveryError] = useState("");
  const [migrationRecoveryWarnings, setMigrationRecoveryWarnings] = useState<string[]>([]);
  const [migrationDiagnosticBusy, setMigrationDiagnosticBusy] = useState(false);
  const [migrationDiagnosticResult, setMigrationDiagnosticResult] = useState<ProfileSecretMigrationDiagnosticExportResult | null>(null);
  const [keyScopeFilter, setKeyScopeFilter] = useState<TrustedHostKey["scope"] | "all">("all");
  const [keyProfileFilter, setKeyProfileFilter] = useState("all");
  const [selectedHostKeyIds, setSelectedHostKeyIds] = useState<string[]>([]);
  const [editingKeyId, setEditingKeyId] = useState("");
  const [editDraft, setEditDraft] = useState<HostKeyEditDraft | null>(null);
  const [hostKeyScan, setHostKeyScan] = useState<HostKeyScanResult | null>(null);
  const [hostKeyScanBusy, setHostKeyScanBusy] = useState(false);
  const [hostKeyScanError, setHostKeyScanError] = useState("");
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const refreshGate = useRef(new KeyedRequestGate<"agent-keys" | "vault" | "recovery" | "host-scan">());
  const mountedRef = useRef(true);
  const credentialSyncRevisionRef = useRef(credentialSyncRevision);

  const selectedProfile = sshSessions.find((session) => session.profile.id === profileId)?.profile ?? null;
  const editingKey = hostKeys.keys.find((key) => key.id === editingKeyId) ?? null;
  const visibleHostKeys = hostKeys.keys.filter((key) => (
    (keyScopeFilter === "all" || key.scope === keyScopeFilter)
    && (keyProfileFilter === "all" || key.profileId === keyProfileFilter)
  ));
  const selectedVisibleHostKeys = visibleHostKeys.filter((key) => selectedHostKeyIds.includes(key.id));
  const clientIdentityItems = sshSessions.flatMap((session) => {
    const profile = session.profile;
    if (!isSshLikeProfile(profile)) return [];
    return profile.connection.identityRefs.map((identity, index): ClientIdentityItem => ({
      selectionId: clientIdentitySelectionId(profile.id, identity, index),
      profileId: profile.id,
      profileName: profile.name,
      identity,
      jumpInUse: profile.connection.jumps.some((jump) => jump.identityRef === identity.id),
    }));
  });
  const normalizedClientKeyQuery = clientKeyQuery.trim().toLowerCase();
  const visibleClientIdentityItems = clientIdentityItems.filter((item) => (
    (clientKeySourceFilter === "all" || item.identity.source === clientKeySourceFilter)
    && (clientKeyProfileFilter === "all" || item.profileId === clientKeyProfileFilter)
    && (!normalizedClientKeyQuery || `${item.identity.label} ${item.identity.fingerprintSha256 ?? ""} ${item.identity.path ?? ""} ${item.profileName}`.toLowerCase().includes(normalizedClientKeyQuery))
  ));
  const clientIdentityGroups = groupClientIdentityItems(visibleClientIdentityItems, clientKeyGroupBy);
  const selectedClientIdentityItems = clientIdentityItems.filter((item) => selectedClientKeyIds.includes(item.selectionId));
  const editingClientIdentityItem = clientIdentityItems.find((item) => item.selectionId === editingClientKeyId) ?? null;
  const editingClientSecretUsage = editingClientIdentityItem?.identity.secretRef
    ? clientIdentityItems.filter((item) => item.identity.secretRef === editingClientIdentityItem.identity.secretRef).length
    : 0;
  const selectedAgentKeys = agentKeys.filter((identity) => selectedAgentKeyIds.includes(identityStableKey(identity)));
  const vaultOperationBusy = credentialOperationBusy || portableVaultBusy || migrationBusy !== null || migrationRecoveryBusy || migrationDiagnosticBusy;
  const credentialMutationsFrozen = migrationRecoveryChecking || Boolean(migrationRecovery) || Boolean(migrationRecoveryStatusError);
  const credentialMutationControlsDisabled = vaultOperationBusy || credentialMutationsFrozen;
  const migrationControlsDisabled = credentialMutationControlsDisabled || migrationRequiresRestart;
  const migrationCleanupSummary = migrationResult ? summarizeProfileSecretCleanup(migrationResult.items) : null;

  useEffect(() => {
    mountedRef.current = true;
    void refreshAgentKeys();
    void refreshPortableVault();
    void refreshMigrationRecovery();
    return () => {
      mountedRef.current = false;
      refreshGate.current.invalidateAll();
    };
  }, []);

  useEffect(() => {
    if (credentialSyncRevisionRef.current === credentialSyncRevision) return;
    credentialSyncRevisionRef.current = credentialSyncRevision;
    void refreshPortableVault(true);
    void refreshMigrationRecovery(true, true);
  }, [credentialSyncRevision]);

  useEffect(() => {
    if (!sshSessions.some((session) => session.profile.id === profileId)) {
      setProfileId(sshSessions[0]?.profile.id ?? "");
    }
    if (migrationScopeProfileId !== "all" && !credentialSessions.some((session) => session.profile.id === migrationScopeProfileId)) {
      setMigrationScopeProfileId("all");
      setMigrationPreviewState(null);
    }
  }, [profileId, sessions, migrationScopeProfileId]);

  useEffect(() => {
    refreshGate.current.invalidate("host-scan");
    setHostKeyScan(null);
    setHostKeyScanError("");
    setHostKeyScanBusy(false);
  }, [profileId]);

  useEffect(() => {
    if (editingKeyId && !hostKeys.keys.some((key) => key.id === editingKeyId)) {
      setEditingKeyId("");
      setEditDraft(null);
    }
    setSelectedHostKeyIds((current) => current.filter((keyId) => hostKeys.keys.some((key) => key.id === keyId)));
  }, [editingKeyId, hostKeys.keys]);

  useEffect(() => {
    const validClientIds = new Set(clientIdentityItems.map((item) => item.selectionId));
    setSelectedClientKeyIds((current) => current.filter((id) => validClientIds.has(id)));
    if (editingClientKeyId && !validClientIds.has(editingClientKeyId)) {
      setEditingClientKeyId("");
      setClientKeyEditDraft(null);
      clientKeyEditExpectedIdentityRef.current = null;
      setClientKeyPrivateKey("");
      setClientKeyPassphrase("");
    }
  }, [sessions]);

  useEffect(() => {
    const validAgentIds = new Set(agentKeys.map(identityStableKey));
    setSelectedAgentKeyIds((current) => current.filter((id) => validAgentIds.has(id)));
  }, [agentKeys]);

  useEffect(() => {
    if (portableVault && !portableVault.unlocked) {
      clearPortableVaultRotation();
      setMigrationPreviewState(null);
    }
  }, [portableVault?.unlocked]);

  function clearPortableVaultRotation() {
    setPortableVaultCurrentPassword("");
    setPortableVaultNewPassword("");
    setPortableVaultConfirmPassword("");
  }

  function invalidateMigrationState() {
    setMigrationPreviewState(null);
    setMigrationResult(null);
    setMigrationError("");
  }

  function currentMigrationRequest(): ProfileSecretMigrationRequest {
    return buildProfileSecretMigrationRequest(
      migrationTarget,
      migrationScopeProfileId,
      credentialSessions.map((session) => session.profile.id),
      migrationCleanupSource,
    );
  }

  async function refreshAgentKeys() {
    if (!isBackendAvailable()) return;
    const token = refreshGate.current.begin("agent-keys");
    if (token === null) return;
    try {
      const next = await invokeBackend<IdentityRef[]>("list_ssh_agent_identities", {});
      if (refreshGate.current.isCurrent("agent-keys", token)) setAgentKeys(next);
    } catch {
      // Keep the last confirmed agent list when enumeration is temporarily unavailable.
    } finally {
      refreshGate.current.finish("agent-keys", token);
    }
  }

  async function refreshPortableVault(replace = false) {
    if (!isBackendAvailable()) return;
    if (replace) refreshGate.current.invalidate("vault");
    const token = refreshGate.current.begin("vault");
    if (token === null) return;
    try {
      const next = await invokeBackend<PortableVaultStatus>("portable_vault_status", {});
      if (refreshGate.current.isCurrent("vault", token)) setPortableVault(next);
    } catch {
      // Preserve the last confirmed vault state on a transient status failure.
    } finally {
      refreshGate.current.finish("vault", token);
    }
  }

  async function refreshMigrationRecovery(clearError = true, replace = false) {
    if (!isBackendAvailable()) {
      setMigrationRecoveryChecking(false);
      return;
    }
    if (replace) refreshGate.current.invalidate("recovery");
    const token = refreshGate.current.begin("recovery");
    if (token === null) return;
    setMigrationRecoveryChecking(true);
    try {
      const pending = await getProfileSecretMigrationRecovery();
      if (!refreshGate.current.isCurrent("recovery", token)) return;
      setMigrationRecovery(pending);
      setMigrationRecoveryStatusError("");
      if (pending) setMigrationPreviewState(null);
      if (clearError) setMigrationRecoveryError("");
    } catch (error) {
      if (refreshGate.current.isCurrent("recovery", token)) {
        setMigrationRecoveryStatusError(formatError(error));
        if (clearError) setMigrationRecoveryError("");
      }
    } finally {
      if (refreshGate.current.finish("recovery", token)) setMigrationRecoveryChecking(false);
    }
  }

  async function unlockPortableVault() {
    if (!portableVaultPassword) return;
    const operationToken = onCredentialOperationStart();
    if (operationToken === null) return;
    refreshGate.current.invalidate("vault");
    const existed = portableVault?.exists ?? false;
    setPortableVaultBusy(true);
    setPortableVaultFeedback(null);
    setError("");
    setStatus("");
    try {
      const next = await invokeBackend<PortableVaultStatus>("unlock_portable_vault", {
        request: { password: portableVaultPassword },
      });
      if (!mountedRef.current) return;
      setPortableVault(next);
      setPortableVaultPassword("");
      setPortableVaultFeedback({ kind: "status", message: existed ? "Portable vault 已解锁" : "Portable vault 已创建并解锁" });
    } catch (error) {
      if (mountedRef.current) {
        setPortableVaultPassword("");
        setPortableVaultFeedback({ kind: "error", message: formatError(error) });
      }
    } finally {
      onCredentialOperationFinish(operationToken);
      if (mountedRef.current) setPortableVaultBusy(false);
    }
  }

  async function lockPortableVault() {
    const operationToken = onCredentialOperationStart();
    if (operationToken === null) return;
    refreshGate.current.invalidate("vault");
    setPortableVaultBusy(true);
    clearPortableVaultRotation();
    setPortableVaultFeedback(null);
    setError("");
    setStatus("");
    try {
      const next = await invokeBackend<PortableVaultStatus>("lock_portable_vault", {});
      if (!mountedRef.current) return;
      setPortableVault(next);
      clearPortableVaultRotation();
      setPortableVaultFeedback({ kind: "status", message: "Portable vault 已锁定" });
    } catch (error) {
      if (mountedRef.current) setPortableVaultFeedback({ kind: "error", message: formatError(error) });
    } finally {
      onCredentialOperationFinish(operationToken);
      if (mountedRef.current) setPortableVaultBusy(false);
    }
  }

  async function rotatePortableVaultPassword() {
    setPortableVaultFeedback(null);
    setError("");
    setStatus("");
    if (!portableVaultCurrentPassword || !portableVaultNewPassword || !portableVaultConfirmPassword) {
      setPortableVaultFeedback({ kind: "error", message: "请填写当前密码、新密码和确认密码" });
      return;
    }
    if (Array.from(portableVaultNewPassword).length < 8) {
      setPortableVaultFeedback({ kind: "error", message: "Portable vault 新主密码至少需要 8 个字符" });
      return;
    }
    if (portableVaultNewPassword !== portableVaultConfirmPassword) {
      setPortableVaultFeedback({ kind: "error", message: "Portable vault 两次输入的新主密码不一致" });
      return;
    }
    if (portableVaultCurrentPassword === portableVaultNewPassword) {
      setPortableVaultFeedback({ kind: "error", message: "Portable vault 新主密码必须与当前密码不同" });
      return;
    }
    const operationToken = onCredentialOperationStart();
    if (operationToken === null) return;
    refreshGate.current.invalidate("vault");
    setPortableVaultBusy(true);
    try {
      const next = await invokeBackend<PortableVaultStatus>("rotate_portable_vault_password", {
        request: {
          currentPassword: portableVaultCurrentPassword,
          newPassword: portableVaultNewPassword,
        },
      });
      if (!mountedRef.current) return;
      setPortableVault(next);
      clearPortableVaultRotation();
      setPortableVaultFeedback({ kind: "status", message: "Portable vault 主密码已更换" });
    } catch (error) {
      if (mountedRef.current) {
        clearPortableVaultRotation();
        setPortableVaultFeedback({ kind: "error", message: formatError(error) });
      }
    } finally {
      onCredentialOperationFinish(operationToken);
      if (mountedRef.current) setPortableVaultBusy(false);
    }
  }

  async function previewProfileSecretMigration() {
    if (!portableVault?.unlocked || migrationRequiresRestart || migrationRecovery || !isBackendAvailable()) return;
    setMigrationBusy("preview");
    setMigrationError("");
    setMigrationResult(null);
    try {
      const request = currentMigrationRequest();
      const preview = await invokeBackend<ProfileSecretMigrationPreview>("preview_profile_secret_migration", { request });
      if (!mountedRef.current) return;
      setMigrationPreviewState({ request, preview });
      setMigrationRequiresRestart(false);
    } catch (error) {
      if (mountedRef.current) {
        const message = formatError(error);
        setMigrationPreviewState(null);
        setMigrationRequiresRestart(isProfileSecretMigrationRestartRequired(message));
        setMigrationError(profileSecretMigrationErrorMessage(message));
      }
    } finally {
      if (mountedRef.current) setMigrationBusy(null);
    }
  }

  async function migrateProfileSecrets() {
    if (!portableVault?.unlocked || migrationRequiresRestart || migrationRecovery || !migrationPreviewState || !isBackendAvailable()) return;
    let request: ProfileSecretMigrationRequest;
    try {
      request = currentMigrationRequest();
    } catch (error) {
      setMigrationError(formatError(error));
      return;
    }
    if (!sameProfileSecretMigrationRequest(request, migrationPreviewState.request)) {
      setMigrationPreviewState(null);
      setMigrationError("迁移设置已变化，请重新预检");
      return;
    }
    if (!canExecuteProfileSecretMigration(migrationPreviewState.preview, true, false, Boolean(migrationRecovery))) return;
    const credentialOperationToken = onCredentialOperationStart();
    if (credentialOperationToken === null) return;
    const mutationTokens = new Map(request.profileIds.map((targetProfileId) => [
      targetProfileId,
      onProfileMutationStart(targetProfileId),
    ]));
    let backendSucceeded = false;
    setMigrationBusy("migrate");
    setMigrationError("");
    try {
      const result = await invokeBackend<ProfileSecretMigrationResponse>("migrate_profile_secrets", {
        request,
        expectedPlanToken: migrationPreviewState.preview.planToken,
      });
      backendSucceeded = true;
      const accepted = result.summaries.map((summary) => {
        const mutationToken = mutationTokens.get(summary.profile.id);
        return mutationToken !== undefined
          && onProfileChange(summary, mutationToken, false);
      }).every(Boolean);
      if (accepted && mountedRef.current) {
        setMigrationPreviewState(null);
        setMigrationResult(result);
        setMigrationRequiresRestart(false);
        setEditingClientKeyId("");
        setClientKeyEditDraft(null);
        setClientKeyPrivateKey("");
        setClientKeyPassphrase("");
      }
      if (result.portableVaultRequiresReunlock) {
        try {
          const next = await invokeBackend<PortableVaultStatus>("lock_portable_vault", {});
          if (mountedRef.current) {
            setPortableVault(next);
            setPortableVaultFeedback({ kind: "status", message: `已迁移 ${result.migratedSecretCount} 个 Secret；请重新解锁 Stronghold` });
          }
        } catch (lockError) {
          if (mountedRef.current) setPortableVaultFeedback({ kind: "error", message: `凭据迁移已提交，但 Stronghold 自动锁定失败: ${formatError(lockError)}` });
        }
      }
    } catch (error) {
      const message = formatError(error);
      if (mountedRef.current) {
        setMigrationPreviewState(null);
        setMigrationRequiresRestart(isProfileSecretMigrationRestartRequired(message));
        setMigrationError(profileSecretMigrationErrorMessage(message));
      }
    } finally {
      for (const [targetProfileId, mutationToken] of mutationTokens) {
        onProfileMutationFinish(targetProfileId, mutationToken, backendSucceeded);
      }
      onCredentialOperationFinish(credentialOperationToken);
      if (mountedRef.current) setMigrationBusy(null);
    }
  }

  async function recoverPendingProfileSecretMigration() {
    if (!migrationRecovery || migrationRecoveryChecking || migrationRecoveryStatusError || migrationRequiresRestart || !isBackendAvailable()) return;
    if (!canRecoverProfileSecretMigration(migrationRecovery, portableVault?.unlocked ?? false, vaultOperationBusy)) return;
    const operationToken = onCredentialOperationStart();
    if (operationToken === null) return;
    setMigrationRecoveryBusy(true);
    setMigrationRecoveryError("");
    setMigrationRecoveryWarnings([]);
    try {
      const result = await recoverProfileSecretMigration(migrationRecovery.migrationId);
      if (mountedRef.current) {
        setMigrationRecovery(result.pending);
        setMigrationRecoveryWarnings(
          result.warnings.length || !result.resolved
            ? result.warnings
            : ["恢复记录已核对并清除"],
        );
        setMigrationRequiresRestart(false);
        setMigrationPreviewState(null);
        if (result.resolved) setMigrationRecoveryError("");
      }
      if (result.pending?.requiresPortableVaultUnlock && portableVault?.unlocked) {
        try {
          const next = await invokeBackend<PortableVaultStatus>("lock_portable_vault", {});
          if (mountedRef.current) {
            setPortableVault(next);
            setPortableVaultFeedback({ kind: "status", message: "恢复 checkpoint 待核对；Stronghold 已锁定，请重新解锁" });
          }
        } catch (lockError) {
          if (mountedRef.current) setPortableVaultFeedback({ kind: "error", message: `恢复记录已保留，但 Stronghold 自动锁定失败: ${formatError(lockError)}` });
        }
      }
    } catch (error) {
      if (mountedRef.current) {
        const message = formatError(error);
        setMigrationRequiresRestart(isProfileSecretMigrationRestartRequired(message));
        setMigrationRecoveryError(profileSecretMigrationErrorMessage(message));
      }
    } finally {
      onCredentialOperationFinish(operationToken);
      if (mountedRef.current) setMigrationRecoveryBusy(false);
    }
  }

  async function exportPendingProfileSecretMigrationDiagnostics() {
    if (migrationRecoveryChecking || (!migrationRecovery && !migrationRecoveryStatusError) || !isBackendAvailable()) return;
    setMigrationDiagnosticBusy(true);
    setMigrationDiagnosticResult(null);
    setMigrationRecoveryError("");
    try {
      const result = await exportProfileSecretMigrationDiagnostics();
      if (!mountedRef.current) return;
      setMigrationDiagnosticResult(result);
      setMigrationRecoveryWarnings(result.warnings);
    } catch (error) {
      if (mountedRef.current) setMigrationRecoveryError(formatError(error));
    } finally {
      if (mountedRef.current) setMigrationDiagnosticBusy(false);
    }
  }

  async function importKnownHostsText() {
    if (!profileId || !knownHostsText.trim()) return;
    const mutationToken = onHostKeyMutationStart();
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("import_known_hosts", {
        request: { profileId, contents: knownHostsText },
      });
      const accepted = onChange(nextStore, mutationToken);
      if (!accepted || !mountedRef.current) return;
      setKnownHostsText("");
      setStatus("known_hosts 已导入到选中的 Profile scope");
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
    }
  }

  async function exportKnownHostsText() {
    setError("");
    setStatus("");
    try {
      setExportText(await invokeBackend<string>("export_known_hosts", {}));
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function scanSelectedProfileHostKey() {
    if (!selectedProfile) return;
    const token = refreshGate.current.begin("host-scan");
    if (token === null) return;
    setHostKeyScanBusy(true);
    setHostKeyScanError("");
    try {
      const scan = await invokeBackend<HostKeyScanResult>("scan_ssh_host_key", {
        profile: prepareProfile(selectedProfile),
        password: null,
        passphrase: null,
      });
      if (!refreshGate.current.isCurrent("host-scan", token)) return;
      setHostKeyScan(scan);
    } catch (error) {
      if (refreshGate.current.isCurrent("host-scan", token)) {
        setHostKeyScan(null);
        setHostKeyScanError(formatError(error));
      }
    } finally {
      const current = refreshGate.current.isCurrent("host-scan", token);
      refreshGate.current.finish("host-scan", token);
      if (current) setHostKeyScanBusy(false);
    }
  }

  async function trustHostKeyScan(decision: "append-to-profile" | "append-to-project" | "replace-for-profile") {
    if (!selectedProfile || !hostKeyScan) return;
    const mutationToken = onHostKeyMutationStart();
    refreshGate.current.invalidate("host-scan");
    setHostKeyScanBusy(true);
    setHostKeyScanError("");
    setError("");
    setStatus("");
    try {
      await invokeBackend<TrustedHostKey | null>("trust_scanned_host_key", {
        request: {
          profile: prepareProfile(selectedProfile),
          observation: hostKeyScan.observation,
          decision,
        },
      });
      const nextStore = await invokeBackend<HostKeyStore>("list_host_keys", {});
      const accepted = onChange(nextStore, mutationToken);
      if (!accepted || !mountedRef.current) return;
      setHostKeyScan(null);
      setStatus(decision === "replace-for-profile" ? "Profile Host key 已替换" : "扫描到的 Host key 已加入信任 Store");
    } catch (error) {
      if (mountedRef.current) setHostKeyScanError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
      if (mountedRef.current) setHostKeyScanBusy(false);
    }
  }

  async function deleteKey(keyId: string) {
    const mutationToken = onHostKeyMutationStart();
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("delete_host_key", { keyId });
      const accepted = onChange(nextStore, mutationToken);
      if (!accepted || !mountedRef.current) return;
      if (editingKeyId === keyId) {
        setEditingKeyId("");
        setEditDraft(null);
      }
      setStatus("Host key 已删除");
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
    }
  }

  async function deleteSelectedHostKeys() {
    if (!selectedHostKeyIds.length) return;
    const mutationToken = onHostKeyMutationStart();
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("delete_host_keys", { keyIds: selectedHostKeyIds });
      const accepted = onChange(nextStore, mutationToken);
      if (!accepted || !mountedRef.current) return;
      setSelectedHostKeyIds([]);
      setEditingKeyId("");
      setEditDraft(null);
      setStatus(`已删除 ${selectedHostKeyIds.length} 个 host key`);
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
    }
  }

  function toggleHostKeySelection(keyId: string, selected: boolean) {
    setSelectedHostKeyIds((current) => (
      selected
        ? Array.from(new Set([...current, keyId]))
        : current.filter((id) => id !== keyId)
    ));
  }

  function selectVisibleHostKeys() {
    setSelectedHostKeyIds((current) => Array.from(new Set([...current, ...visibleHostKeys.map((key) => key.id)])));
  }

  function startEditKey(key: TrustedHostKey) {
    setEditingKeyId(key.id);
    setEditDraft({
      keyId: key.id,
      expectedKey: { ...key },
      profileId: key.profileId ?? profileId,
      alias: key.alias,
      host: key.host,
      port: key.port,
      scope: key.scope,
      label: key.label ?? "",
    });
    setError("");
    setStatus("");
  }

  async function saveEditedHostKey() {
    if (!editDraft) return;
    const mutationToken = onHostKeyMutationStart();
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("update_host_key", {
        request: {
          keyId: editDraft.keyId,
          expectedKey: editDraft.expectedKey,
          profileId: editDraft.profileId || null,
          alias: editDraft.alias,
          host: editDraft.host,
          port: editDraft.port,
          scope: editDraft.scope,
          label: editDraft.label || null,
        },
      });
      const accepted = onChange(nextStore, mutationToken);
      if (!accepted || !mountedRef.current) return;
      setEditingKeyId("");
      setEditDraft(null);
      setStatus("Host key 已更新");
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
    }
  }

  async function saveProfileFromManager(
    profile: SessionProfile,
    expectedProfile: SessionProfile,
    message: string,
    existingMutationToken?: number,
  ): Promise<{ persisted: boolean; accepted: boolean }> {
    const mutationToken = existingMutationToken ?? onProfileMutationStart(profile.id);
    let backendSucceeded = false;
    setError("");
    setStatus("");
    try {
      const saved = await invokeBackend<SessionSummary>("save_session_profile", { profile: prepareProfile(profile), expectedProfile });
      backendSucceeded = true;
      const accepted = onProfileChange(saved, mutationToken);
      if (accepted && mountedRef.current) setStatus(message);
      return { persisted: true, accepted };
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
      return { persisted: false, accepted: false };
    } finally {
      onProfileMutationFinish(profile.id, mutationToken, backendSucceeded);
    }
  }

  async function readPrivateKeyFile(file: File | null) {
    if (!file) return;
    setError("");
    setStatus("");
    try {
      setPrivateKeyText(await file.text());
      if (!privateKeyLabel.trim()) {
        setPrivateKeyLabel(file.name.replace(/\.(pem|key|txt)$/i, "") || "profile key");
      }
      setStatus(`已读取 ${file.name}`);
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function importPrivateKeyToProfile() {
    if (!selectedProfile || !isSshLikeProfile(selectedProfile)) return;
    const profile = selectedProfile;
    const privateKey = privateKeyText.trim();
    if (!privateKey) return;
    if (!privateKey.includes("PRIVATE KEY")) {
      setError("私钥内容看起来不是 OpenSSH/PEM private key");
      return;
    }
    const mutationToken = onProfileMutationStart(profile.id);
    let mutationDelegated = false;
    let newSecretRef: string | null = null;
    setClientKeyMutationBusy(true);
    setError("");
    setStatus("");
    try {
      const label = privateKeyLabel.trim() || "profile key";
      const response = await invokeBackend<{ secretRef: string }>("save_secret", {
        request: { secretRef: null, secret: privateKeyText, storage: privateKeyStorage === "auto" ? null : privateKeyStorage },
      });
      newSecretRef = response.secretRef;
      if (!onProfileMutationCurrent(profile.id, mutationToken)) {
        try {
          await invokeBackend("delete_secret", { secretRef: newSecretRef });
        } catch {
          // A superseded import must not continue into a stale Profile save.
        }
        newSecretRef = null;
        return;
      }
      const identityRef: IdentityRef = {
        id: `vault:${typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : Date.now()}`,
        label,
        source: "profile-vault",
        fingerprintSha256: null,
        path: null,
        secretRef: response.secretRef,
      };
      mutationDelegated = true;
      const saveResult = await saveProfileFromManager({
        ...profile,
        connection: {
          ...profile.connection,
          identityRefs: [identityRef, ...profile.connection.identityRefs],
          identityPolicy: {
            ...profile.connection.identityPolicy,
            identitiesOnly: true,
          },
        },
      }, profile, `已导入私钥到 ${profile.name}`, mutationToken);
      if (saveResult.persisted) {
        newSecretRef = null;
        if (saveResult.accepted && mountedRef.current) setPrivateKeyText("");
      } else {
        try {
          await invokeBackend("delete_secret", { secretRef: response.secretRef });
        } catch {
          // Preserve the original profile-save error if best-effort cleanup also fails.
        }
        newSecretRef = null;
      }
    } catch (error) {
      if (newSecretRef && !mutationDelegated) {
        try {
          await invokeBackend("delete_secret", { secretRef: newSecretRef });
        } catch {
          // Preserve the original import error if best-effort cleanup also fails.
        }
      }
      if (mountedRef.current) setError(formatError(error));
    } finally {
      if (!mutationDelegated) onProfileMutationFinish(profile.id, mutationToken, true);
      if (mountedRef.current) setClientKeyMutationBusy(false);
    }
  }

  async function copyHostKeysToProfile(keys: TrustedHostKey[]) {
    if (!selectedProfile || !isSshLikeProfile(selectedProfile)) return;
    const currentKeys = selectedProfile.connection.trustedHostKeys;
    const copiedKeys: TrustedHostKey[] = [];
    for (const key of keys) {
      const copied: TrustedHostKey = {
        ...key,
        id: `${selectedProfile.id}:${key.alias}:${key.port}:${key.algorithm}:${key.fingerprintSha256}`,
        profileId: selectedProfile.id,
        scope: "profile",
        label: key.label ?? `copied from ${key.scope}`,
        lastSeen: new Date().toISOString(),
      };
      const exists = [...currentKeys, ...copiedKeys].some((item) => (
        item.algorithm === copied.algorithm
        && item.fingerprintSha256 === copied.fingerprintSha256
        && item.alias === copied.alias
        && item.port === copied.port
      ));
      if (!exists) {
        copiedKeys.push(copied);
      }
    }
    if (!copiedKeys.length) {
      setStatus("选中的 Profile 已包含这些 host key");
      return;
    }
    await saveProfileFromManager({
      ...selectedProfile,
      connection: {
        ...selectedProfile.connection,
        trustedHostKeys: [...copiedKeys, ...currentKeys],
      },
    }, selectedProfile, `已复制 ${copiedKeys.length} 个 host key 到 ${selectedProfile.name}`);
  }

  async function copyHostKeyToProfile(key: TrustedHostKey) {
    await copyHostKeysToProfile([key]);
  }

  async function copySelectedHostKeysToProfile() {
    await copyHostKeysToProfile(selectedVisibleHostKeys);
  }

  async function copyAgentIdentitiesToProfile(identitiesToCopy: IdentityRef[]) {
    if (!selectedProfile || !isSshLikeProfile(selectedProfile)) return;
    const { identities: nextIdentities, added, updated } = mergeAgentIdentities(
      selectedProfile.connection.identityRefs,
      identitiesToCopy,
      createLocalId,
    );
    if (!added && !updated) return;
    const saveResult = await saveProfileFromManager({
      ...selectedProfile,
      connection: {
        ...selectedProfile.connection,
        identityRefs: nextIdentities,
        agentPolicy: {
          ...selectedProfile.connection.agentPolicy,
          enabled: true,
          offerMode: selectedProfile.connection.agentPolicy.offerMode === "disabled" ? "after-profile-keys" : selectedProfile.connection.agentPolicy.offerMode,
        },
      },
    }, selectedProfile, `Agent keys: ${added} added, ${updated} updated · ${selectedProfile.name}`);
    if (saveResult.accepted && mountedRef.current) setSelectedAgentKeyIds([]);
  }

  async function copyAgentIdentityToProfile(identity: IdentityRef) {
    await copyAgentIdentitiesToProfile([identity]);
  }

  async function copyClientIdentitiesToProfile(items: ClientIdentityItem[]) {
    if (!selectedProfile || !isSshLikeProfile(selectedProfile) || !items.length) return;
    const currentIdentities = selectedProfile.connection.identityRefs;
    const nextIdentities = [...currentIdentities];
    let copied = 0;
    let copiedAgent = false;
    let copiedProfileKey = false;
    for (const item of items) {
      const identity = item.identity;
      const stableKey = identityStableKey(identity);
      if (nextIdentities.some((existing) => identityStableKey(existing) === stableKey)) continue;
      let id = identity.id;
      if (nextIdentities.some((existing) => existing.id === id)) {
        id = `${identity.id}:${createLocalId()}`;
      }
      nextIdentities.unshift({ ...identity, id });
      copied += 1;
      copiedAgent ||= identity.source === "agent";
      copiedProfileKey ||= identity.source !== "agent";
    }
    if (!copied) {
      setStatus(`${selectedProfile.name} 已包含选中的 client keys`);
      return;
    }
    const saveResult = await saveProfileFromManager({
      ...selectedProfile,
      connection: {
        ...selectedProfile.connection,
        identityRefs: nextIdentities,
        identityPolicy: {
          ...selectedProfile.connection.identityPolicy,
          identitiesOnly: copiedProfileKey ? true : selectedProfile.connection.identityPolicy.identitiesOnly,
        },
        agentPolicy: copiedAgent ? {
          ...selectedProfile.connection.agentPolicy,
          enabled: true,
          offerMode: selectedProfile.connection.agentPolicy.offerMode === "disabled" ? "after-profile-keys" : selectedProfile.connection.agentPolicy.offerMode,
        } : selectedProfile.connection.agentPolicy,
      },
    }, selectedProfile, `已复制 ${copied} 个 client key 到 ${selectedProfile.name}`);
    if (saveResult.accepted && mountedRef.current) setSelectedClientKeyIds([]);
  }

  async function moveSelectedClientIdentitiesFirst() {
    if (!selectedClientIdentityItems.length) return;
    const selectedIds = new Set(selectedClientIdentityItems.map((item) => item.selectionId));
    const targets = sshSessions.flatMap((session) => {
      const profile = session.profile;
      if (!isSshLikeProfile(profile)) return [];
      const selected = profile.connection.identityRefs.filter((identity, index) => (
        selectedIds.has(clientIdentitySelectionId(profile.id, identity, index))
      ));
      if (!selected.length) return [];
      const remaining = profile.connection.identityRefs.filter((identity, index) => (
        !selectedIds.has(clientIdentitySelectionId(profile.id, identity, index))
      ));
      return [{ profile, identityRefs: [...selected, ...remaining] }];
    });
    const mutationTokens = new Map(targets.map(({ profile }) => [
      profile.id,
      onProfileMutationStart(profile.id),
    ]));
    const completedProfiles = new Set<string>();
    let superseded = false;
    setError("");
    setStatus("");
    let updatedProfiles = 0;
    try {
      for (const { profile, identityRefs } of targets) {
        const mutationToken = mutationTokens.get(profile.id)!;
        if (!onProfileMutationCurrent(profile.id, mutationToken)) {
          superseded = true;
          continue;
        }
        const saved = await invokeBackend<SessionSummary>("save_session_profile", {
          profile: prepareProfile({
            ...profile,
            connection: { ...profile.connection, identityRefs },
          }),
          expectedProfile: profile,
        });
        completedProfiles.add(profile.id);
        if (!onProfileChange(saved, mutationToken)) superseded = true;
        updatedProfiles += 1;
      }
      if (mountedRef.current && !superseded) {
        setSelectedClientKeyIds([]);
        setStatus(`已在 ${updatedProfiles} 个 Profile 中置顶所选 client keys`);
      }
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      for (const [targetProfileId, mutationToken] of mutationTokens) {
        onProfileMutationFinish(
          targetProfileId,
          mutationToken,
          completedProfiles.has(targetProfileId),
        );
      }
    }
  }

  async function removeSelectedClientIdentities() {
    if (!selectedClientIdentityItems.length) return;
    const targets = sshSessions.flatMap((session) => {
      const profile = session.profile;
      if (!isSshLikeProfile(profile)) return [];
      const selected = selectedClientIdentityItems.filter((item) => item.profileId === profile.id);
      const removableItems = selected.filter((item) => !item.jumpInUse);
      return removableItems.length ? [{ profile, removableItems }] : [];
    });
    const mutationTokens = new Map(targets.map(({ profile }) => [
      profile.id,
      onProfileMutationStart(profile.id),
    ]));
    const completedProfiles = new Set<string>();
    let superseded = false;
    setError("");
    setStatus("");
    let removed = 0;
    const skipped = selectedClientIdentityItems.filter((item) => item.jumpInUse).length;
    try {
      for (const { profile, removableItems } of targets) {
        const mutationToken = mutationTokens.get(profile.id)!;
        let profileCompleted = true;
        for (const item of removableItems) {
          if (!onProfileMutationCurrent(profile.id, mutationToken)) {
            superseded = true;
            profileCompleted = false;
            break;
          }
          const response = await invokeBackend<ClientIdentityMutationResponse>("delete_client_identity", {
            request: { profileId: profile.id, identityId: item.identity.id, deleteSecret: false },
          });
          if (!onProfileChange(response.summary, mutationToken)) superseded = true;
          removed += 1;
        }
        if (profileCompleted) completedProfiles.add(profile.id);
      }
      if (mountedRef.current && !superseded) {
        setSelectedClientKeyIds([]);
        setStatus(`已移除 ${removed} 个 client key 引用${skipped ? `，跳过 ${skipped} 个 Jump Host 使用中的 key` : ""}`);
      }
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      for (const [targetProfileId, mutationToken] of mutationTokens) {
        onProfileMutationFinish(
          targetProfileId,
          mutationToken,
          completedProfiles.has(targetProfileId),
        );
      }
    }
  }

  function toggleClientIdentitySelection(selectionId: string, selected: boolean) {
    setSelectedClientKeyIds((current) => selected
      ? Array.from(new Set([...current, selectionId]))
      : current.filter((id) => id !== selectionId));
  }

  function startEditClientIdentity(item: ClientIdentityItem) {
    setEditingClientKeyId(item.selectionId);
    clientKeyEditExpectedIdentityRef.current = { ...item.identity };
    setClientKeyEditDraft({
      profileId: item.profileId,
      identityId: item.identity.id,
      label: item.identity.label,
      source: item.identity.source,
      fingerprintSha256: item.identity.fingerprintSha256 ?? "",
      path: item.identity.path ?? "",
      secretRef: item.identity.secretRef ?? "",
    });
    setClientKeyPrivateKey("");
    setClientKeyPassphrase("");
    setClientKeyStorage(item.identity.secretRef?.startsWith("stronghold:") ? "portable" : "auto");
    setError("");
    setStatus("");
  }

  function applyClientIdentityMutation(response: ClientIdentityMutationResponse, message: string, token: number) {
    const accepted = onProfileChange(response.summary, token);
    if (!accepted || !mountedRef.current) return false;
    if (clientKeyEditDraft) {
      const connection = response.summary.profile.connection;
      if (connection.kind === "ssh" || connection.kind === "tmux") {
        const identity = connection.identityRefs.find((item) => item.id === clientKeyEditDraft.identityId);
        if (identity) {
          clientKeyEditExpectedIdentityRef.current = { ...identity };
          setClientKeyEditDraft({
            profileId: response.summary.profile.id,
            identityId: identity.id,
            label: identity.label,
            source: identity.source,
            fingerprintSha256: identity.fingerprintSha256 ?? "",
            path: identity.path ?? "",
            secretRef: identity.secretRef ?? "",
          });
        }
      }
    }
    const suffix = response.cleanupWarning
      ? ` · ${response.cleanupWarning}`
      : response.oldSecretDeleted
        ? " · 旧 secret 已清理"
        : response.oldSecretShared
          ? " · 旧 secret 仍被共享，已保留"
          : "";
    setStatus(`${message}${suffix}`);
    return true;
  }

  async function saveClientIdentity() {
    const expectedIdentity = clientKeyEditExpectedIdentityRef.current;
    if (!clientKeyEditDraft || !expectedIdentity) return;
    const mutationProfileId = clientKeyEditDraft.profileId;
    const mutationToken = onProfileMutationStart(mutationProfileId);
    let backendSucceeded = false;
    setClientKeyMutationBusy(true);
    setError("");
    setStatus("");
    try {
      const response = await invokeBackend<ClientIdentityMutationResponse>("update_client_identity", {
        request: {
          profileId: clientKeyEditDraft.profileId,
          identityId: clientKeyEditDraft.identityId,
          expectedIdentity,
          label: clientKeyEditDraft.label,
          source: clientKeyEditDraft.source,
          fingerprintSha256: clientKeyEditDraft.fingerprintSha256 || null,
          path: clientKeyEditDraft.path || null,
          secretRef: clientKeyEditDraft.secretRef || null,
        },
      });
      backendSucceeded = true;
      applyClientIdentityMutation(response, "Client identity 已更新", mutationToken);
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onProfileMutationFinish(mutationProfileId, mutationToken, backendSucceeded);
      if (mountedRef.current) setClientKeyMutationBusy(false);
    }
  }

  async function rotateClientIdentity() {
    if (!clientKeyEditDraft || !clientKeyPrivateKey.trim()) return;
    const mutationProfileId = clientKeyEditDraft.profileId;
    const mutationToken = onProfileMutationStart(mutationProfileId);
    let backendSucceeded = false;
    setClientKeyMutationBusy(true);
    setError("");
    setStatus("");
    try {
      const response = await invokeBackend<ClientIdentityMutationResponse>("rotate_client_identity", {
        request: {
          profileId: clientKeyEditDraft.profileId,
          identityId: clientKeyEditDraft.identityId,
          privateKey: clientKeyPrivateKey,
          passphrase: clientKeyPassphrase || null,
          storage: clientKeyStorage === "auto" ? null : clientKeyStorage,
        },
      });
      backendSucceeded = true;
      if (applyClientIdentityMutation(response, "Vault 私钥已轮换", mutationToken)) {
        setClientKeyPrivateKey("");
        setClientKeyPassphrase("");
      }
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onProfileMutationFinish(mutationProfileId, mutationToken, backendSucceeded);
      if (mountedRef.current) setClientKeyMutationBusy(false);
    }
  }

  async function deleteEditedClientIdentity(deleteSecret: boolean) {
    if (!clientKeyEditDraft || editingClientIdentityItem?.jumpInUse) return;
    const action = deleteSecret ? "移除该引用并清理未共享 secret" : "移除该 identity 引用";
    if (!window.confirm(`${action}？`)) return;
    const mutationProfileId = clientKeyEditDraft.profileId;
    const mutationToken = onProfileMutationStart(mutationProfileId);
    let backendSucceeded = false;
    setClientKeyMutationBusy(true);
    setError("");
    setStatus("");
    try {
      const response = await invokeBackend<ClientIdentityMutationResponse>("delete_client_identity", {
        request: {
          profileId: clientKeyEditDraft.profileId,
          identityId: clientKeyEditDraft.identityId,
          deleteSecret,
        },
      });
      backendSucceeded = true;
      if (applyClientIdentityMutation(response, "Client identity 引用已移除", mutationToken)) {
        setEditingClientKeyId("");
        setClientKeyEditDraft(null);
        clientKeyEditExpectedIdentityRef.current = null;
        setClientKeyPrivateKey("");
        setClientKeyPassphrase("");
      }
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onProfileMutationFinish(mutationProfileId, mutationToken, backendSucceeded);
      if (mountedRef.current) setClientKeyMutationBusy(false);
    }
  }

  function toggleAgentIdentitySelection(identity: IdentityRef, selected: boolean) {
    const id = identityStableKey(identity);
    setSelectedAgentKeyIds((current) => selected
      ? Array.from(new Set([...current, id]))
      : current.filter((item) => item !== id));
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog key-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>密钥管理器</strong>
          <button type="button" title="关闭" aria-label="关闭密钥管理器" onClick={onClose}><X size={20} /></button>
        </header>
        <div className="key-content">
          <section className="key-list">
            <div className="key-list-toolbar">
              <select value={keyScopeFilter} onChange={(event) => setKeyScopeFilter(event.target.value as TrustedHostKey["scope"] | "all")}>
                <option value="all">全部 scope</option>
                <option value="profile">profile</option>
                <option value="project">project</option>
                <option value="user">user</option>
              </select>
              <select value={keyProfileFilter} onChange={(event) => setKeyProfileFilter(event.target.value)}>
                <option value="all">全部 profile</option>
                {sshSessions.map((session) => (
                  <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                ))}
              </select>
              <button type="button" onClick={selectVisibleHostKeys} disabled={!visibleHostKeys.length}>全选</button>
              <button type="button" onClick={() => setSelectedHostKeyIds([])} disabled={!selectedHostKeyIds.length}>清除</button>
            </div>
            <div className="key-batch-actions">
              <span>{selectedHostKeyIds.length} selected</span>
              <button type="button" onClick={() => void copySelectedHostKeysToProfile()} disabled={credentialMutationControlsDisabled || !selectedVisibleHostKeys.length || !selectedProfile}>复制到 Profile</button>
              <button type="button" onClick={() => void deleteSelectedHostKeys()} disabled={!selectedHostKeyIds.length}>删除</button>
            </div>
            {visibleHostKeys.map((key) => (
              <div key={key.id} className="key-row">
                <label className="key-row-select">
                  <input type="checkbox" checked={selectedHostKeyIds.includes(key.id)} onChange={(event) => toggleHostKeySelection(key.id, event.target.checked)} />
                </label>
                <strong>{key.alias}:{key.port}</strong>
                <span>{key.algorithm} · {key.fingerprintSha256}</span>
                <small>{key.scope} · {key.label ?? key.host} · 最近验证 {formatHostKeyDate(key.lastSeen)}</small>
                <div className="key-row-actions">
                  <button onClick={() => startEditKey(key)}>编辑</button>
                  <button onClick={() => void copyHostKeyToProfile(key)} disabled={credentialMutationControlsDisabled || !selectedProfile}>复制到 Profile</button>
                  <button onClick={() => void deleteKey(key.id)}>删除</button>
                </div>
              </div>
            ))}
            {!hostKeys.keys.length ? <div className="empty-pane top">没有保存的 host key</div> : null}
            {hostKeys.keys.length && !visibleHostKeys.length ? <div className="empty-pane top">当前分组没有 host key</div> : null}
          </section>
          <section className="key-editor">
            <DialogField label="Profile:">
              <select value={profileId} onChange={(event) => setProfileId(event.target.value)}>
                {sshSessions.map((session) => (
                  <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                ))}
              </select>
            </DialogField>
            <section className="host-key-scan-panel" aria-live="polite">
              <header>
                <div><strong>当前 Host Key</strong><small>{selectedProfile ? describeSshProfileTarget(selectedProfile) : "未选择 Profile"}</small></div>
                <button type="button" onClick={() => void scanSelectedProfileHostKey()} disabled={!selectedProfile || hostKeyScanBusy}>
                  <RefreshCw size={14} className={hostKeyScanBusy ? "loading" : ""} />{hostKeyScanBusy ? "扫描中" : "扫描"}
                </button>
              </header>
              {hostKeyScan ? (
                <div className={`host-key-scan-result ${hostKeyScan.evaluation.status}`}>
                  <div className="host-key-scan-status">
                    {hostKeyScan.evaluation.status === "trusted" ? <CheckCircle2 size={15} /> : <AlertCircle size={15} />}
                    <strong>{hostKeyScanStatus(hostKeyScan)}</strong>
                  </div>
                  <dl>
                    <div><dt>目标</dt><dd>{hostKeyScan.observation.alias || hostKeyScan.observation.host}:{hostKeyScan.observation.port}</dd></div>
                    <div><dt>算法</dt><dd>{hostKeyScan.observation.algorithm}</dd></div>
                    <div><dt>指纹</dt><dd>{hostKeyScanFingerprint(hostKeyScan)}</dd></div>
                    {hostKeyScan.evaluation.status === "mismatch" ? <div><dt>已保存</dt><dd>{hostKeyScan.evaluation.expected.map((key) => key.fingerprintSha256).join(" · ")}</dd></div> : null}
                  </dl>
                  {hostKeyScan.evaluation.status !== "trusted" ? (
                    <div className="host-key-scan-actions">
                      <button type="button" onClick={() => void trustHostKeyScan("append-to-profile")} disabled={hostKeyScanBusy}>加入 Profile</button>
                      <button type="button" onClick={() => void trustHostKeyScan("append-to-project")} disabled={hostKeyScanBusy}>加入 Project</button>
                      {hostKeyScan.evaluation.status === "mismatch" ? <button type="button" className="danger" onClick={() => void trustHostKeyScan("replace-for-profile")} disabled={hostKeyScanBusy}>替换 Profile</button> : null}
                    </div>
                  ) : null}
                </div>
              ) : null}
              {hostKeyScanError ? <div className="host-key-scan-error">{hostKeyScanError}</div> : null}
            </section>
            {editDraft ? (
              <section className="key-edit-panel">
                <div className="key-edit-heading">
                  <strong>Host Key</strong>
                  <button type="button" onClick={() => { setEditingKeyId(""); setEditDraft(null); }}>关闭</button>
                </div>
                <DialogField label="Alias:">
                  <input value={editDraft.alias} onChange={(event) => setEditDraft({ ...editDraft, alias: event.target.value })} />
                </DialogField>
                <DialogField label="Host:">
                  <input value={editDraft.host} onChange={(event) => setEditDraft({ ...editDraft, host: event.target.value })} />
                </DialogField>
                <DialogField label="Port:">
                  <input type="number" min={1} max={65535} value={editDraft.port} onChange={(event) => setEditDraft({ ...editDraft, port: Number(event.target.value) || 22 })} />
                </DialogField>
                <DialogField label="Scope:">
                  <select value={editDraft.scope} onChange={(event) => setEditDraft({ ...editDraft, scope: event.target.value as TrustedHostKey["scope"] })}>
                    <option value="profile">profile</option>
                    <option value="project">project</option>
                    <option value="user">user</option>
                  </select>
                </DialogField>
                <DialogField label="Profile:">
                  <select value={editDraft.profileId} onChange={(event) => setEditDraft({ ...editDraft, profileId: event.target.value })}>
                    <option value="">无</option>
                    {sshSessions.map((session) => (
                      <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                    ))}
                  </select>
                </DialogField>
                <DialogField label="Label:">
                  <input value={editDraft.label} onChange={(event) => setEditDraft({ ...editDraft, label: event.target.value })} />
                </DialogField>
                <div className="key-edit-meta">
                  <span>{editingKey?.algorithm ?? ""}</span>
                  <span>{editingKey?.fingerprintSha256 ?? ""}</span>
                  <span>首次 {formatHostKeyDate(editingKey?.firstSeen)} · 最近 {formatHostKeyDate(editingKey?.lastSeen)}</span>
                </div>
                <div className="key-actions">
                  <button type="button" onClick={() => void saveEditedHostKey()}>保存编辑</button>
                </div>
              </section>
            ) : null}
            <DialogField label="known_hosts:">
              <textarea value={knownHostsText} onChange={(event) => setKnownHostsText(event.target.value)} placeholder="粘贴 OpenSSH known_hosts 内容" />
            </DialogField>
            {error ? <div className="utility-error">{error}</div> : null}
            {status ? <div className="utility-status">{status}</div> : null}
            <div className="key-actions">
              <button onClick={() => void importKnownHostsText()} disabled={!profileId || !knownHostsText.trim()}>导入</button>
              <button onClick={() => void exportKnownHostsText()}>导出</button>
            </div>
            {exportText ? (
              <textarea className="key-export" value={exportText} onChange={(event) => setExportText(event.target.value)} />
            ) : null}
          </section>
          <section className="key-agent-list">
            <div className="key-agent-header">
              <span><KeyRound size={15} /><strong>Client Keys</strong></span>
              <small>{clientIdentityItems.length} identities</small>
            </div>
            <div className="portable-vault-bar" title={portableVault?.path ?? "Portable Stronghold vault"}>
              <span className={portableVault?.unlocked ? "unlocked" : ""}>{portableVault?.unlocked ? <Unlock size={14} /> : <Lock size={14} />}<strong>Stronghold</strong><small>{portableVault?.unlocked ? "Unlocked" : portableVault?.exists ? "Locked" : "Not created"}</small></span>
              <input type="password" aria-label={portableVault?.exists ? "Stronghold 主密码" : "新建 Stronghold 主密码"} value={portableVaultPassword} onChange={(event) => setPortableVaultPassword(event.target.value)} placeholder={portableVault?.exists ? "Master password" : "New master password"} disabled={vaultOperationBusy || portableVault?.unlocked} onKeyDown={(event) => { if (event.key === "Enter") void unlockPortableVault(); }} />
              {portableVault?.unlocked
                ? <button className="key-icon-button" type="button" title="锁定 portable vault" aria-label="锁定 portable vault" onClick={() => void lockPortableVault()} disabled={vaultOperationBusy}><Lock size={14} /></button>
                : <button className="key-icon-button" type="button" title="解锁 portable vault" aria-label="解锁 portable vault" onClick={() => void unlockPortableVault()} disabled={vaultOperationBusy || !portableVaultPassword}><Unlock size={14} /></button>}
            </div>
            {portableVaultFeedback ? <div className={`portable-vault-feedback ${portableVaultFeedback.kind}`} role={portableVaultFeedback.kind === "error" ? "alert" : "status"} aria-live="polite">{portableVaultFeedback.message}</div> : null}
            {portableVault?.unlocked ? (
              <details className="portable-vault-rotation" onToggle={(event) => { if (!event.currentTarget.open) { clearPortableVaultRotation(); setPortableVaultFeedback(null); } }}>
                <summary><RefreshCw size={14} /><span>更换主密码</span></summary>
                <div className="portable-vault-rotation-fields">
                  <label><span>当前主密码</span><input type="password" autoComplete="current-password" value={portableVaultCurrentPassword} onChange={(event) => setPortableVaultCurrentPassword(event.target.value)} disabled={credentialMutationControlsDisabled} /></label>
                  <label><span>新主密码</span><input type="password" autoComplete="new-password" value={portableVaultNewPassword} onChange={(event) => setPortableVaultNewPassword(event.target.value)} disabled={credentialMutationControlsDisabled} /></label>
                  <label><span>确认新主密码</span><input type="password" autoComplete="new-password" value={portableVaultConfirmPassword} onChange={(event) => setPortableVaultConfirmPassword(event.target.value)} disabled={credentialMutationControlsDisabled} onKeyDown={(event) => { if (event.key === "Enter") void rotatePortableVaultPassword(); }} /></label>
                  <button type="button" onClick={() => void rotatePortableVaultPassword()} disabled={credentialMutationControlsDisabled || !portableVaultCurrentPassword || !portableVaultNewPassword || !portableVaultConfirmPassword}><RefreshCw size={14} />更换主密码</button>
                </div>
              </details>
            ) : null}
            {migrationRecovery || migrationRecoveryStatusError || migrationRecoveryError || migrationRecoveryWarnings.length ? (
              <section className={`portable-vault-migration-recovery${migrationRecovery?.disposition === "conflict" ? " conflict" : ""}`} aria-live="polite">
                <header>
                  <span>{migrationRecovery || migrationRecoveryStatusError ? <AlertCircle size={15} /> : <CheckCircle2 size={15} />}<strong>{migrationRecovery ? "待恢复的凭据迁移" : migrationRecoveryStatusError ? "无法核对凭据迁移状态" : "凭据迁移恢复完成"}</strong></span>
                  {migrationRecovery ? <small>{migrationRecoveryDispositionLabels[migrationRecovery.disposition]}</small> : null}
                </header>
                {migrationRecovery ? (
                  <>
                    <dl>
                      <div><dt>阶段</dt><dd>{migrationRecoveryStateLabels[migrationRecovery.state]}</dd></div>
                      <div><dt>Profile</dt><dd>{migrationRecovery.profileCount}</dd></div>
                      <div><dt>Secret</dt><dd>{migrationRecovery.secretCount}</dd></div>
                    </dl>
                    <p>{migrationRecovery.message}</p>
                    {migrationRecovery.requiresPortableVaultUnlock ? <p className="portable-vault-migration-recovery-unlock"><Lock size={13} />请先锁定并重新解锁 Stronghold</p> : null}
                    {migrationRecovery.disposition === "conflict" || migrationRecovery.state === "needs-resolution"
                      ? <p className="portable-vault-migration-recovery-manual">自动恢复已停止；请人工核对 Profile 引用与两侧 provider，PortMate 不会自动改写 Profile。</p>
                      : migrationRecoveryStatusError
                        ? null
                        : <button type="button" onClick={() => void recoverPendingProfileSecretMigration()} disabled={migrationRecoveryChecking || !canRecoverProfileSecretMigration(migrationRecovery, portableVault?.unlocked ?? false, vaultOperationBusy || migrationRequiresRestart)}><RefreshCw size={14} />{migrationRecoveryBusy ? "核对中" : "核对并恢复"}</button>}
                  </>
                ) : null}
                {migrationRecovery || migrationRecoveryStatusError ? <button type="button" onClick={() => void exportPendingProfileSecretMigrationDiagnostics()} disabled={migrationRecoveryChecking || vaultOperationBusy}><FileText size={14} />{migrationDiagnosticBusy ? "导出中" : "导出诊断"}</button> : null}
                {migrationDiagnosticResult ? <p className="portable-vault-migration-diagnostic-result" title={migrationDiagnosticResult.path}>诊断已导出：{migrationDiagnosticResult.path} · {formatBytes(migrationDiagnosticResult.size)} · SHA-256 {migrationDiagnosticResult.sha256.slice(0, 16)}...</p> : null}
                {migrationDiagnosticResult ? <button type="button" onClick={() => void navigator.clipboard?.writeText(`${migrationDiagnosticResult.path}\n${migrationDiagnosticResult.checksumPath}\nSHA-256 ${migrationDiagnosticResult.sha256}`).catch(() => {})}><Copy size={14} />复制导出信息</button> : null}
                {migrationRecoveryWarnings.map((warning) => <p className="portable-vault-migration-recovery-warning" key={warning}>{warning}</p>)}
                {migrationRecoveryStatusError ? <p className="portable-vault-migration-recovery-error" role="alert">状态读取失败：{migrationRecoveryStatusError}</p> : null}
                {migrationRecoveryStatusError ? <button type="button" onClick={() => void refreshMigrationRecovery()} disabled={migrationRecoveryChecking || vaultOperationBusy}><RefreshCw size={14} />{migrationRecoveryChecking ? "读取中" : "重新读取"}</button> : null}
                {migrationRecoveryError ? <p className="portable-vault-migration-recovery-error" role="alert">{migrationRecoveryError}</p> : null}
              </section>
            ) : null}
            {portableVault?.unlocked || migrationResult || migrationError || migrationRecovery ? (
              <details className="portable-vault-migration">
                <summary><ArrowRightLeft size={14} /><span>迁移 Profile 凭据</span></summary>
                {portableVault?.unlocked && !migrationRecovery ? (
                  <>
                    <div className="portable-vault-migration-config">
                      <div className="portable-vault-migration-direction" role="group" aria-label="凭据迁移方向">
                        <button type="button" aria-pressed={migrationTarget === "portable"} onClick={() => { setMigrationTarget("portable"); invalidateMigrationState(); }} disabled={migrationControlsDisabled}>Native → Stronghold</button>
                        <button type="button" aria-pressed={migrationTarget === "native"} onClick={() => { setMigrationTarget("native"); invalidateMigrationState(); }} disabled={migrationControlsDisabled}>Stronghold → Native</button>
                      </div>
                      <label><span>Profile 范围</span><select value={migrationScopeProfileId} onChange={(event) => { setMigrationScopeProfileId(event.target.value); invalidateMigrationState(); }} disabled={migrationControlsDisabled}><option value="all">全部凭据 Profile</option>{credentialSessions.map((session) => <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>)}</select></label>
                      <label className="portable-vault-migration-cleanup"><input type="checkbox" checked={migrationCleanupSource} onChange={(event) => { setMigrationCleanupSource(event.target.checked); invalidateMigrationState(); }} disabled={migrationControlsDisabled} /><span>清理未共享的源 Secret</span></label>
                      <button className="portable-vault-migration-preview-button" type="button" onClick={() => void previewProfileSecretMigration()} disabled={migrationControlsDisabled || !credentialSessions.length}><RefreshCw size={14} />{migrationBusy === "preview" ? "预检中" : "预检"}</button>
                    </div>
                    {migrationPreviewState ? (
                      <div className="portable-vault-migration-preview" role="status" aria-live="polite">
                        <dl>
                          <div><dt>Profile</dt><dd>{migrationPreviewState.preview.affectedProfileCount}/{migrationPreviewState.preview.selectedProfileCount}</dd></div>
                          <div><dt>引用</dt><dd>{migrationPreviewState.preview.eligibleReferenceCount}</dd></div>
                          <div><dt>Secret</dt><dd>{migrationPreviewState.preview.eligibleSecretCount}</dd></div>
                          <div><dt>共享保留</dt><dd>{migrationPreviewState.preview.retainedSharedSecretCount}</dd></div>
                        </dl>
                        {migrationPreviewState.preview.alreadyTargetReferenceCount ? <p>{migrationPreviewState.preview.alreadyTargetReferenceCount} 个引用已位于目标存储</p> : null}
                        {migrationPreviewState.preview.retainedInFlightSecretCount ? <p>{migrationPreviewState.preview.retainedInFlightSecretCount} 个源 Secret 因建连中而保留</p> : null}
                        {migrationPreviewState.preview.excludedReservedReferenceCount ? <p>{migrationPreviewState.preview.excludedReservedReferenceCount} 个 MCP token 保留引用已排除</p> : null}
                        <button type="button" onClick={() => void migrateProfileSecrets()} disabled={!canExecuteProfileSecretMigration(migrationPreviewState.preview, portableVault.unlocked, migrationControlsDisabled, Boolean(migrationRecovery))}><ArrowRightLeft size={14} />{migrationBusy === "migrate" ? "迁移中" : migrationPreviewState.preview.eligibleSecretCount ? "确认迁移" : "无需迁移"}</button>
                      </div>
                    ) : null}
                  </>
                ) : null}
                {migrationResult && migrationCleanupSummary ? (
                  <div className="portable-vault-migration-result" role="status" aria-live="polite">
                    <strong>{migrationResult.migratedProfileCount} 个 Profile · {migrationResult.migratedReferenceCount} 个引用 · {migrationResult.migratedSecretCount} 个 Secret</strong>
                    <span>源清理：{migrationCleanupSummary.deleted} 删除 · {migrationCleanupSummary["retained-shared"]} 共享保留 · {migrationCleanupSummary["retained-in-use"]} 建连保留 · {migrationCleanupSummary["retained-by-request"]} 按设置保留 · {migrationCleanupSummary.failed} 失败</span>
                    {migrationResult.warnings.map((warning) => <p key={warning}>{warning}</p>)}
                  </div>
                ) : null}
                {migrationError ? <div className="portable-vault-migration-error" role="alert">{migrationError}</div> : null}
              </details>
            ) : null}
            <div className="client-key-filters">
              <label className="client-key-search">
                <Search size={14} />
                <input value={clientKeyQuery} onChange={(event) => setClientKeyQuery(event.target.value)} placeholder="搜索 label、指纹或路径" />
              </label>
              <select value={clientKeySourceFilter} onChange={(event) => setClientKeySourceFilter(event.target.value as IdentityRef["source"] | "all")} aria-label="Client key 来源">
                <option value="all">全部来源</option>
                <option value="profile-vault">Profile Vault</option>
                <option value="system-file">System File</option>
                <option value="agent">SSH Agent</option>
                <option value="public-key-only">Public Key</option>
              </select>
              <select value={clientKeyProfileFilter} onChange={(event) => setClientKeyProfileFilter(event.target.value)} aria-label="Client key Profile">
                <option value="all">全部 Profile</option>
                {sshSessions.map((session) => (
                  <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                ))}
              </select>
              <select value={clientKeyGroupBy} onChange={(event) => setClientKeyGroupBy(event.target.value as ClientIdentityGroupBy)} aria-label="Client key 分组">
                <option value="profile">按 Profile 分组</option>
                <option value="source">按来源分组</option>
              </select>
            </div>
            <div className="client-key-batch">
              <span>{selectedClientIdentityItems.length} selected</span>
              <button type="button" onClick={() => setSelectedClientKeyIds((current) => Array.from(new Set([...current, ...visibleClientIdentityItems.map((item) => item.selectionId)])))} disabled={!visibleClientIdentityItems.length}>全选结果</button>
              <button type="button" onClick={() => setSelectedClientKeyIds([])} disabled={!selectedClientKeyIds.length}>清除</button>
              <div className="client-key-command-group">
                <button className="key-icon-button" type="button" title={`复制到 ${selectedProfile?.name ?? "Profile"}`} aria-label={`复制到 ${selectedProfile?.name ?? "Profile"}`} onClick={() => void copyClientIdentitiesToProfile(selectedClientIdentityItems)} disabled={credentialMutationControlsDisabled || !selectedClientIdentityItems.length || !selectedProfile}><Copy size={15} /></button>
                <button className="key-icon-button" type="button" title="在各自 Profile 中置顶" aria-label="在各自 Profile 中置顶" onClick={() => void moveSelectedClientIdentitiesFirst()} disabled={credentialMutationControlsDisabled || !selectedClientIdentityItems.length}><ArrowUp size={15} /></button>
                <button className="key-icon-button danger" type="button" title="从各自 Profile 移除引用" aria-label="从各自 Profile 移除引用" onClick={() => void removeSelectedClientIdentities()} disabled={credentialMutationControlsDisabled || !selectedClientIdentityItems.length}><Trash2 size={15} /></button>
              </div>
            </div>
            <div className="client-key-groups">
              {clientIdentityGroups.map((group) => (
                <section key={group.id} className="client-key-group">
                  <header><strong>{group.label}</strong><span>{group.items.length}</span></header>
                  {group.items.map((item) => (
                    <div key={item.selectionId} className={`client-key-row${item.jumpInUse ? " in-use" : ""}${editingClientKeyId === item.selectionId ? " editing" : ""}`}>
                      <input type="checkbox" checked={selectedClientKeyIds.includes(item.selectionId)} onChange={(event) => toggleClientIdentitySelection(item.selectionId, event.target.checked)} />
                      <span className="client-key-main">
                        <strong title={item.identity.label}>{item.identity.label}</strong>
                        <code title={item.identity.fingerprintSha256 ?? item.identity.path ?? item.identity.id}>{item.identity.fingerprintSha256 ?? item.identity.path ?? "No fingerprint"}</code>
                      </span>
                      <span className="client-key-meta">
                        <span>{identitySourceLabel(item.identity.source)}</span>
                        {clientKeyGroupBy === "source" ? <span>{item.profileName}</span> : null}
                        {item.jumpInUse ? <span className="client-key-in-use">Jump Host 使用中</span> : null}
                      </span>
                      <button className="key-icon-button client-key-edit-button" type="button" title="编辑 client identity" aria-label={`编辑 ${item.identity.label}`} onClick={() => startEditClientIdentity(item)}><Pencil size={14} /></button>
                    </div>
                  ))}
                </section>
              ))}
              {!clientIdentityItems.length ? <div className="empty-pane top">Profile 中还没有 client identity</div> : null}
              {clientIdentityItems.length && !visibleClientIdentityItems.length ? <div className="empty-pane top">当前筛选没有 client identity</div> : null}
            </div>
            {clientKeyEditDraft && editingClientIdentityItem ? (
              <section className="client-key-inspector">
                <header>
                  <span><Pencil size={14} /><strong>Identity Inspector</strong></span>
                  <button className="key-icon-button" type="button" title="关闭检查器" aria-label="关闭 identity 检查器" onClick={() => { setEditingClientKeyId(""); setClientKeyEditDraft(null); clientKeyEditExpectedIdentityRef.current = null; }}><X size={14} /></button>
                </header>
                <div className="client-key-inspector-grid">
                  <label><span>Label</span><input value={clientKeyEditDraft.label} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, label: event.target.value })} /></label>
                  <label><span>Source</span><select value={clientKeyEditDraft.source} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, source: event.target.value as IdentityRef["source"] })}><option value="profile-vault">Profile Vault</option><option value="system-file">System File</option><option value="agent">SSH Agent</option><option value="public-key-only">Public Key</option></select></label>
                  <label><span>Fingerprint</span><input value={clientKeyEditDraft.fingerprintSha256} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, fingerprintSha256: event.target.value })} placeholder="SHA256:..." /></label>
                  <label><span>Path / Agent comment</span><input value={clientKeyEditDraft.path} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, path: event.target.value })} disabled={clientKeyEditDraft.source === "profile-vault"} /></label>
                  <label><span>Identity ID</span><input value={clientKeyEditDraft.identityId} readOnly /></label>
                  <label><span>Profile</span><input value={editingClientIdentityItem.profileName} readOnly /></label>
                  {clientKeyEditDraft.source === "profile-vault" ? <label><span>Rotation storage</span><select value={clientKeyStorage} onChange={(event) => setClientKeyStorage(event.target.value as SecretStorageChoice)}><option value="auto">Auto / native first</option><option value="native">Native keyring</option><option value="portable" disabled={!portableVault?.unlocked}>Portable Stronghold</option></select></label> : null}
                  {clientKeyEditDraft.source === "profile-vault" ? <label className="client-key-secret-ref"><span>Secret ref</span><input value={clientKeyEditDraft.secretRef} readOnly /></label> : null}
                </div>
                <div className="client-key-impact">
                  <span>{editingClientIdentityItem.jumpInUse ? "Jump Host 使用中" : "未被 Jump Host 使用"}</span>
                  {editingClientSecretUsage > 1 ? <span>{editingClientSecretUsage} 个 identity 共享此 secret</span> : <span>{editingClientSecretUsage ? "Secret 未共享" : "无 secret"}</span>}
                </div>
                <div className="client-key-inspector-actions">
                  <button type="button" onClick={() => void saveClientIdentity()} disabled={clientKeyMutationBusy || credentialMutationControlsDisabled}>保存字段</button>
                  <button className="danger" type="button" onClick={() => void deleteEditedClientIdentity(false)} disabled={clientKeyMutationBusy || credentialMutationControlsDisabled || editingClientIdentityItem.jumpInUse}>移除引用</button>
                  {editingClientIdentityItem.identity.secretRef ? <button className="danger" type="button" onClick={() => void deleteEditedClientIdentity(true)} disabled={clientKeyMutationBusy || credentialMutationControlsDisabled || editingClientIdentityItem.jumpInUse}>移除并清理 Secret</button> : null}
                </div>
                {clientKeyEditDraft.source === "profile-vault" ? (
                  <div className="client-key-rotation">
                    <textarea value={clientKeyPrivateKey} onChange={(event) => setClientKeyPrivateKey(event.target.value)} placeholder="新的 OpenSSH private key" />
                    <input type="password" value={clientKeyPassphrase} onChange={(event) => setClientKeyPassphrase(event.target.value)} placeholder="新私钥口令（可选）" />
                    <button type="button" onClick={() => void rotateClientIdentity()} disabled={clientKeyMutationBusy || credentialMutationControlsDisabled || !clientKeyPrivateKey.trim()}><RefreshCw size={14} />轮换 Vault 私钥</button>
                  </div>
                ) : null}
              </section>
            ) : null}
            <details className="key-import-panel">
              <summary><Plus size={14} />导入私钥到 {selectedProfile?.name ?? "Profile"}</summary>
              <input value={privateKeyLabel} onChange={(event) => setPrivateKeyLabel(event.target.value)} placeholder="Key label" />
              <input type="file" accept=".pem,.key,.txt" onChange={(event) => void readPrivateKeyFile(event.currentTarget.files?.[0] ?? null)} />
              <select value={privateKeyStorage} onChange={(event) => setPrivateKeyStorage(event.target.value as SecretStorageChoice)}><option value="auto">存储：自动（优先系统）</option><option value="native">存储：系统密钥库</option><option value="portable" disabled={!portableVault?.unlocked}>存储：Portable Stronghold</option></select>
              <textarea value={privateKeyText} onChange={(event) => setPrivateKeyText(event.target.value)} placeholder="粘贴 OpenSSH private key" />
              <button onClick={() => void importPrivateKeyToProfile()} disabled={clientKeyMutationBusy || credentialMutationControlsDisabled || !selectedProfile || !privateKeyText.trim()}>导入到 Profile</button>
            </details>
            <div className="key-agent-header agent-section-header">
              <span><strong>Agent Keys</strong><small>{agentKeys.length} visible</small></span>
              <button onClick={() => void refreshAgentKeys()}>刷新</button>
            </div>
            <div className="client-key-batch agent-key-batch">
              <span>{selectedAgentKeys.length} selected</span>
              <button type="button" onClick={() => setSelectedAgentKeyIds(agentKeys.map(identityStableKey))} disabled={!agentKeys.length}>全选</button>
              <button type="button" onClick={() => setSelectedAgentKeyIds([])} disabled={!selectedAgentKeyIds.length}>清除</button>
              <button className="key-icon-button" type="button" title={`批量添加到 ${selectedProfile?.name ?? "Profile"}`} aria-label={`批量添加到 ${selectedProfile?.name ?? "Profile"}`} onClick={() => void copyAgentIdentitiesToProfile(selectedAgentKeys)} disabled={credentialMutationControlsDisabled || !selectedAgentKeys.length || !selectedProfile}><UserPlus size={15} /></button>
            </div>
            <div className="agent-key-list">
              {agentKeys.map((identity, index) => (
                <div key={`${identityStableKey(identity)}:${index}`} className="client-key-row agent-row">
                  <input type="checkbox" checked={selectedAgentKeyIds.includes(identityStableKey(identity))} onChange={(event) => toggleAgentIdentitySelection(identity, event.target.checked)} />
                  <span className="client-key-main">
                    <strong title={identity.label}>{identity.label}</strong>
                    <code title={identity.fingerprintSha256 ?? ""}>{identity.fingerprintSha256 ?? "未识别指纹"}</code>
                  </span>
                  <span className="client-key-meta"><span>{identity.path ?? "ssh-agent"}</span></span>
                  <button className="key-icon-button" type="button" title={`添加到 ${selectedProfile?.name ?? "Profile"}`} aria-label={`添加 ${identity.label} 到 ${selectedProfile?.name ?? "Profile"}`} onClick={() => void copyAgentIdentityToProfile(identity)} disabled={credentialMutationControlsDisabled || !selectedProfile}><UserPlus size={15} /></button>
                </div>
              ))}
              {!agentKeys.length ? <div className="empty-pane top">没有可见的 ssh-agent 身份</div> : null}
            </div>
          </section>
        </div>
      </section>
    </div>
  );
}

function DialogField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="dialog-field">
      <span>{label}</span>
      {children}
    </label>
  );
}

function isSshLikeProfile(profile: SessionProfile): profile is SessionProfile & { connection: Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> } {
  return profile.connection.kind === "ssh" || profile.connection.kind === "tmux";
}

function describeSshProfileTarget(profile: SessionProfile) {
  if (!isSshLikeProfile(profile)) return profile.name;
  const alias = profile.connection.hostKeyPolicy.alias?.trim();
  const host = alias || profile.connection.endpoint.host;
  return `${host}:${profile.connection.endpoint.port}`;
}

function hostKeyScanStatus(scan: HostKeyScanResult) {
  switch (scan.evaluation.status) {
    case "trusted":
      return "与已信任 Host Key 一致";
    case "unknown":
      return "尚未信任此 Host Key";
    case "mismatch":
      return "Host Key 与已保存记录不一致";
  }
}

function hostKeyScanFingerprint(scan: HostKeyScanResult) {
  return scan.evaluation.status === "mismatch"
    ? scan.evaluation.observedFingerprintSha256
    : scan.evaluation.fingerprintSha256;
}

function formatHostKeyDate(value?: string | null) {
  if (!value) return "-";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "-" : date.toLocaleString();
}

function clientIdentitySelectionId(profileId: string, identity: IdentityRef, index: number) {
  return `${profileId}\0${identity.id}\0${index}`;
}

function identitySourceLabel(source: IdentityRef["source"]) {
  switch (source) {
    case "profile-vault":
      return "Profile Vault";
    case "system-file":
      return "System File";
    case "agent":
      return "SSH Agent";
    case "public-key-only":
      return "Public Key";
  }
}

function groupClientIdentityItems(items: ClientIdentityItem[], groupBy: ClientIdentityGroupBy) {
  const groups = new Map<string, { id: string; label: string; items: ClientIdentityItem[] }>();
  for (const item of items) {
    const id = groupBy === "profile" ? item.profileId : item.identity.source;
    const label = groupBy === "profile" ? item.profileName : identitySourceLabel(item.identity.source);
    const group = groups.get(id) ?? { id, label, items: [] };
    group.items.push(item);
    groups.set(id, group);
  }
  return Array.from(groups.values());
}

function createLocalId() {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function formatError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
