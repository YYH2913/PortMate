import { t, useLocale, localizeDiagnostic } from "./i18n";
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
import { formatPortableVaultError } from "./portable-vault-error";
import { invokeBackend, isBackendAvailable } from "./api";
import { identityStableKey, mergeAgentIdentities } from "./client-identity-state";
import { formatBytes } from "./display-formatters";
import { KeyedRequestGate } from "./keyed-request-gate";
import {
  clientIdentityEditDraftHasUnsavedChanges,
  hostKeyEditDraftHasUnsavedChanges,
} from "./key-manager-draft-state";
import type {
  ClientIdentityEditDraftState,
  HostKeyEditDraftState,
  HostKeyEditFields,
} from "./key-manager-draft-state";
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

const MAX_PRIVATE_KEY_IMPORT_BYTES = 1024 * 1024;

const migrationRecoveryStateLabels: Record<ProfileSecretMigrationRecoverySummary["state"], string> = {
  "target-write-pending": "target-write-pending-verification",
  "targets-verified": "target-verified",
  "profiles-committed": "profiles-committed",
  "source-cleanup-pending": "source-cleanup-pending",
  "target-cleanup-pending": "target-rollback-pending",
  "needs-resolution": "manual-verification-required",
};

const migrationRecoveryDispositionLabels: Record<ProfileSecretMigrationRecoverySummary["disposition"], string> = {
  "not-committed": "original-references-active",
  committed: "target-references-active",
  conflict: "projection-conflict",
};

type HostKeyEditDraft = HostKeyEditDraftState & {
  keyId: string;
  expectedKey: TrustedHostKey;
};

type ClientIdentityGroupBy = "profile" | "source";

type ClientIdentityItem = {
  selectionId: string;
  profileId: string;
  profileName: string;
  identity: IdentityRef;
  jumpInUse: boolean;
};

type ClientIdentityEditDraft = ClientIdentityEditDraftState;

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
  onPortableVaultStatusChange,
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
  onCredentialOperationFinish: (token: number, changed?: boolean) => void;
  onPortableVaultStatusChange?: (status: PortableVaultStatus) => void;
  onClose: () => void;
}) {
  useLocale();
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
  const [knownHostsExportBusy, setKnownHostsExportBusy] = useState(false);
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
  const [clientKeyMutationBusy, setClientKeyMutationBusy] = useState(false);
  const clientKeyMutationGate = useRef(new KeyedRequestGate<"profile-write">());
  const [selectedAgentKeyIds, setSelectedAgentKeyIds] = useState<string[]>([]);
  const [privateKeyLabel, setPrivateKeyLabel] = useState("profile key");
  const [privateKeyText, setPrivateKeyText] = useState("");
  const privateKeyFileReadGate = useRef(new KeyedRequestGate<"private-key-file">());
  const privateKeyFileReadActive = useRef(false);
  const [privateKeyFileReadBusy, setPrivateKeyFileReadBusy] = useState(false);
  const [portableVault, setPortableVault] = useState<PortableVaultStatus | null>(null);
  const [portableVaultPassword, setPortableVaultPassword] = useState("");
  const [portableVaultCreateConfirmPassword, setPortableVaultCreateConfirmPassword] = useState("");
  const [portableVaultCurrentPassword, setPortableVaultCurrentPassword] = useState("");
  const [portableVaultNewPassword, setPortableVaultNewPassword] = useState("");
  const [portableVaultConfirmPassword, setPortableVaultConfirmPassword] = useState("");
  const [portableVaultFeedback, setPortableVaultFeedback] = useState<{ kind: "error" | "status"; message: string } | null>(null);
  const [portableVaultBusy, setPortableVaultBusy] = useState(false);
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
  const migrationPreviewOperationTokenRef = useRef<number | null>(null);
  const [keyScopeFilter, setKeyScopeFilter] = useState<TrustedHostKey["scope"] | "all">("all");
  const [keyProfileFilter, setKeyProfileFilter] = useState("all");
  const [selectedHostKeyIds, setSelectedHostKeyIds] = useState<string[]>([]);
  const [editingKeyId, setEditingKeyId] = useState("");
  const [editDraft, setEditDraft] = useState<HostKeyEditDraft | null>(null);
  const [hostKeyMutationBusy, setHostKeyMutationBusy] = useState(false);
  const hostKeyMutationGate = useRef(new KeyedRequestGate<"write">());
  const [hostKeyScan, setHostKeyScan] = useState<HostKeyScanResult | null>(null);
  const [hostKeyScanBusy, setHostKeyScanBusy] = useState(false);
  const [hostKeyScanError, setHostKeyScanError] = useState("");
  const hostKeyScanProfileKeyRef = useRef("");
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const refreshGate = useRef(new KeyedRequestGate<"agent-keys" | "vault" | "recovery" | "host-scan" | "known-hosts-export">());
  const mountedRef = useRef(true);
  const credentialSyncRevisionRef = useRef(credentialSyncRevision);

  const selectedProfile = sshSessions.find((session) => session.profile.id === profileId)?.profile ?? null;
  const selectedProfileScanKey = selectedProfile
    ? JSON.stringify(prepareProfile(selectedProfile))
    : "";
  const selectedProfileScanKeyRef = useRef(selectedProfileScanKey);
  selectedProfileScanKeyRef.current = selectedProfileScanKey;
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
  const credentialProfilesKey = JSON.stringify(credentialSessions.map((session) => session.profile));
  const vaultOperationBusy = credentialOperationBusy || portableVaultBusy || migrationBusy !== null || migrationRecoveryBusy || migrationDiagnosticBusy;
  const credentialMutationsFrozen = migrationRecoveryChecking || Boolean(migrationRecovery) || Boolean(migrationRecoveryStatusError);
  const credentialMutationControlsDisabled = clientKeyMutationBusy || vaultOperationBusy || credentialMutationsFrozen;
  const clientKeyControlsDisabled = clientKeyMutationBusy || credentialMutationControlsDisabled;
  const privateKeyImportControlsDisabled = clientKeyControlsDisabled || privateKeyFileReadBusy;
  const hostKeyDraftDirty = hostKeyEditDraftHasUnsavedChanges(editDraft);
  const clientIdentityDraftDirty = clientIdentityEditDraftHasUnsavedChanges(
    clientKeyEditDraft,
    clientKeyEditExpectedIdentityRef.current,
    clientKeyPrivateKey,
    clientKeyPassphrase,
  );
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
      hostKeyMutationGate.current.invalidateAll();
      privateKeyFileReadGate.current.invalidateAll();
      privateKeyFileReadActive.current = false;
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
  }, [profileId, sessions]);

  useEffect(() => {
    if (migrationScopeProfileId !== "all" && !credentialSessions.some((session) => session.profile.id === migrationScopeProfileId)) {
      setMigrationScopeProfileId("all");
    }
    invalidateMigrationState();
  }, [credentialProfilesKey]);

  useEffect(() => {
    refreshGate.current.invalidate("host-scan");
    hostKeyScanProfileKeyRef.current = "";
    setHostKeyScan(null);
    setHostKeyScanError("");
    setHostKeyScanBusy(false);
  }, [selectedProfileScanKey]);

  useEffect(() => {
    refreshGate.current.invalidate("known-hosts-export");
    setKnownHostsExportBusy(false);
    setExportText("");
  }, [hostKeys.keys]);

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
      invalidateMigrationPreview();
    }
  }, [portableVault?.unlocked]);

  function clearPortableVaultRotation() {
    setPortableVaultCurrentPassword("");
    setPortableVaultNewPassword("");
    setPortableVaultConfirmPassword("");
  }

  function invalidateMigrationPreview() {
    const operationToken = migrationPreviewOperationTokenRef.current;
    if (operationToken !== null) {
      migrationPreviewOperationTokenRef.current = null;
      onCredentialOperationFinish(operationToken, false);
    }
    setMigrationPreviewState(null);
    setMigrationBusy((current) => current === "preview" ? null : current);
  }

  function invalidateMigrationState() {
    invalidateMigrationPreview();
    setMigrationResult(null);
    setMigrationError("");
  }

  function beginClientKeyMutation() {
    const token = clientKeyMutationGate.current.begin("profile-write");
    if (token !== null) setClientKeyMutationBusy(true);
    return token;
  }

  function finishClientKeyMutation(token: number) {
    if (clientKeyMutationGate.current.finish("profile-write", token) && mountedRef.current) {
      setClientKeyMutationBusy(false);
    }
  }

  function beginHostKeyWrite() {
    const token = hostKeyMutationGate.current.begin("write");
    if (token !== null) setHostKeyMutationBusy(true);
    return token;
  }

  function finishHostKeyWrite(token: number) {
    if (hostKeyMutationGate.current.finish("write", token) && mountedRef.current) {
      setHostKeyMutationBusy(false);
    }
  }

  function currentMigrationRequest(): ProfileSecretMigrationRequest {
    return buildProfileSecretMigrationRequest(
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
      if (refreshGate.current.isCurrent("vault", token)) applyPortableVaultStatus(next);
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

  function applyPortableVaultStatus(next: PortableVaultStatus) {
    setPortableVault(next);
    onPortableVaultStatusChange?.(next);
  }

  async function createPortableVault() {
    setPortableVaultFeedback(null);
    setError("");
    setStatus("");
    if (portableVault?.exists) {
      setPortableVaultFeedback({ kind: "error", message: t("stronghold-already-exists-use-unlock") });
      return;
    }
    if (!portableVaultPassword) {
      setPortableVaultFeedback({ kind: "error", message: t("enter-a-new-stronghold-master-password") });
      return;
    }
    if (Array.from(portableVaultPassword).length < 8) {
      setPortableVaultFeedback({ kind: "error", message: t("the-new-stronghold-master-password-must-contain-at-least") });
      return;
    }
    if (portableVaultPassword !== portableVaultCreateConfirmPassword) {
      setPortableVaultFeedback({ kind: "error", message: t("the-stronghold-master-passwords-do-not-match") });
      return;
    }
    const operationToken = onCredentialOperationStart();
    if (operationToken === null) return;
    refreshGate.current.invalidate("vault");
    setPortableVaultBusy(true);
    try {
      const next = await invokeBackend<PortableVaultStatus>("create_portable_vault", {
        request: { password: portableVaultPassword },
      });
      if (!mountedRef.current) return;
      applyPortableVaultStatus(next);
      setPortableVaultPassword("");
      setPortableVaultCreateConfirmPassword("");
      setPortableVaultFeedback({ kind: "status", message: t("stronghold-created-and-unlocked-credentials-can-now-be-saved") });
    } catch (error) {
      if (mountedRef.current) {
        setPortableVaultPassword("");
        setPortableVaultCreateConfirmPassword("");
        setPortableVaultFeedback({ kind: "error", message: formatPortableVaultError(error) });
      }
    } finally {
      onCredentialOperationFinish(operationToken);
      if (mountedRef.current) setPortableVaultBusy(false);
    }
  }

  async function unlockPortableVault() {
    if (!portableVaultPassword) return;
    if (!portableVault?.exists) {
      setPortableVaultFeedback({ kind: "error", message: t("stronghold-has-not-been-created-use-the-setup-below") });
      return;
    }
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
      applyPortableVaultStatus(next);
      setPortableVaultPassword("");
      setPortableVaultFeedback({ kind: "status", message: existed ? t("portable-vault-unlocked") : t("portable-vault-created-and-unlocked") });
    } catch (error) {
      if (mountedRef.current) {
        setPortableVaultPassword("");
        setPortableVaultFeedback({ kind: "error", message: formatPortableVaultError(error) });
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
      applyPortableVaultStatus(next);
      clearPortableVaultRotation();
      setPortableVaultFeedback({ kind: "status", message: t("portable-vault-locked") });
    } catch (error) {
      if (mountedRef.current) setPortableVaultFeedback({ kind: "error", message: formatPortableVaultError(error) });
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
      setPortableVaultFeedback({ kind: "error", message: t("enter-the-current-password-new-password-and-confirmation") });
      return;
    }
    if (Array.from(portableVaultNewPassword).length < 8) {
      setPortableVaultFeedback({ kind: "error", message: t("the-new-portable-vault-master-password-must-contain-at") });
      return;
    }
    if (portableVaultNewPassword !== portableVaultConfirmPassword) {
      setPortableVaultFeedback({ kind: "error", message: t("the-new-portable-vault-master-passwords-do-not-match") });
      return;
    }
    if (portableVaultCurrentPassword === portableVaultNewPassword) {
      setPortableVaultFeedback({ kind: "error", message: t("the-new-portable-vault-master-password-must-differ-from") });
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
      applyPortableVaultStatus(next);
      clearPortableVaultRotation();
      setPortableVaultFeedback({ kind: "status", message: t("portable-vault-master-password-changed") });
    } catch (error) {
      if (mountedRef.current) {
        clearPortableVaultRotation();
        setPortableVaultFeedback({ kind: "error", message: formatPortableVaultError(error) });
      }
    } finally {
      onCredentialOperationFinish(operationToken);
      if (mountedRef.current) setPortableVaultBusy(false);
    }
  }

  async function previewProfileSecretMigration() {
    if (!portableVault?.unlocked || migrationRequiresRestart || migrationRecovery || !isBackendAvailable()) return;
    const operationToken = onCredentialOperationStart();
    if (operationToken === null) return;
    migrationPreviewOperationTokenRef.current = operationToken;
    setMigrationBusy("preview");
    setMigrationError("");
    setMigrationResult(null);
    try {
      const request = currentMigrationRequest();
      const preview = await invokeBackend<ProfileSecretMigrationPreview>("preview_profile_secret_migration", { request });
      if (!mountedRef.current || migrationPreviewOperationTokenRef.current !== operationToken) return;
      setMigrationPreviewState({ request, preview });
      setMigrationRequiresRestart(false);
    } catch (error) {
      if (mountedRef.current && migrationPreviewOperationTokenRef.current === operationToken) {
        const message = formatError(error);
        setMigrationPreviewState(null);
        setMigrationRequiresRestart(isProfileSecretMigrationRestartRequired(message));
        setMigrationError(profileSecretMigrationErrorMessage(message));
      }
    } finally {
      if (migrationPreviewOperationTokenRef.current === operationToken) {
        migrationPreviewOperationTokenRef.current = null;
        onCredentialOperationFinish(operationToken, false);
        if (mountedRef.current) setMigrationBusy(null);
      }
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
      setMigrationError(t("migration-settings-changed-run-preflight-again"));
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
            applyPortableVaultStatus(next);
            setPortableVaultFeedback({ kind: "status", message: t("migrated-secrets-unlock-stronghold-again", [result.migratedSecretCount]) });
          }
        } catch (lockError) {
          if (mountedRef.current) setPortableVaultFeedback({ kind: "error", message: t("credential-migration-committed-but-stronghold-auto-lock-failed", [formatError(lockError)]) });
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
            : [t("recovery-record-verified-and-cleared")],
        );
        setMigrationRequiresRestart(false);
        setMigrationPreviewState(null);
        if (result.resolved) setMigrationRecoveryError("");
      }
      if (result.pending?.requiresPortableVaultUnlock && portableVault?.unlocked) {
        try {
          const next = await invokeBackend<PortableVaultStatus>("lock_portable_vault", {});
          if (mountedRef.current) {
            applyPortableVaultStatus(next);
            setPortableVaultFeedback({ kind: "status", message: t("recovery-checkpoint-needs-verification-stronghold-was-locked-unlock-it") });
          }
        } catch (lockError) {
          if (mountedRef.current) setPortableVaultFeedback({ kind: "error", message: t("recovery-record-retained-but-stronghold-auto-lock-failed", [formatError(lockError)]) });
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
    const operationToken = onCredentialOperationStart();
    if (operationToken === null) return;
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
      onCredentialOperationFinish(operationToken, false);
      if (mountedRef.current) setMigrationDiagnosticBusy(false);
    }
  }

  async function importKnownHostsText() {
    if (hostKeyMutationBusy || !profileId || !knownHostsText.trim()) return;
    const writeToken = beginHostKeyWrite();
    if (writeToken === null) return;
    const mutationToken = onHostKeyMutationStart();
    const pendingProfileId = profileId;
    const pendingContents = knownHostsText;
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("import_known_hosts", {
        request: { profileId: pendingProfileId, contents: pendingContents },
      });
      const accepted = onChange(nextStore, mutationToken);
      if (!accepted || !mountedRef.current) return;
      setKnownHostsText("");
      setStatus(t("known-hosts-imported-into-the-selected-profile-scope"));
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
      finishHostKeyWrite(writeToken);
    }
  }

  async function exportKnownHostsText() {
    const token = refreshGate.current.begin("known-hosts-export");
    if (token === null) return;
    setKnownHostsExportBusy(true);
    setError("");
    setStatus("");
    try {
      const contents = await invokeBackend<string>("export_known_hosts", {});
      if (refreshGate.current.isCurrent("known-hosts-export", token)) setExportText(contents);
    } catch (error) {
      if (refreshGate.current.isCurrent("known-hosts-export", token)) setError(formatError(error));
    } finally {
      if (refreshGate.current.finish("known-hosts-export", token) && mountedRef.current) {
        setKnownHostsExportBusy(false);
      }
    }
  }

  async function scanSelectedProfileHostKey() {
    if (!selectedProfile) return;
    const token = refreshGate.current.begin("host-scan");
    if (token === null) return;
    const pendingProfile = prepareProfile(selectedProfile);
    const pendingProfileKey = selectedProfileScanKey;
    setHostKeyScanBusy(true);
    setHostKeyScanError("");
    try {
      const scan = await invokeBackend<HostKeyScanResult>("scan_ssh_host_key", {
        request: {
          profile: pendingProfile,
          credentialHandle: null,
        },
      });
      if (!refreshGate.current.isCurrent("host-scan", token)
        || selectedProfileScanKeyRef.current !== pendingProfileKey) return;
      hostKeyScanProfileKeyRef.current = pendingProfileKey;
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
    if (hostKeyMutationBusy
      || !selectedProfile
      || !hostKeyScan
      || hostKeyScanProfileKeyRef.current !== selectedProfileScanKey) return;
    const writeToken = beginHostKeyWrite();
    if (writeToken === null) return;
    const mutationToken = onHostKeyMutationStart();
    const pendingProfile = prepareProfile(selectedProfile);
    const pendingObservation = hostKeyScan.observation;
    refreshGate.current.invalidate("host-scan");
    setHostKeyScanBusy(true);
    setHostKeyScanError("");
    setError("");
    setStatus("");
    try {
      await invokeBackend<TrustedHostKey | null>("trust_scanned_host_key", {
        request: {
          profile: pendingProfile,
          observation: pendingObservation,
          decision,
        },
      });
      const nextStore = await invokeBackend<HostKeyStore>("list_host_keys", {});
      const accepted = onChange(nextStore, mutationToken);
      if (!accepted || !mountedRef.current) return;
      setHostKeyScan(null);
      setStatus(decision === "replace-for-profile" ? t("profile-host-key-replaced") : t("scanned-host-key-added-to-the-trust-store"));
    } catch (error) {
      if (mountedRef.current) setHostKeyScanError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
      if (mountedRef.current) {
        setHostKeyScanBusy(false);
      }
      finishHostKeyWrite(writeToken);
    }
  }

  async function deleteKey(keyId: string) {
    if (hostKeyMutationBusy) return;
    const key = hostKeys.keys.find((item) => item.id === keyId);
    if (!key) return;
    const writeToken = beginHostKeyWrite();
    if (writeToken === null) return;
    const unsavedWarning = editingKeyId === keyId && hostKeyDraftDirty
      ? t("unsaved-changes-in-the-host-key-editor-will-also")
      : "";
    if (!window.confirm(t("delete-host-key", [key.alias, key.port, key.fingerprintSha256, unsavedWarning]))) {
      finishHostKeyWrite(writeToken);
      return;
    }
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
      setStatus(t("host-key-deleted"));
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
      finishHostKeyWrite(writeToken);
    }
  }

  async function deleteSelectedHostKeys() {
    if (hostKeyMutationBusy || !selectedHostKeyIds.length) return;
    const pendingKeyIds = [...selectedHostKeyIds];
    const writeToken = beginHostKeyWrite();
    if (writeToken === null) return;
    const unsavedWarning = editingKeyId && pendingKeyIds.includes(editingKeyId) && hostKeyDraftDirty
      ? t("unsaved-changes-in-the-host-key-editor-will-also")
      : "";
    if (!window.confirm(t("delete-selected-host-keys", [pendingKeyIds.length, unsavedWarning]))) {
      finishHostKeyWrite(writeToken);
      return;
    }
    const mutationToken = onHostKeyMutationStart();
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("delete_host_keys", { keyIds: pendingKeyIds });
      const accepted = onChange(nextStore, mutationToken);
      if (!accepted || !mountedRef.current) return;
      setSelectedHostKeyIds([]);
      setEditingKeyId("");
      setEditDraft(null);
      setStatus(t("deleted-host-keys", [pendingKeyIds.length]));
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
      finishHostKeyWrite(writeToken);
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
    if (hostKeyMutationBusy || !confirmDiscardHostKeyDraft(t("switch-host-key"))) return;
    const baseline: HostKeyEditFields = {
      profileId: key.profileId ?? "",
      alias: key.alias,
      host: key.host,
      port: key.port,
      scope: key.scope,
      label: key.label ?? "",
    };
    setEditingKeyId(key.id);
    setEditDraft({
      keyId: key.id,
      expectedKey: { ...key },
      ...baseline,
      baseline,
    });
    setError("");
    setStatus("");
  }

  async function saveEditedHostKey() {
    if (!editDraft || hostKeyMutationBusy) return;
    const writeToken = beginHostKeyWrite();
    if (writeToken === null) return;
    const pendingDraft = editDraft;
    const mutationToken = onHostKeyMutationStart();
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("update_host_key", {
        request: {
          keyId: pendingDraft.keyId,
          expectedKey: pendingDraft.expectedKey,
          profileId: pendingDraft.profileId || null,
          alias: pendingDraft.alias,
          host: pendingDraft.host,
          port: pendingDraft.port,
          scope: pendingDraft.scope,
          label: pendingDraft.label || null,
        },
      });
      const accepted = onChange(nextStore, mutationToken);
      if (!accepted || !mountedRef.current) return;
      setEditingKeyId("");
      setEditDraft(null);
      setStatus(t("host-key-updated"));
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onHostKeyMutationFinish(mutationToken);
      finishHostKeyWrite(writeToken);
    }
  }

  async function saveProfileFromManager(
    profile: SessionProfile,
    expectedProfile: SessionProfile,
    message: string,
    existingMutationToken?: number,
    existingClientMutationToken?: number,
  ): Promise<{ persisted: boolean; accepted: boolean }> {
    const clientMutationToken = existingClientMutationToken ?? beginClientKeyMutation();
    if (clientMutationToken === null) return { persisted: false, accepted: false };
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
      if (existingClientMutationToken === undefined) finishClientKeyMutation(clientMutationToken);
    }
  }

  async function readPrivateKeyFile(file: File | null) {
    if (!file) return;
    const token = privateKeyFileReadGate.current.replace("private-key-file");
    privateKeyFileReadActive.current = true;
    setPrivateKeyFileReadBusy(true);
    setError("");
    setStatus("");
    setPrivateKeyText("");
    if (file.size > MAX_PRIVATE_KEY_IMPORT_BYTES) {
      setError(t("private-key-file-must-not-exceed", [formatBytes(MAX_PRIVATE_KEY_IMPORT_BYTES)]));
      if (privateKeyFileReadGate.current.finish("private-key-file", token)) {
        privateKeyFileReadActive.current = false;
        setPrivateKeyFileReadBusy(false);
      }
      return;
    }
    try {
      const text = await file.text();
      if (!privateKeyFileReadGate.current.isCurrent("private-key-file", token)) return;
      setPrivateKeyText(text);
      if (!privateKeyLabel.trim()) {
        setPrivateKeyLabel(file.name.replace(/\.(pem|key|txt)$/i, "") || "profile key");
      }
      setStatus(t("read", [file.name]));
    } catch (error) {
      if (privateKeyFileReadGate.current.isCurrent("private-key-file", token)) {
        setError(formatError(error));
      }
    } finally {
      if (privateKeyFileReadGate.current.finish("private-key-file", token)) {
        privateKeyFileReadActive.current = false;
        setPrivateKeyFileReadBusy(false);
      }
    }
  }

  async function importPrivateKeyToProfile() {
    if (privateKeyFileReadActive.current
      || clientKeyControlsDisabled
      || !selectedProfile
      || !isSshLikeProfile(selectedProfile)) return;
    if (!portableVault?.unlocked) {
      setError(t("unlock-stronghold-before-importing-a-private-key"));
      return;
    }
    const profile = selectedProfile;
    const privateKey = privateKeyText.trim();
    if (!privateKey) return;
    if (new TextEncoder().encode(privateKeyText).byteLength > MAX_PRIVATE_KEY_IMPORT_BYTES) {
      setError(t("private-key-content-must-not-exceed", [formatBytes(MAX_PRIVATE_KEY_IMPORT_BYTES)]));
      return;
    }
    if (!privateKey.includes("PRIVATE KEY")) {
      setError(t("the-content-does-not-appear-to-be-an-openssh"));
      return;
    }
    const clientMutationToken = beginClientKeyMutation();
    if (clientMutationToken === null) return;
    const mutationToken = onProfileMutationStart(profile.id);
    let mutationDelegated = false;
    let newSecretRef: string | null = null;
    setError("");
    setStatus("");
    try {
      const label = privateKeyLabel.trim() || "profile key";
      const response = await invokeBackend<{ secretRef: string }>("save_secret", {
        request: { secretRef: null, secret: privateKeyText, storage: "portable" },
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
      }, profile, t("private-key-imported-into", [profile.name]), mutationToken, clientMutationToken);
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
      finishClientKeyMutation(clientMutationToken);
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
      setStatus(t("the-selected-profile-already-contains-these-host-keys"));
      return;
    }
    await saveProfileFromManager({
      ...selectedProfile,
      connection: {
        ...selectedProfile.connection,
        trustedHostKeys: [...copiedKeys, ...currentKeys],
      },
    }, selectedProfile, t("copied-host-keys-to", [copiedKeys.length, selectedProfile.name]));
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
      setStatus(t("already-contains-the-selected-client-keys", [selectedProfile.name]));
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
    }, selectedProfile, t("copied-client-keys-to", [copied, selectedProfile.name]));
    if (saveResult.accepted && mountedRef.current) setSelectedClientKeyIds([]);
  }

  async function moveSelectedClientIdentitiesFirst() {
    if (!selectedClientIdentityItems.length || clientKeyControlsDisabled) return;
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
    const clientMutationToken = beginClientKeyMutation();
    if (clientMutationToken === null) return;
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
        setStatus(t("moved-selected-client-keys-to-the-top-in-profiles", [updatedProfiles]));
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
      finishClientKeyMutation(clientMutationToken);
    }
  }

  async function removeSelectedClientIdentities() {
    if (!selectedClientIdentityItems.length || clientKeyControlsDisabled) return;
    const targets = sshSessions.flatMap((session) => {
      const profile = session.profile;
      if (!isSshLikeProfile(profile)) return [];
      const selected = selectedClientIdentityItems.filter((item) => item.profileId === profile.id);
      const removableItems = selected.filter((item) => !item.jumpInUse);
      return removableItems.length ? [{ profile, removableItems }] : [];
    });
    const removableCount = targets.reduce((count, target) => count + target.removableItems.length, 0);
    const skipped = selectedClientIdentityItems.length - removableCount;
    if (!removableCount) {
      setStatus(t("all-selected-client-keys-are-used-by-jump-hosts"));
      return;
    }
    if (!window.confirm(t("remove-client-identity-references-from-their-profiles", [removableCount, skipped ? t("skip-jump-identities-confirmation", [skipped]) : ""]))) return;
    const clientMutationToken = beginClientKeyMutation();
    if (clientMutationToken === null) return;
    const mutationTokens = new Map(targets.map(({ profile }) => [
      profile.id,
      onProfileMutationStart(profile.id),
    ]));
    const completedProfiles = new Set<string>();
    let superseded = false;
    setError("");
    setStatus("");
    let removed = 0;
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
        setStatus(t("removed-client-key-references", [removed, skipped ? t("skipped-jump-identities-suffix", [skipped]) : ""]));
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
      finishClientKeyMutation(clientMutationToken);
    }
  }

  function toggleClientIdentitySelection(selectionId: string, selected: boolean) {
    setSelectedClientKeyIds((current) => selected
      ? Array.from(new Set([...current, selectionId]))
      : current.filter((id) => id !== selectionId));
  }

  function startEditClientIdentity(item: ClientIdentityItem) {
    if (clientKeyControlsDisabled || !confirmDiscardClientIdentityDraft(t("switch-identity"))) return;
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
        ? t("old-secret-deleted")
        : response.oldSecretShared
          ? t("shared-old-secret-retained")
          : "";
    setStatus(`${message}${suffix}`);
    return true;
  }

  async function saveClientIdentity() {
    const expectedIdentity = clientKeyEditExpectedIdentityRef.current;
    if (!clientKeyEditDraft || !expectedIdentity || clientKeyControlsDisabled) return;
    const clientMutationToken = beginClientKeyMutation();
    if (clientMutationToken === null) return;
    const pendingDraft = clientKeyEditDraft;
    const mutationProfileId = pendingDraft.profileId;
    const mutationToken = onProfileMutationStart(mutationProfileId);
    let backendSucceeded = false;
    setError("");
    setStatus("");
    try {
      const response = await invokeBackend<ClientIdentityMutationResponse>("update_client_identity", {
        request: {
          profileId: pendingDraft.profileId,
          identityId: pendingDraft.identityId,
          expectedIdentity,
          label: pendingDraft.label,
          source: pendingDraft.source,
          fingerprintSha256: pendingDraft.fingerprintSha256 || null,
          path: pendingDraft.path || null,
          secretRef: pendingDraft.secretRef || null,
        },
      });
      backendSucceeded = true;
      applyClientIdentityMutation(response, t("client-identity-updated"), mutationToken);
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onProfileMutationFinish(mutationProfileId, mutationToken, backendSucceeded);
      finishClientKeyMutation(clientMutationToken);
    }
  }

  async function rotateClientIdentity() {
    if (!clientKeyEditDraft || !clientKeyPrivateKey.trim() || clientKeyControlsDisabled) return;
    if (!portableVault?.unlocked) {
      setError(t("unlock-stronghold-before-rotating-a-vault-private-key"));
      return;
    }
    const clientMutationToken = beginClientKeyMutation();
    if (clientMutationToken === null) return;
    const mutationProfileId = clientKeyEditDraft.profileId;
    const mutationToken = onProfileMutationStart(mutationProfileId);
    let backendSucceeded = false;
    setError("");
    setStatus("");
    try {
      const response = await invokeBackend<ClientIdentityMutationResponse>("rotate_client_identity", {
        request: {
          profileId: clientKeyEditDraft.profileId,
          identityId: clientKeyEditDraft.identityId,
          privateKey: clientKeyPrivateKey,
          passphrase: clientKeyPassphrase || null,
          storage: "portable",
        },
      });
      backendSucceeded = true;
      if (applyClientIdentityMutation(response, t("vault-private-key-rotated"), mutationToken)) {
        setClientKeyPrivateKey("");
        setClientKeyPassphrase("");
      }
    } catch (error) {
      if (mountedRef.current) setError(formatError(error));
    } finally {
      onProfileMutationFinish(mutationProfileId, mutationToken, backendSucceeded);
      finishClientKeyMutation(clientMutationToken);
    }
  }

  async function deleteEditedClientIdentity(deleteSecret: boolean) {
    if (!clientKeyEditDraft || !editingClientIdentityItem || editingClientIdentityItem.jumpInUse || clientKeyControlsDisabled) return;
    const action = deleteSecret ? t("remove-this-reference-and-delete-the-unshared-secret") : t("remove-this-identity-reference");
    const unsavedWarning = clientIdentityDraftDirty
      ? t("unsaved-changes-in-the-identity-editor-will-also-be")
      : "";
    if (!window.confirm(`${action}“${editingClientIdentityItem.identity.label}”（${editingClientIdentityItem.profileName}）？${unsavedWarning}`)) return;
    const clientMutationToken = beginClientKeyMutation();
    if (clientMutationToken === null) return;
    const mutationProfileId = clientKeyEditDraft.profileId;
    const mutationToken = onProfileMutationStart(mutationProfileId);
    let backendSucceeded = false;
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
      if (applyClientIdentityMutation(response, t("client-identity-reference-removed"), mutationToken)) {
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
      finishClientKeyMutation(clientMutationToken);
    }
  }

  function toggleAgentIdentitySelection(identity: IdentityRef, selected: boolean) {
    const id = identityStableKey(identity);
    setSelectedAgentKeyIds((current) => selected
      ? Array.from(new Set([...current, id]))
      : current.filter((item) => item !== id));
  }

  function confirmDiscardHostKeyDraft(action: string): boolean {
    return !hostKeyDraftDirty || window.confirm(t("the-host-key-editor-has-unsaved-changes-will-discard", [action]));
  }

  function closeHostKeyEditor() {
    if (hostKeyMutationBusy || !confirmDiscardHostKeyDraft(t("close-editor"))) return;
    setEditingKeyId("");
    setEditDraft(null);
  }

  function confirmDiscardClientIdentityDraft(action: string): boolean {
    return !clientIdentityDraftDirty || window.confirm(t("the-identity-editor-has-unsaved-changes-will-discard-them", [action]));
  }

  function closeClientIdentityEditor() {
    if (clientKeyMutationBusy || !confirmDiscardClientIdentityDraft(t("close-inspector"))) return;
    setEditingClientKeyId("");
    setClientKeyEditDraft(null);
    clientKeyEditExpectedIdentityRef.current = null;
    setClientKeyPrivateKey("");
    setClientKeyPassphrase("");
  }

  function closeDialog() {
    const dirtySections = [
      hostKeyDraftDirty ? t("host-key-draft") : "",
      clientIdentityDraftDirty ? t("identity-draft") : "",
      knownHostsText ? t("known-hosts-import-content") : "",
      privateKeyText ? t("private-key-import-content") : "",
    ].filter(Boolean);
    if (dirtySections.length
      && !window.confirm(t("in-the-key-manager-has-unsaved-changes-closing-will", [dirtySections.join("、")]))) return;
    onClose();
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && closeDialog()}>
      <section className="wind-dialog key-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{t("key-manager")}</strong>
          <button type="button" title={t("close")} aria-label={t("close-key-manager")} onClick={closeDialog}><X size={20} /></button>
        </header>
        <div className="key-content">
          <section className="key-list">
            <div className="key-list-toolbar">
              <select value={keyScopeFilter} disabled={hostKeyMutationBusy} onChange={(event) => setKeyScopeFilter(event.target.value as TrustedHostKey["scope"] | "all")}>
                <option value="all">{t("all-scopes")}</option>
                <option value="profile">{t("ui-profile")}</option>
                <option value="project">{t("ui-project")}</option>
                <option value="user">{t("ui-user")}</option>
              </select>
              <select value={keyProfileFilter} disabled={hostKeyMutationBusy} onChange={(event) => setKeyProfileFilter(event.target.value)}>
                <option value="all">{t("all-profiles")}</option>
                {sshSessions.map((session) => (
                  <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                ))}
              </select>
              <button type="button" onClick={selectVisibleHostKeys} disabled={hostKeyMutationBusy || !visibleHostKeys.length}>{t("select-all-2")}</button>
              <button type="button" onClick={() => setSelectedHostKeyIds([])} disabled={hostKeyMutationBusy || !selectedHostKeyIds.length}>{t("clear")}</button>
            </div>
            <div className="key-batch-actions">
              <span>{selectedHostKeyIds.length}{" "}{t("ui-selected")}</span>
              <button type="button" onClick={() => void copySelectedHostKeysToProfile()} disabled={hostKeyMutationBusy || clientKeyControlsDisabled || !selectedVisibleHostKeys.length || !selectedProfile}>{t("copy-to-profile")}</button>
              <button type="button" onClick={() => void deleteSelectedHostKeys()} disabled={hostKeyMutationBusy || !selectedHostKeyIds.length}>{t("delete")}</button>
            </div>
            {visibleHostKeys.map((key) => (
              <div key={key.id} className="key-row">
                <label className="key-row-select">
                  <input type="checkbox" disabled={hostKeyMutationBusy} checked={selectedHostKeyIds.includes(key.id)} onChange={(event) => toggleHostKeySelection(key.id, event.target.checked)} />
                </label>
                <strong>{key.alias}:{key.port}</strong>
                <span>{key.algorithm} · {key.fingerprintSha256}</span>
                <small>{key.scope} · {key.label ?? key.host}{t("last-verified")}{formatHostKeyDate(key.lastSeen)}</small>
                <div className="key-row-actions">
                  <button onClick={() => startEditKey(key)} disabled={hostKeyMutationBusy}>{t("edit")}</button>
                  <button onClick={() => void copyHostKeyToProfile(key)} disabled={hostKeyMutationBusy || clientKeyControlsDisabled || !selectedProfile}>{t("copy-to-profile")}</button>
                  <button onClick={() => void deleteKey(key.id)} disabled={hostKeyMutationBusy}>{t("delete")}</button>
                </div>
              </div>
            ))}
            {!hostKeys.keys.length ? <div className="empty-pane top">{t("no-saved-host-keys")}</div> : null}
            {hostKeys.keys.length && !visibleHostKeys.length ? <div className="empty-pane top">{t("no-host-keys-in-this-group")}</div> : null}
          </section>
          <section className="key-editor">
            <DialogField label={t("ui-profile-2")}>
              <select value={profileId} disabled={hostKeyMutationBusy || clientKeyMutationBusy} onChange={(event) => setProfileId(event.target.value)}>
                {sshSessions.map((session) => (
                  <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                ))}
              </select>
            </DialogField>
            <section className="host-key-scan-panel" aria-live="polite">
              <header>
                <div><strong>{t("current-host-key")}</strong><small>{selectedProfile ? describeSshProfileTarget(selectedProfile) : t("no-profile-selected")}</small></div>
                <button type="button" onClick={() => void scanSelectedProfileHostKey()} disabled={hostKeyMutationBusy || !selectedProfile || hostKeyScanBusy}>
                  <RefreshCw size={14} className={hostKeyScanBusy ? "loading" : ""} />{hostKeyScanBusy ? t("scanning-2") : t("scan")}
                </button>
              </header>
              {hostKeyScan ? (
                <div className={`host-key-scan-result ${hostKeyScan.evaluation.status}`}>
                  <div className="host-key-scan-status">
                    {hostKeyScan.evaluation.status === "trusted" ? <CheckCircle2 size={15} /> : <AlertCircle size={15} />}
                    <strong>{hostKeyScanStatus(hostKeyScan)}</strong>
                  </div>
                  <dl>
                    <div><dt>{t("target")}</dt><dd>{hostKeyScan.observation.alias || hostKeyScan.observation.host}:{hostKeyScan.observation.port}</dd></div>
                    <div><dt>{t("algorithm")}</dt><dd>{hostKeyScan.observation.algorithm}</dd></div>
                    <div><dt>{t("fingerprint")}</dt><dd>{hostKeyScanFingerprint(hostKeyScan)}</dd></div>
                    {hostKeyScan.evaluation.status === "mismatch" ? <div><dt>{t("saved-2")}</dt><dd>{hostKeyScan.evaluation.expected.map((key) => key.fingerprintSha256).join(" · ")}</dd></div> : null}
                  </dl>
                  {hostKeyScan.evaluation.status !== "trusted" ? (
                    <div className="host-key-scan-actions">
                      <button type="button" onClick={() => void trustHostKeyScan("append-to-profile")} disabled={hostKeyMutationBusy || hostKeyScanBusy}>{t("add-to-profile")}</button>
                      <button type="button" onClick={() => void trustHostKeyScan("append-to-project")} disabled={hostKeyMutationBusy || hostKeyScanBusy}>{t("add-to-project")}</button>
                      {hostKeyScan.evaluation.status === "mismatch" ? <button type="button" className="danger" onClick={() => void trustHostKeyScan("replace-for-profile")} disabled={hostKeyMutationBusy || hostKeyScanBusy}>{t("replace-profile")}</button> : null}
                    </div>
                  ) : null}
                </div>
              ) : null}
              {hostKeyScanError ? <div className="host-key-scan-error">{localizeDiagnostic(hostKeyScanError)}</div> : null}
            </section>
            {editDraft ? (
              <section className="key-edit-panel">
                <div className="key-edit-heading">
                  <strong>{t("ui-host-key")}</strong>
                  <button type="button" disabled={hostKeyMutationBusy} onClick={closeHostKeyEditor}>{t("close")}</button>
                </div>
                <DialogField label={t("ui-alias")}>
                  <input value={editDraft.alias} disabled={hostKeyMutationBusy} onChange={(event) => setEditDraft({ ...editDraft, alias: event.target.value })} />
                </DialogField>
                <DialogField label={t("ui-host")}>
                  <input value={editDraft.host} disabled={hostKeyMutationBusy} onChange={(event) => setEditDraft({ ...editDraft, host: event.target.value })} />
                </DialogField>
                <DialogField label={t("port-3")}>
                  <input type="number" min={1} max={65535} value={editDraft.port} disabled={hostKeyMutationBusy} onChange={(event) => setEditDraft({ ...editDraft, port: Number(event.target.value) || 22 })} />
                </DialogField>
                <DialogField label={t("ui-scope")}>
                  <select value={editDraft.scope} disabled={hostKeyMutationBusy} onChange={(event) => setEditDraft({ ...editDraft, scope: event.target.value as TrustedHostKey["scope"] })}>
                    <option value="profile">{t("ui-profile")}</option>
                    <option value="project">{t("ui-project")}</option>
                    <option value="user">{t("ui-user")}</option>
                  </select>
                </DialogField>
                <DialogField label={t("ui-profile-2")}>
                  <select value={editDraft.profileId} disabled={hostKeyMutationBusy} onChange={(event) => setEditDraft({ ...editDraft, profileId: event.target.value })}>
                    <option value="">{t("none")}</option>
                    {sshSessions.map((session) => (
                      <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                    ))}
                  </select>
                </DialogField>
                <DialogField label={t("ui-label")}>
                  <input value={editDraft.label} disabled={hostKeyMutationBusy} onChange={(event) => setEditDraft({ ...editDraft, label: event.target.value })} />
                </DialogField>
                <div className="key-edit-meta">
                  <span>{editingKey?.algorithm ?? ""}</span>
                  <span>{editingKey?.fingerprintSha256 ?? ""}</span>
                  <span>{t("first-last", [formatHostKeyDate(editingKey?.firstSeen), formatHostKeyDate(editingKey?.lastSeen)])}</span>
                </div>
                <div className="key-actions">
                  <button type="button" onClick={() => void saveEditedHostKey()} disabled={hostKeyMutationBusy}>{t("save-changes")}</button>
                </div>
              </section>
            ) : null}
            <DialogField label={t("ui-known-hosts")}>
              <textarea value={knownHostsText} disabled={hostKeyMutationBusy} onChange={(event) => setKnownHostsText(event.target.value)} placeholder={t("paste-openssh-known-hosts-content")} />
            </DialogField>
            {error ? <div className="utility-error">{localizeDiagnostic(error)}</div> : null}
            {status ? <div className="utility-status">{status}</div> : null}
            <div className="key-actions">
              <button onClick={() => void importKnownHostsText()} disabled={hostKeyMutationBusy || !profileId || !knownHostsText.trim()}>{t("import")}</button>
              <button onClick={() => void exportKnownHostsText()} disabled={hostKeyMutationBusy || knownHostsExportBusy}>{knownHostsExportBusy ? t("exporting") : t("export")}</button>
            </div>
            {exportText ? (
              <textarea className="key-export" value={exportText} onChange={(event) => setExportText(event.target.value)} />
            ) : null}
          </section>
          <section className="key-agent-list">
            <div className="key-agent-header">
              <span><KeyRound size={15} /><strong>{t("ui-client-keys")}</strong></span>
              <small>{clientIdentityItems.length}{" "}{t("ui-identities")}</small>
            </div>
            <section
              className={`portable-vault-panel${portableVault?.unlocked ? " unlocked" : ""}`}
              title={portableVault?.path ?? "Stronghold"}
              aria-labelledby="portable-vault-title"
            >
              <div className="portable-vault-bar" data-vault-state={portableVault?.unlocked ? "unlocked" : portableVault?.exists ? "locked" : "not-created"}>
                <span className={portableVault?.unlocked ? "unlocked" : ""}>
                  {portableVault?.unlocked ? <Unlock size={14} /> : <Lock size={14} />}
                  <strong id="portable-vault-title">Stronghold</strong>
                  <small>{portableVault?.unlocked ? t("unlocked") : portableVault?.exists ? t("locked") : portableVault ? t("not-created") : t("checking")}</small>
                </span>
                {portableVault?.unlocked ? (
                  <button className="key-icon-button" type="button" title={t("lock-portable-vault")} aria-label={t("lock-portable-vault")} onClick={() => void lockPortableVault()} disabled={vaultOperationBusy}><Lock size={14} /></button>
                ) : portableVault?.exists ? (
                  <div className="portable-vault-unlock-actions">
                    <input type="password" aria-label={t("stronghold-master-password")} autoComplete="current-password" value={portableVaultPassword} onChange={(event) => setPortableVaultPassword(event.target.value)} placeholder={t("enter-the-master-password-to-unlock")} disabled={vaultOperationBusy} onKeyDown={(event) => { if (event.key === "Enter") void unlockPortableVault(); }} />
                    <button className="key-icon-button" type="button" title={t("unlock-portable-vault")} aria-label={t("unlock-portable-vault")} onClick={() => void unlockPortableVault()} disabled={vaultOperationBusy || !portableVaultPassword}><Unlock size={14} /></button>
                  </div>
                ) : null}
              </div>
              {!portableVault ? (
                <div className="portable-vault-loading" role="status">{t("reading-stronghold-status")}</div>
              ) : !portableVault.exists ? (
                <div className="portable-vault-create" aria-live="polite">
                  <div className="portable-vault-create-copy">
                    <strong>{t("create-stronghold-vault")}</strong>
                    <span>{t("securely-stores-ssh-passwords-private-key-passphrases-and-onekeys")}</span>
                  </div>
                  <label>
                    <span>{t("new-master-password")}</span>
                    <input type="password" aria-label={t("new-stronghold-master-password")} autoComplete="new-password" value={portableVaultPassword} onChange={(event) => setPortableVaultPassword(event.target.value)} placeholder={t("at-least-8-characters")} disabled={vaultOperationBusy} />
                  </label>
                  <label>
                    <span>{t("confirm-master-password")}</span>
                    <input type="password" aria-label={t("confirm-stronghold-master-password")} autoComplete="new-password" value={portableVaultCreateConfirmPassword} onChange={(event) => setPortableVaultCreateConfirmPassword(event.target.value)} placeholder={t("re-enter-the-master-password")} disabled={vaultOperationBusy} onKeyDown={(event) => { if (event.key === "Enter") void createPortableVault(); }} />
                  </label>
                  <button type="button" className="portable-vault-create-button" onClick={() => void createPortableVault()} disabled={vaultOperationBusy || !portableVaultPassword || !portableVaultCreateConfirmPassword}>
                    <KeyRound size={14} />{portableVaultBusy ? t("creating") : t("create-stronghold")}
                  </button>
                  <small className="portable-vault-create-note">{t("after-creation-the-vault-is-unlocked-and-ready-to")}</small>
                </div>
              ) : null}
            </section>
            {portableVaultFeedback ? <div className={`portable-vault-feedback ${portableVaultFeedback.kind}`} role={portableVaultFeedback.kind === "error" ? "alert" : "status"} aria-live="polite">{portableVaultFeedback.message}</div> : null}
            {portableVault?.unlocked ? (
              <details className="portable-vault-rotation" onToggle={(event) => { if (!event.currentTarget.open) { clearPortableVaultRotation(); setPortableVaultFeedback(null); } }}>
                <summary><RefreshCw size={14} /><span>{t("change-master-password")}</span></summary>
                <div className="portable-vault-rotation-fields">
                  <label><span>{t("current-master-password")}</span><input type="password" autoComplete="current-password" value={portableVaultCurrentPassword} onChange={(event) => setPortableVaultCurrentPassword(event.target.value)} disabled={credentialMutationControlsDisabled} /></label>
                  <label><span>{t("new-master-password")}</span><input type="password" autoComplete="new-password" value={portableVaultNewPassword} onChange={(event) => setPortableVaultNewPassword(event.target.value)} disabled={credentialMutationControlsDisabled} /></label>
                  <label><span>{t("confirm-new-master-password")}</span><input type="password" autoComplete="new-password" value={portableVaultConfirmPassword} onChange={(event) => setPortableVaultConfirmPassword(event.target.value)} disabled={credentialMutationControlsDisabled} onKeyDown={(event) => { if (event.key === "Enter") void rotatePortableVaultPassword(); }} /></label>
                  <button type="button" onClick={() => void rotatePortableVaultPassword()} disabled={credentialMutationControlsDisabled || !portableVaultCurrentPassword || !portableVaultNewPassword || !portableVaultConfirmPassword}><RefreshCw size={14} />{t("change-master-password")}</button>
                </div>
              </details>
            ) : null}
            {migrationRecovery || migrationRecoveryStatusError || migrationRecoveryError || migrationRecoveryWarnings.length ? (
              <section className={`portable-vault-migration-recovery${migrationRecovery?.disposition === "conflict" ? " conflict" : ""}`} aria-live="polite">
                <header>
                  <span>{migrationRecovery || migrationRecoveryStatusError ? <AlertCircle size={15} /> : <CheckCircle2 size={15} />}<strong>{migrationRecovery ? t("credential-migration-awaiting-recovery") : migrationRecoveryStatusError ? t("unable-to-verify-credential-migration-status") : t("credential-migration-recovery-completed")}</strong></span>
                  {migrationRecovery ? <small>{t(migrationRecoveryDispositionLabels[migrationRecovery.disposition])}</small> : null}
                </header>
                {migrationRecovery ? (
                  <>
                    <dl>
                      <div><dt>{t("stage")}</dt><dd>{t(migrationRecoveryStateLabels[migrationRecovery.state])}</dd></div>
                      <div><dt>{t("ui-profile")}</dt><dd>{migrationRecovery.profileCount}</dd></div>
                      <div><dt>{t("ui-secret")}</dt><dd>{migrationRecovery.secretCount}</dd></div>
                    </dl>
                    <p>{migrationRecovery.message}</p>
                    {migrationRecovery.requiresPortableVaultUnlock ? <p className="portable-vault-migration-recovery-unlock"><Lock size={13} />{t("lock-and-unlock-stronghold-again-first")}</p> : null}
                    {migrationRecovery.disposition === "conflict" || migrationRecovery.state === "needs-resolution"
                      ? <p className="portable-vault-migration-recovery-manual">{t("automatic-recovery-stopped-verify-profile-references-and-both-providers")}</p>
                      : migrationRecoveryStatusError
                        ? null
                        : <button type="button" onClick={() => void recoverPendingProfileSecretMigration()} disabled={migrationRecoveryChecking || !canRecoverProfileSecretMigration(migrationRecovery, portableVault?.unlocked ?? false, vaultOperationBusy || migrationRequiresRestart)}><RefreshCw size={14} />{migrationRecoveryBusy ? t("verifying-2") : t("verify-and-recover")}</button>}
                  </>
                ) : null}
                {migrationRecovery || migrationRecoveryStatusError ? <button className="portable-vault-migration-diagnostic-button" type="button" onClick={() => void exportPendingProfileSecretMigrationDiagnostics()} disabled={migrationRecoveryChecking || vaultOperationBusy}><FileText size={14} />{migrationDiagnosticBusy ? t("exporting") : t("export-diagnostics")}</button> : null}
                {migrationDiagnosticResult ? <p className="portable-vault-migration-diagnostic-result" title={migrationDiagnosticResult.path}>{t("diagnostics-exported-sha-256", [migrationDiagnosticResult.path, formatBytes(migrationDiagnosticResult.size), migrationDiagnosticResult.sha256.slice(0, 16)])}</p> : null}
                {migrationDiagnosticResult ? <button type="button" onClick={() => void navigator.clipboard?.writeText(`${migrationDiagnosticResult.path}\n${migrationDiagnosticResult.checksumPath}\nSHA-256 ${migrationDiagnosticResult.sha256}`).catch(() => {})}><Copy size={14} />{t("copy-export-details")}</button> : null}
                {migrationRecoveryWarnings.map((warning) => <p className="portable-vault-migration-recovery-warning" key={warning}>{warning}</p>)}
                {migrationRecoveryStatusError ? <p className="portable-vault-migration-recovery-error" role="alert">{t("failed-to-read-status", [migrationRecoveryStatusError])}</p> : null}
                {migrationRecoveryStatusError ? <button type="button" onClick={() => void refreshMigrationRecovery()} disabled={migrationRecoveryChecking || vaultOperationBusy}><RefreshCw size={14} />{migrationRecoveryChecking ? t("loading") : t("reload")}</button> : null}
                {migrationRecoveryError ? <p className="portable-vault-migration-recovery-error" role="alert">{localizeDiagnostic(migrationRecoveryError)}</p> : null}
              </section>
            ) : null}
            {portableVault?.unlocked || migrationResult || migrationError || migrationRecovery ? (
              <details className="portable-vault-migration">
                <summary><ArrowRightLeft size={14} /><span>{t("migrate-profile-credentials")}</span></summary>
                {portableVault?.unlocked && !migrationRecovery ? (
                  <>
                    <div className="portable-vault-migration-config">
                      <div className="portable-vault-migration-direction" role="group" aria-label={t("credential-migration-direction")}>
                        <span>{t("system-keyring-stronghold")}</span>
                      </div>
                      <label><span>{t("profile-scope")}</span><select value={migrationScopeProfileId} onChange={(event) => { setMigrationScopeProfileId(event.target.value); invalidateMigrationState(); }} disabled={migrationControlsDisabled}><option value="all">{t("all-credential-profiles")}</option>{credentialSessions.map((session) => <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>)}</select></label>
                      <label className="portable-vault-migration-cleanup"><input type="checkbox" checked={migrationCleanupSource} onChange={(event) => { setMigrationCleanupSource(event.target.checked); invalidateMigrationState(); }} disabled={migrationControlsDisabled} /><span>{t("delete-unshared-source-secrets")}</span></label>
                      <button className="portable-vault-migration-preview-button" type="button" onClick={() => void previewProfileSecretMigration()} disabled={migrationControlsDisabled || !credentialSessions.length}><RefreshCw size={14} />{migrationBusy === "preview" ? t("running-preflight") : t("preflight")}</button>
                    </div>
                    {migrationPreviewState ? (
                      <div className="portable-vault-migration-preview" role="status" aria-live="polite">
                        <dl>
                          <div><dt>{t("ui-profile")}</dt><dd>{migrationPreviewState.preview.affectedProfileCount}/{migrationPreviewState.preview.selectedProfileCount}</dd></div>
                          <div><dt>{t("references")}</dt><dd>{migrationPreviewState.preview.eligibleReferenceCount}</dd></div>
                          <div><dt>{t("ui-secret")}</dt><dd>{migrationPreviewState.preview.eligibleSecretCount}</dd></div>
                          <div><dt>{t("shared-and-retained")}</dt><dd>{migrationPreviewState.preview.retainedSharedSecretCount}</dd></div>
                        </dl>
                        {migrationPreviewState.preview.alreadyTargetReferenceCount ? <p>{t("references-are-already-in-the-target-store", [migrationPreviewState.preview.alreadyTargetReferenceCount])}</p> : null}
                        {migrationPreviewState.preview.retainedInFlightSecretCount ? <p>{t("source-secrets-retained-for-in-progress-connections", [migrationPreviewState.preview.retainedInFlightSecretCount])}</p> : null}
                        {migrationPreviewState.preview.excludedReservedReferenceCount ? <p>{t("reserved-mcp-token-references-excluded", [migrationPreviewState.preview.excludedReservedReferenceCount])}</p> : null}
                        <button type="button" onClick={() => void migrateProfileSecrets()} disabled={!canExecuteProfileSecretMigration(migrationPreviewState.preview, portableVault.unlocked, migrationControlsDisabled, Boolean(migrationRecovery))}><ArrowRightLeft size={14} />{migrationBusy === "migrate" ? t("migrating") : migrationPreviewState.preview.eligibleSecretCount ? t("confirm-migration") : t("no-migration-needed")}</button>
                      </div>
                    ) : null}
                  </>
                ) : null}
                {migrationResult && migrationCleanupSummary ? (
                  <div className="portable-vault-migration-result" role="status" aria-live="polite">
                    <strong>{t("profiles-references-secrets", [migrationResult.migratedProfileCount, migrationResult.migratedReferenceCount, migrationResult.migratedSecretCount])}</strong>
                    <span>{t("source-cleanup-deleted-shared-connecting-retained-by-settings-failed", [migrationCleanupSummary.deleted, migrationCleanupSummary["retained-shared"], migrationCleanupSummary["retained-in-use"], migrationCleanupSummary["retained-by-request"], migrationCleanupSummary.failed])}</span>
                    {migrationResult.warnings.map((warning) => <p key={warning}>{warning}</p>)}
                  </div>
                ) : null}
                {migrationError ? <div className="portable-vault-migration-error" role="alert">{localizeDiagnostic(migrationError)}</div> : null}
              </details>
            ) : null}
            <div className="client-key-filters">
              <label className="client-key-search">
                <Search size={14} />
                <input value={clientKeyQuery} onChange={(event) => setClientKeyQuery(event.target.value)} placeholder={t("search-label-fingerprint-or-path")} />
              </label>
              <select value={clientKeySourceFilter} onChange={(event) => setClientKeySourceFilter(event.target.value as IdentityRef["source"] | "all")} aria-label={t("client-key-source")}>
                <option value="all">{t("all-sources")}</option>
                <option value="profile-vault">{t("ui-profile-vault")}</option>
                <option value="system-file">{t("ui-system-file")}</option>
                <option value="agent">{t("ssh-agent")}</option>
                <option value="public-key-only">{t("public-key")}</option>
              </select>
              <select value={clientKeyProfileFilter} onChange={(event) => setClientKeyProfileFilter(event.target.value)} aria-label={t("ui-client-key-profile")}>
                <option value="all">{t("all-profiles-2")}</option>
                {sshSessions.map((session) => (
                  <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                ))}
              </select>
              <select value={clientKeyGroupBy} onChange={(event) => setClientKeyGroupBy(event.target.value as ClientIdentityGroupBy)} aria-label={t("client-key-grouping")}>
                <option value="profile">{t("group-by-profile")}</option>
                <option value="source">{t("group-by-source")}</option>
              </select>
            </div>
            <div className="client-key-batch">
              <span>{selectedClientIdentityItems.length}{" "}{t("ui-selected")}</span>
              <button type="button" onClick={() => setSelectedClientKeyIds((current) => Array.from(new Set([...current, ...visibleClientIdentityItems.map((item) => item.selectionId)])))} disabled={clientKeyControlsDisabled || !visibleClientIdentityItems.length}>{t("select-all-results")}</button>
              <button type="button" onClick={() => setSelectedClientKeyIds([])} disabled={clientKeyControlsDisabled || !selectedClientKeyIds.length}>{t("clear")}</button>
              <div className="client-key-command-group">
                <button className="key-icon-button" type="button" title={t("copy-to", [selectedProfile?.name ?? "Profile"])} aria-label={t("copy-to", [selectedProfile?.name ?? "Profile"])} onClick={() => void copyClientIdentitiesToProfile(selectedClientIdentityItems)} disabled={clientKeyControlsDisabled || !selectedClientIdentityItems.length || !selectedProfile}><Copy size={15} /></button>
                <button className="key-icon-button" type="button" title={t("move-to-top-in-each-profile")} aria-label={t("move-to-top-in-each-profile")} onClick={() => void moveSelectedClientIdentitiesFirst()} disabled={clientKeyControlsDisabled || !selectedClientIdentityItems.length}><ArrowUp size={15} /></button>
                <button className="key-icon-button danger" type="button" title={t("remove-references-from-their-profiles")} aria-label={t("remove-references-from-their-profiles")} onClick={() => void removeSelectedClientIdentities()} disabled={clientKeyControlsDisabled || !selectedClientIdentityItems.length}><Trash2 size={15} /></button>
              </div>
            </div>
            <div className="client-key-groups">
              {clientIdentityGroups.map((group) => (
                <section key={group.id} className="client-key-group">
                  <header><strong>{group.label}</strong><span>{group.items.length}</span></header>
                  {group.items.map((item) => (
                    <div key={item.selectionId} className={`client-key-row${item.jumpInUse ? " in-use" : ""}${editingClientKeyId === item.selectionId ? " editing" : ""}`}>
                      <input type="checkbox" disabled={clientKeyControlsDisabled} checked={selectedClientKeyIds.includes(item.selectionId)} onChange={(event) => toggleClientIdentitySelection(item.selectionId, event.target.checked)} />
                      <span className="client-key-main">
                        <strong title={item.identity.label}>{item.identity.label}</strong>
                        <code title={item.identity.fingerprintSha256 ?? item.identity.path ?? item.identity.id}>{item.identity.fingerprintSha256 ?? item.identity.path ?? t("unknown-fingerprint")}</code>
                      </span>
                      <span className="client-key-meta">
                        <span>{identitySourceLabel(item.identity.source)}</span>
                        {clientKeyGroupBy === "source" ? <span>{item.profileName}</span> : null}
                        {item.jumpInUse ? <span className="client-key-in-use">{t("used-by-jump-host")}</span> : null}
                      </span>
                      <button className="key-icon-button client-key-edit-button" type="button" title={t("edit-client-identity")} aria-label={t("edit-2", [item.identity.label])} disabled={clientKeyControlsDisabled} onClick={() => startEditClientIdentity(item)}><Pencil size={14} /></button>
                    </div>
                  ))}
                </section>
              ))}
              {!clientIdentityItems.length ? <div className="empty-pane top">{t("this-profile-has-no-client-identities")}</div> : null}
              {clientIdentityItems.length && !visibleClientIdentityItems.length ? <div className="empty-pane top">{t("no-client-identities-match-the-current-filter")}</div> : null}
            </div>
            {clientKeyEditDraft && editingClientIdentityItem ? (
              <section className="client-key-inspector">
                <header>
                  <span><Pencil size={14} /><strong>{t("ui-identity-inspector")}</strong></span>
                  <button className="key-icon-button" type="button" title={t("close-inspector")} aria-label={t("close-identity-inspector")} disabled={clientKeyMutationBusy} onClick={closeClientIdentityEditor}><X size={14} /></button>
                </header>
                <div className="client-key-inspector-grid">
                  <label><span>{t("ui-label-2")}</span><input value={clientKeyEditDraft.label} disabled={clientKeyControlsDisabled} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, label: event.target.value })} /></label>
                  <label><span>{t("ui-source")}</span><select value={clientKeyEditDraft.source} disabled={clientKeyControlsDisabled} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, source: event.target.value as IdentityRef["source"] })}><option value="profile-vault">{t("ui-profile-vault")}</option><option value="system-file">{t("ui-system-file")}</option><option value="agent">{t("ssh-agent")}</option><option value="public-key-only">{t("public-key")}</option></select></label>
                  <label><span>{t("fingerprint")}</span><input value={clientKeyEditDraft.fingerprintSha256} disabled={clientKeyControlsDisabled} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, fingerprintSha256: event.target.value })} placeholder="SHA256:..." /></label>
                  <label><span>{t("ui-path-agent-comment")}</span><input value={clientKeyEditDraft.path} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, path: event.target.value })} disabled={clientKeyControlsDisabled || clientKeyEditDraft.source === "profile-vault"} /></label>
                  <label><span>{t("ui-identity-id")}</span><input value={clientKeyEditDraft.identityId} readOnly /></label>
                  <label><span>{t("ui-profile")}</span><input value={editingClientIdentityItem.profileName} readOnly /></label>
                  {clientKeyEditDraft.source === "profile-vault" ? <label><span>{t("ui-rotation-storage")}</span><input value="Stronghold" readOnly /></label> : null}
                  {clientKeyEditDraft.source === "profile-vault" ? <label className="client-key-secret-ref"><span>{t("ui-secret-ref")}</span><input value={clientKeyEditDraft.secretRef} readOnly /></label> : null}
                </div>
                <div className="client-key-impact">
                  <span>{editingClientIdentityItem.jumpInUse ? t("used-by-jump-host") : t("not-used-by-a-jump-host")}</span>
                  {editingClientSecretUsage > 1 ? <span>{t("identities-share-this-secret", [editingClientSecretUsage])}</span> : <span>{editingClientSecretUsage ? t("secret-is-not-shared") : t("no-secret")}</span>}
                </div>
                <div className="client-key-inspector-actions">
                  <button type="button" onClick={() => void saveClientIdentity()} disabled={clientKeyControlsDisabled}>{t("save-fields")}</button>
                  <button className="danger" type="button" onClick={() => void deleteEditedClientIdentity(false)} disabled={clientKeyControlsDisabled || editingClientIdentityItem.jumpInUse}>{t("remove-reference")}</button>
                  {editingClientIdentityItem.identity.secretRef ? <button className="danger" type="button" onClick={() => void deleteEditedClientIdentity(true)} disabled={clientKeyControlsDisabled || editingClientIdentityItem.jumpInUse}>{t("remove-and-delete-secret")}</button> : null}
                </div>
                {clientKeyEditDraft.source === "profile-vault" ? (
                  <div className="client-key-rotation">
                    <textarea value={clientKeyPrivateKey} disabled={clientKeyControlsDisabled} onChange={(event) => setClientKeyPrivateKey(event.target.value)} placeholder={t("new-openssh-private-key")} />
                    <input type="password" value={clientKeyPassphrase} disabled={clientKeyControlsDisabled} onChange={(event) => setClientKeyPassphrase(event.target.value)} placeholder={t("new-private-key-passphrase-optional")} />
                    <button type="button" onClick={() => void rotateClientIdentity()} disabled={clientKeyControlsDisabled || !portableVault?.unlocked || !clientKeyPrivateKey.trim()}><RefreshCw size={14} />{t("rotate-vault-private-key")}</button>
                  </div>
                ) : null}
              </section>
            ) : null}
            <details className="key-import-panel" aria-busy={privateKeyFileReadBusy}>
              <summary><Plus size={14} />{t("import-private-key-into")}{selectedProfile?.name ?? "Profile"}</summary>
              <input value={privateKeyLabel} disabled={privateKeyImportControlsDisabled} onChange={(event) => setPrivateKeyLabel(event.target.value)} placeholder={t("ui-key-label")} />
              <input type="file" accept=".pem,.key,.txt" disabled={clientKeyControlsDisabled} onChange={(event) => {
                void readPrivateKeyFile(event.currentTarget.files?.[0] ?? null);
                event.currentTarget.value = "";
              }} />
              <input value="storage-stronghold-unlock-first" aria-label={t("private-key-storage")} readOnly />
              <textarea value={privateKeyText} maxLength={MAX_PRIVATE_KEY_IMPORT_BYTES} disabled={privateKeyImportControlsDisabled} onChange={(event) => {
                const value = event.target.value;
                if (new TextEncoder().encode(value).byteLength > MAX_PRIVATE_KEY_IMPORT_BYTES) {
                  setError(t("private-key-content-must-not-exceed", [formatBytes(MAX_PRIVATE_KEY_IMPORT_BYTES)]));
                  return;
                }
                setPrivateKeyText(value);
              }} placeholder={t("paste-openssh-private-key")} />
              <button onClick={() => void importPrivateKeyToProfile()} disabled={privateKeyImportControlsDisabled || !portableVault?.unlocked || !selectedProfile || !privateKeyText.trim()}>{t("import-into-profile")}</button>
            </details>
            <div className="key-agent-header agent-section-header">
              <span><strong>{t("ui-agent-keys")}</strong><small>{agentKeys.length}{" "}{t("ui-visible")}</small></span>
              <button onClick={() => void refreshAgentKeys()}>{t("refresh")}</button>
            </div>
            <div className="client-key-batch agent-key-batch">
              <span>{selectedAgentKeys.length}{" "}{t("ui-selected")}</span>
              <button type="button" onClick={() => setSelectedAgentKeyIds(agentKeys.map(identityStableKey))} disabled={clientKeyControlsDisabled || !agentKeys.length}>{t("select-all-2")}</button>
              <button type="button" onClick={() => setSelectedAgentKeyIds([])} disabled={clientKeyControlsDisabled || !selectedAgentKeyIds.length}>{t("clear")}</button>
              <button className="key-icon-button" type="button" title={t("batch-add-to", [selectedProfile?.name ?? "Profile"])} aria-label={t("batch-add-to", [selectedProfile?.name ?? "Profile"])} onClick={() => void copyAgentIdentitiesToProfile(selectedAgentKeys)} disabled={clientKeyControlsDisabled || !selectedAgentKeys.length || !selectedProfile}><UserPlus size={15} /></button>
            </div>
            <div className="agent-key-list">
              {agentKeys.map((identity, index) => (
                <div key={`${identityStableKey(identity)}:${index}`} className="client-key-row agent-row">
                  <input type="checkbox" disabled={clientKeyControlsDisabled} checked={selectedAgentKeyIds.includes(identityStableKey(identity))} onChange={(event) => toggleAgentIdentitySelection(identity, event.target.checked)} />
                  <span className="client-key-main">
                    <strong title={identity.label}>{identity.label}</strong>
                    <code title={identity.fingerprintSha256 ?? ""}>{identity.fingerprintSha256 ?? t("unknown-fingerprint")}</code>
                  </span>
                  <span className="client-key-meta"><span>{identity.path ?? "ssh-agent"}</span></span>
                  <button className="key-icon-button" type="button" title={t("add-to", [selectedProfile?.name ?? "Profile"])} aria-label={t("add-to-2", [identity.label, selectedProfile?.name ?? "Profile"])} onClick={() => void copyAgentIdentityToProfile(identity)} disabled={clientKeyControlsDisabled || !selectedProfile}><UserPlus size={15} /></button>
                </div>
              ))}
              {!agentKeys.length ? <div className="empty-pane top">{t("no-visible-ssh-agent-identities")}</div> : null}
            </div>
          </section>
        </div>
      </section>
    </div>
  );
}

function DialogField({ label, children }: { label: string; children: ReactNode }) {
  useLocale();
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
      return t("matches-a-trusted-host-key");
    case "unknown":
      return t("this-host-key-is-not-yet-trusted");
    case "mismatch":
      return t("host-key-does-not-match-the-saved-record");
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
      return t("ui-profile-vault");
    case "system-file":
      return t("ui-system-file");
    case "agent":
      return t("ssh-agent");
    case "public-key-only":
      return t("public-key");
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
