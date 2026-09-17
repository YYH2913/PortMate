import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MutableRefObject, ReactNode } from "react";
import {
  Activity,
  Cable,
  CircleAlert,
  FolderTree,
  Layers3,
  Lock,
  Network,
  PlugZap,
  Plus,
  RefreshCw,
  Save,
  Search,
  Server,
  SlidersHorizontal,
  SquareTerminal,
  Trash2,
  Usb,
  X,
} from "lucide-react";
import { invokeBackend } from "./api";
import { hostKeyProfileRequestKey } from "./host-key-profile-state";
import { sshHealthProfileRequestKey } from "./ssh-health-profile-state";
import { KeyedRequestGate } from "./keyed-request-gate";
import type { ProxyPasswordUpdate } from "./proxy-settings";
import ShellArgumentsEditor from "./ShellArgumentsEditor";
import {
  convertDraftProtocol,
  createDefaultTrigger,
  createIdentityRef,
  createJumpHostKeyPolicy,
  createSerialConnection,
  createShellConnection,
  createSshConnection,
  createTcpConnection,
  describeHostKeyEvaluation,
  profileCredentialSecretRefs,
  protocolFromKind,
  serialPortOptions,
  chooseSshPrivateKeyPath,
} from "./session-profile-helpers";
import {
  filterSessionTree,
  flattenSessionTree,
  MAX_SESSION_PROFILE_GROUP_CHARACTERS,
  MAX_SESSION_PROFILE_NAME_CHARACTERS,
  MAX_SESSION_PROFILE_TAG_INPUT_CHARACTERS,
  normalizeSessionMetadataText,
  protocolSettingsSection,
  protocolTabs,
  removeJumpSecretDraftIndex,
  sessionSectionLabel,
  sessionSettingTrees,
  validateQuickConnectProfile,
} from "./session-settings-state";
import type { ProtocolTab, QuickConnectField, QuickConnectIssue } from "./session-settings-state";
import { COMMON_SERIAL_BAUD_RATES, serialConnectionBounds } from "./serial-connection-settings";
import { SSH_AUTH_ORDER_OPTIONS, sshConnectionBounds } from "./ssh-connection-settings";
import { tcpConnectionBounds } from "./tcp-connection-settings";
import {
  MAX_TERMINAL_FONT_FAMILY_CHARACTERS,
  MAX_TERMINAL_NAME_BYTES,
  TERMINAL_PROFILE_BOUNDS,
} from "./terminal-settings-state";
import { normalizeTerminalTheme, TERMINAL_THEME_OPTIONS } from "./terminal-theme";
import {
  canAddTrigger,
  canAddTriggerAction,
  defaultTriggerAction,
  MAX_TRIGGER_ACTION_VALUE_CHARACTERS,
  MAX_TRIGGER_LABEL_CHARACTERS,
  MAX_TRIGGER_MATCHER_CHARACTERS,
  patchTriggerAction,
  triggerActionValue,
} from "./trigger-state";
import type {
  AuthMethod,
  HostKeyPolicy,
  HostKeyScanResult,
  IdentityRef,
  JumpHop,
  ProxyConfig,
  SessionProfile,
  SessionSummary,
  SshHealthReport,
  TriggerAction,
  TriggerSpec,
  TrustedHostKey,
} from "./types";

type DraftHostKeyDecisionValue = "append-to-profile" | "replace-for-profile";
type DraftHostKeyDecisionResponse = {
  trusted: TrustedHostKey;
  trustedHostKeys: TrustedHostKey[];
};

export default function SessionSettingsDialog({
  draft,
  mode,
  prepareProfile,
  serialPorts,
  onRefreshSerialPorts,
  initialSection,
  onDraftChange,
  onSave,
  onConnect,
  onOpenClientKeyManager,
  onExportProfile,
  onClose,
}: {
  draft: SessionProfile;
  mode: "create" | "edit";
  prepareProfile: (profile: SessionProfile) => SessionProfile;
  serialPorts: string[];
  onRefreshSerialPorts?: () => Promise<string[]>;
  initialSection: string;
  onDraftChange: (draft: SessionProfile) => void;
  onSave: (proxyPasswordUpdate: ProxyPasswordUpdate) => Promise<SessionSummary | null>;
  onConnect: (saved: SessionSummary) => void;
  onOpenClientKeyManager?: () => Promise<void>;
  onExportProfile?: () => Promise<void>;
  onClose: () => void;
}) {
  const { locale } = useLocale();
  const [activeProtocol, setActiveProtocol] = useState<ProtocolTab>(() => protocolFromKind(draft.kind));
  const [activeSection, setActiveSection] = useState(initialSection);
  const [surface, setSurface] = useState<"quick" | "advanced">(() => (
    mode === "create" && initialSection === "session" ? "quick" : "advanced"
  ));
  const [proxyPasswordUpdate, setProxyPasswordUpdate] = useState<ProxyPasswordUpdate>(null);
  const [writeBusy, setWriteBusy] = useState(false);
  const [secretCleanupError, setSecretCleanupError] = useState("");
  const [selectedIdentityId, setSelectedIdentityId] = useState("");
  const [sectionQuery, setSectionQuery] = useState("");
  const [serialPortsRefreshing, setSerialPortsRefreshing] = useState(false);
  const [serialPortsRefreshError, setSerialPortsRefreshError] = useState("");
  const writeGate = useRef(new KeyedRequestGate<"write">());
  const serialPortsRefreshGate = useRef(new KeyedRequestGate<"ports">());
  const stagedSecretRefs = useRef(new Set<string>());
  const connectionDrafts = useRef(new Map<ProtocolTab, SessionProfile["connection"]>([
    [protocolFromKind(draft.kind), draft.connection],
  ]));
  const quickTargetRef = useRef<HTMLInputElement | HTMLSelectElement | null>(null);
  const sessionTree = sessionSettingTrees[activeProtocol];
  const allowedSections = useMemo(() => flattenSessionTree(sessionTree), [sessionTree]);
  const visibleTree = useMemo(
    () => filterSessionTree(sessionTree, sectionQuery, sessionSectionLabel),
    [locale, sectionQuery, sessionTree],
  );
  const visibleSections = useMemo(() => flattenSessionTree(visibleTree), [visibleTree]);
  const quickValidation = useMemo(() => validateQuickConnectProfile(draft), [draft, locale]);
  const busy = writeBusy;
  const quickSurface = mode === "create" && surface === "quick";

  const refreshSerialPorts = useCallback(async () => {
    if (!onRefreshSerialPorts) {
      setSerialPortsRefreshError(t("reading-the-device-serial-port-list-is-unavailable-in"));
      return;
    }
    const token = serialPortsRefreshGate.current.begin("ports");
    if (token === null) return;
    setSerialPortsRefreshing(true);
    setSerialPortsRefreshError("");
    try {
      await onRefreshSerialPorts();
    } catch (error) {
      if (serialPortsRefreshGate.current.isCurrent("ports", token)) {
        setSerialPortsRefreshError(formatError(error));
      }
    } finally {
      if (serialPortsRefreshGate.current.finish("ports", token)) {
        setSerialPortsRefreshing(false);
      }
    }
  }, [onRefreshSerialPorts]);

  useEffect(() => {
    if (!allowedSections.includes(activeSection)) {
      setActiveSection("session");
      return;
    }
    if (visibleSections.length && !visibleSections.includes(activeSection)) {
      setActiveSection(visibleSections[0]);
    }
  }, [activeSection, allowedSections, visibleSections]);

  useEffect(() => {
    writeGate.current.invalidateAll();
    setWriteBusy(false);
    void cleanupStagedSecrets();
    setActiveProtocol(protocolFromKind(draft.kind));
    setActiveSection(initialSection);
    setSectionQuery("");
    setSurface(mode === "create" && initialSection === "session" ? "quick" : "advanced");
    connectionDrafts.current.clear();
    connectionDrafts.current.set(protocolFromKind(draft.kind), draft.connection);
  }, [draft.id, initialSection, mode]);

  useEffect(() => {
    connectionDrafts.current.set(activeProtocol, draft.connection);
  }, [activeProtocol, draft.connection]);

  useEffect(() => {
    if (!quickSurface) return;
    const frame = requestAnimationFrame(() => quickTargetRef.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [activeProtocol, quickSurface]);

  useEffect(() => () => {
    writeGate.current.invalidateAll();
    serialPortsRefreshGate.current.invalidateAll();
  }, []);

  useEffect(() => {
    if (activeProtocol !== "Serial" || !onRefreshSerialPorts) return;
    void refreshSerialPorts();
  }, [activeProtocol, onRefreshSerialPorts, refreshSerialPorts]);

  function beginWriteOperation() {
    const token = writeGate.current.begin("write");
    if (token !== null) setWriteBusy(true);
    return token;
  }

  function finishWriteOperation(token: number) {
    if (writeGate.current.finish("write", token)) setWriteBusy(false);
  }

  function registerStagedSecret(secretRef: string, token: number) {
    if (!writeGate.current.isCurrent("write", token)) return false;
    stagedSecretRefs.current.add(secretRef);
    return true;
  }

  function changeProtocol(tab: ProtocolTab) {
    if (busy) return;
    connectionDrafts.current.set(activeProtocol, draft.connection);
    const converted = convertDraftProtocol(draft, tab);
    const cachedConnection = connectionDrafts.current.get(tab);
    const nextDraft = cachedConnection
      ? { ...converted, kind: cachedConnection.kind, connection: cachedConnection }
      : converted;
    setActiveProtocol(tab);
    setActiveSection(surface === "advanced" ? protocolSettingsSection(tab) : "session");
    setSectionQuery("");
    setProxyPasswordUpdate(null);
    onDraftChange(nextDraft);
  }

  function openAdvancedSettings() {
    setActiveSection(protocolSettingsSection(activeProtocol));
    setSurface("advanced");
  }

  async function cleanupStagedSecrets(retained = new Set<string>(), token?: number) {
    for (const secretRef of retained) stagedSecretRefs.current.delete(secretRef);
    const failures: string[] = [];
    for (const secretRef of [...stagedSecretRefs.current]) {
      try {
        await invokeBackend("delete_secret", { secretRef });
        stagedSecretRefs.current.delete(secretRef);
      } catch (error) {
        failures.push(formatError(error));
      }
    }
    if (token === undefined || writeGate.current.isCurrent("write", token)) {
      setSecretCleanupError(failures.length ? t("failed-to-clean-up-staged-credentials", [failures.join("；")]) : "");
    }
    return failures.length === 0;
  }

  async function submit(connectAfterSave: boolean) {
    const token = beginWriteOperation();
    if (token === null) return;
    setSecretCleanupError("");
    try {
      const saved = await onSave(proxyPasswordUpdate);
      if (!saved) return;
      const cleaned = await cleanupStagedSecrets(profileCredentialSecretRefs(saved.profile), token);
      if (!cleaned || !writeGate.current.isCurrent("write", token)) return;
      onClose();
      if (connectAfterSave) onConnect(saved);
    } finally {
      finishWriteOperation(token);
    }
  }

  async function cancel() {
    const token = beginWriteOperation();
    if (token === null) return;
    setSecretCleanupError("");
    try {
      if (await cleanupStagedSecrets(new Set(), token)) {
        if (writeGate.current.isCurrent("write", token)) onClose();
      }
    } finally {
      finishWriteOperation(token);
    }
  }

  return (
    <DialogFrame
      title={mode === "create" ? t("new-session") : t("session-settings")}
      className={`session-settings-dialog ${mode === "create" ? "create-session-dialog" : "edit-session-dialog"} ${quickSurface ? "quick" : "advanced"}`}
      dataSessionProtocol={activeProtocol}
      dataSessionSection={activeSection}
      onClose={() => void cancel()}
      closeDisabled={busy}
    >
      {quickSurface ? (
        <>
          <QuickProtocolTabs activeProtocol={activeProtocol} busy={busy} onChange={changeProtocol} />
          <section className="session-quick-form" id="quick-session-fields" role="tabpanel" aria-label={t("quick-setup", [protocolLabel(activeProtocol)])} inert={busy}>
            <QuickSessionFields
              activeProtocol={activeProtocol}
              draft={draft}
              issues={quickValidation.issues}
              serialPorts={serialPorts}
              serialPortsRefreshing={serialPortsRefreshing}
              serialPortsRefreshError={serialPortsRefreshError}
              onRefreshSerialPorts={() => void refreshSerialPorts()}
              targetRef={quickTargetRef}
              onDraftChange={onDraftChange}
            />
            <QuickSessionMetadata draft={draft} onDraftChange={onDraftChange} />
          </section>
        </>
      ) : (
        <>
          <QuickProtocolTabs
            activeProtocol={activeProtocol}
            busy={busy}
            compact
            ariaLabel={t("session-type")}
            controlsId="session-settings-panel"
            onChange={changeProtocol}
          />
          <SessionSettingsSidebar
            mode={mode}
            busy={busy}
            query={sectionQuery}
            tree={visibleTree}
            activeSection={activeSection}
            onQueryChange={setSectionQuery}
            onSelectSection={setActiveSection}
            onReturnToQuick={() => {
              setSectionQuery("");
              setSurface("quick");
            }}
          />
          <section className="session-form" id="session-settings-panel" role="tabpanel" aria-label={sessionSectionLabel(activeSection)} inert={busy}>
            <header className="session-settings-pane-title">
              <h2>{sessionSectionLabel(activeSection)}</h2>
            </header>
            <SessionSettingsContent
              activeProtocol={activeProtocol}
              activeSection={activeSection}
              draft={draft}
              prepareProfile={prepareProfile}
              serialPorts={serialPorts}
              serialPortsRefreshing={serialPortsRefreshing}
              serialPortsRefreshError={serialPortsRefreshError}
              onRefreshSerialPorts={() => void refreshSerialPorts()}
              onDraftChange={onDraftChange}
              proxyPasswordUpdate={proxyPasswordUpdate}
              onProxyPasswordUpdateChange={setProxyPasswordUpdate}
              writeBusy={writeBusy}
              onWriteStart={beginWriteOperation}
              onSecretCreated={registerStagedSecret}
              onWriteFinish={finishWriteOperation}
              selectedIdentityId={selectedIdentityId}
              onSelectedIdentityIdChange={setSelectedIdentityId}
              onOpenClientKeyManager={onOpenClientKeyManager}
            />
          </section>
        </>
      )}
      <div className={`dialog-actions session-settings-actions ${mode === "create" ? "create-actions" : "edit-actions"}`}>
        {mode === "create" && quickSurface ? (
          <button type="button" className="session-advanced-button" onClick={openAdvancedSettings} disabled={busy}>
            <SlidersHorizontal size={15} />{t("advanced-settings")}</button>
        ) : null}
        <span className={`session-action-status ${secretCleanupError ? "error" : ""}`} aria-live="polite">
          {localizeDiagnostic(secretCleanupError) || (mode === "create" && !quickValidation.valid ? (
            <><CircleAlert size={14} />{t("complete-the-highlighted-connection-details")}</>
          ) : null)}
        </span>
        {mode === "create" ? (
          <>
            <button type="button" className="session-cancel-button" onClick={() => void cancel()} disabled={busy}>{t("cancel")}</button>
            <button type="button" className="session-save-button" onClick={() => void submit(false)} disabled={busy}>
              <Save size={15} />{t("save-only")}</button>
            <button type="button" className="session-connect-button" onClick={() => void submit(true)} disabled={busy || !quickValidation.valid}>
              <PlugZap size={15} />{t("connect")}</button>
          </>
        ) : (
          <>
            {onExportProfile ? <button type="button" onClick={() => void onExportProfile()} disabled={busy}>{t("export-portable-profile")}</button> : null}
            <button onClick={() => void submit(false)} disabled={busy}>{t("save")}</button>
            <button onClick={() => void submit(true)} disabled={busy}>{t("save-and-connect")}</button>
            <button onClick={() => void cancel()} disabled={busy}>{t("cancel")}</button>
          </>
        )}
      </div>
    </DialogFrame>
  );
}

function QuickProtocolTabs({
  activeProtocol,
  busy,
  onChange,
  compact = false,
  ariaLabel,
  controlsId = "quick-session-fields",
}: {
  activeProtocol: ProtocolTab;
  busy: boolean;
  onChange: (protocol: ProtocolTab) => void;
  compact?: boolean;
  ariaLabel?: string;
  controlsId?: string;
}) {
  useLocale();
  return (
    <div className={`session-protocol-tabs${compact ? " compact" : ""}`} role="tablist" aria-label={ariaLabel ?? t("connection-protocol")}>
      {protocolTabs.map((protocol) => (
        <button
          type="button"
          role="tab"
          aria-selected={activeProtocol === protocol}
          aria-controls={controlsId}
          className={activeProtocol === protocol ? "active" : ""}
          disabled={busy}
          key={protocol}
          onClick={() => onChange(protocol)}
        >
          <ProtocolIcon protocol={protocol} />
          <span>{protocolLabel(protocol)}</span>
        </button>
      ))}
    </div>
  );
}

function SessionSettingsSidebar({
  mode,
  busy,
  query,
  tree,
  activeSection,
  onQueryChange,
  onSelectSection,
  onReturnToQuick,
}: {
  mode: "create" | "edit";
  busy: boolean;
  query: string;
  tree: readonly { label: string; children?: readonly string[] }[];
  activeSection: string;
  onQueryChange: (value: string) => void;
  onSelectSection: (section: string) => void;
  onReturnToQuick: () => void;
}) {
  useLocale();
  return (
    <aside className="session-settings-nav">
      {mode === "create" ? (
        <button type="button" className="session-quick-return" onClick={onReturnToQuick} disabled={busy}>
          <SquareTerminal size={15} />{t("quick-setup-2")}
        </button>
      ) : null}
      <label className="settings-search">
        <Search size={14} aria-hidden="true" />
        <input
          type="search"
          value={query}
          disabled={busy}
          placeholder={t("search-settings")}
          aria-label={t("search-settings")}
          onChange={(event) => onQueryChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape" && query) {
              event.preventDefault();
              onQueryChange("");
            }
          }}
        />
      </label>
      <nav className="session-settings-tree" role="tree" aria-label={t("session-settings-pages")}>
        {tree.length ? tree.map((node) => (
          <SessionTreeBranch
            key={node.label}
            node={node}
            busy={busy}
            activeSection={activeSection}
            onSelectSection={onSelectSection}
          />
        )) : (
          <p className="settings-search-empty">{t("no-matching-settings")}</p>
        )}
      </nav>
    </aside>
  );
}

function SessionTreeBranch({
  node,
  busy,
  activeSection,
  onSelectSection,
}: {
  node: { label: string; children?: readonly string[] };
  busy: boolean;
  activeSection: string;
  onSelectSection: (section: string) => void;
}) {
  useLocale();
  return (
    <div className="session-settings-tree-branch">
      <SessionTreeItem
        section={node.label}
        selected={activeSection === node.label}
        busy={busy}
        onSelect={onSelectSection}
      />
      {node.children?.length ? (
        <div className="session-settings-tree-children" role="group">
          {node.children.map((child) => (
            <SessionTreeItem
              key={child}
              section={child}
              nested
              selected={activeSection === child}
              busy={busy}
              onSelect={onSelectSection}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function SessionTreeItem({
  section,
  nested = false,
  selected,
  busy,
  onSelect,
}: {
  section: string;
  nested?: boolean;
  selected: boolean;
  busy: boolean;
  onSelect: (section: string) => void;
}) {
  useLocale();
  return (
    <button
      type="button"
      role="treeitem"
      aria-selected={selected}
      className={`session-settings-tree-item${nested ? " nested" : ""}${selected ? " active" : ""}`}
      disabled={busy}
      onClick={() => onSelect(section)}
    >
      {sessionSectionLabel(section)}
    </button>
  );
}

function ProtocolIcon({ protocol }: { protocol: ProtocolTab }) {
  useLocale();
  const props = { size: 17, "aria-hidden": true } as const;
  switch (protocol) {
    case "Shell": return <SquareTerminal {...props} />;
    case "SSH": return <Server {...props} />;
    case "Tmux": return <Layers3 {...props} />;
    case "Telnet": return <Network {...props} />;
    case "Tcp": return <Cable {...props} />;
    case "Serial": return <Usb {...props} />;
  }
}

function protocolLabel(protocol: ProtocolTab) {
  return protocol === "Tcp" ? "TCP" : protocol;
}

function QuickField({
  label,
  required = false,
  error,
  className = "",
  group = false,
  children,
}: {
  label: string;
  required?: boolean;
  error?: string;
  className?: string;
  group?: boolean;
  children: ReactNode;
}) {
  useLocale();
  const Field = group ? "div" : "label";
  return (
    <Field className={`session-quick-field ${error ? "invalid" : ""} ${className}`} role={group ? "group" : undefined} aria-label={group ? label : undefined}>
      <span className="session-quick-field-label">{label}{required ? <sup aria-hidden="true">*</sup> : null}</span>
      {children}
      <span className="session-quick-field-error" aria-hidden={!error}>{localizeDiagnostic(error) ?? ""}</span>
    </Field>
  );
}

function QuickToggle({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  useLocale();
  return (
    <div className="session-quick-toggle">
      <span>{label}</span>
      <button type="button" className={checked ? "switch-toggle on" : "switch-toggle"} aria-label={label} aria-pressed={checked} onClick={() => onChange(!checked)}>
        <span />
      </button>
    </div>
  );
}

function QuickSessionFields({
  activeProtocol,
  draft,
  issues,
  serialPorts,
  serialPortsRefreshing,
  serialPortsRefreshError,
  onRefreshSerialPorts,
  targetRef,
  onDraftChange,
}: {
  activeProtocol: ProtocolTab;
  draft: SessionProfile;
  issues: QuickConnectIssue[];
  serialPorts: string[];
  serialPortsRefreshing: boolean;
  serialPortsRefreshError: string;
  onRefreshSerialPorts: () => void;
  targetRef: MutableRefObject<HTMLInputElement | HTMLSelectElement | null>;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  useLocale();
  const errorFor = (field: QuickConnectField) => issues.find((issue) => issue.field === field)?.message;
  const setTargetRef = (node: HTMLInputElement | HTMLSelectElement | null) => {
    targetRef.current = node;
  };

  return (
    <div className="session-quick-target" key={activeProtocol}>
      <header className="session-quick-section-heading">
        <ProtocolIcon protocol={activeProtocol} />
        <h2>{t("connection-target")}</h2>
      </header>
      {activeProtocol === "SSH" || activeProtocol === "Tmux" ? (
        <QuickSshFields
          protocol={activeProtocol}
          draft={draft}
          targetError={errorFor("target")}
          portError={errorFor("port")}
          setTargetRef={setTargetRef}
          onDraftChange={onDraftChange}
        />
      ) : null}
      {activeProtocol === "Telnet" || activeProtocol === "Tcp" ? (
        <QuickTcpFields
          protocol={activeProtocol}
          draft={draft}
          targetError={errorFor("target")}
          portError={errorFor("port")}
          setTargetRef={setTargetRef}
          onDraftChange={onDraftChange}
        />
      ) : null}
      {activeProtocol === "Serial" ? (
        <QuickSerialFields
          draft={draft}
          serialPorts={serialPorts}
          serialPortsRefreshing={serialPortsRefreshing}
          serialPortsRefreshError={serialPortsRefreshError}
          onRefreshSerialPorts={onRefreshSerialPorts}
          targetError={errorFor("target")}
          baudRateError={errorFor("baudRate")}
          setTargetRef={setTargetRef}
          onDraftChange={onDraftChange}
        />
      ) : null}
      {activeProtocol === "Shell" ? (
        <QuickShellFields draft={draft} setTargetRef={setTargetRef} onDraftChange={onDraftChange} />
      ) : null}
    </div>
  );
}

function QuickSshFields({
  protocol,
  draft,
  targetError,
  portError,
  setTargetRef,
  onDraftChange,
}: {
  protocol: "SSH" | "Tmux";
  draft: SessionProfile;
  targetError?: string;
  portError?: string;
  setTargetRef: (node: HTMLInputElement | HTMLSelectElement | null) => void;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  useLocale();
  const kind: "ssh" | "tmux" = protocol === "Tmux" ? "tmux" : "ssh";
  const current = draft.connection.kind === "ssh" || draft.connection.kind === "tmux"
    ? draft.connection
    : createSshConnection();
  const ssh = { ...current, kind };

  return (
    <div className="session-quick-grid ssh-quick-grid">
      <QuickField label={t("host-ip")} required error={targetError}>
        <input
          ref={setTargetRef}
          aria-label={t("host-or-ip", [protocolLabel(protocol)])}
          aria-invalid={Boolean(targetError)}
          autoComplete="off"
          placeholder={t("router-local-or-192-168-1-10")}
          value={ssh.endpoint.host}
          onChange={(event) => onDraftChange({
            ...draft,
            kind,
            connection: { ...ssh, kind, endpoint: { ...ssh.endpoint, host: event.target.value } },
          })}
        />
      </QuickField>
      <QuickField label={t("port")} required error={portError}>
        <input
          type="number"
          inputMode="numeric"
          aria-label={t("port-2", [protocolLabel(protocol)])}
          aria-invalid={Boolean(portError)}
          min={1}
          max={65535}
          value={ssh.endpoint.port || ""}
          onChange={(event) => onDraftChange({
            ...draft,
            kind,
            connection: { ...ssh, endpoint: { ...ssh.endpoint, port: Number(event.target.value) } },
          })}
        />
      </QuickField>
      <QuickField label={t("username")} className="ssh-username-field">
        <input
          aria-label={t("username-2", [protocolLabel(protocol)])}
          autoComplete="username"
          placeholder="root"
          value={ssh.username}
          onChange={(event) => onDraftChange({
            ...draft,
            kind,
            connection: { ...ssh, kind, username: event.target.value },
          })}
        />
      </QuickField>
    </div>
  );
}

function QuickTcpFields({
  protocol,
  draft,
  targetError,
  portError,
  setTargetRef,
  onDraftChange,
}: {
  protocol: "Telnet" | "Tcp";
  draft: SessionProfile;
  targetError?: string;
  portError?: string;
  setTargetRef: (node: HTMLInputElement | HTMLSelectElement | null) => void;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  useLocale();
  const kind: "telnet" | "tcp" = protocol === "Telnet" ? "telnet" : "tcp";
  const current = draft.connection.kind === "telnet" || draft.connection.kind === "tcp"
    ? draft.connection
    : createTcpConnection(kind);
  const tcp = { ...current, kind };
  const update = (patch: Partial<typeof tcp>) => onDraftChange({
    ...draft,
    kind,
    connection: { ...tcp, ...patch, kind },
  });

  return (
    <div className="session-quick-grid target-port-grid">
      <QuickField label={t("host")} required error={targetError}>
        <input
          ref={setTargetRef}
          aria-label={t("host-2", [protocolLabel(protocol)])}
          aria-invalid={Boolean(targetError)}
          autoComplete="off"
          placeholder="192.168.1.10"
          value={tcp.host}
          onChange={(event) => update({ host: event.target.value })}
        />
      </QuickField>
      <QuickField label={t("port")} required error={portError}>
        <input
          type="number"
          inputMode="numeric"
          aria-label={t("port-2", [protocolLabel(protocol)])}
          aria-invalid={Boolean(portError)}
          min={1}
          max={65535}
          value={tcp.port || ""}
          onChange={(event) => update({ port: Number(event.target.value) })}
        />
      </QuickField>
      {protocol === "Tcp" ? (
        <QuickToggle label="TLS" checked={tcp.tlsEnabled} onChange={(tlsEnabled) => update({ tlsEnabled })} />
      ) : null}
    </div>
  );
}

function QuickSerialFields({
  draft,
  serialPorts,
  serialPortsRefreshing,
  serialPortsRefreshError,
  onRefreshSerialPorts,
  targetError,
  baudRateError,
  setTargetRef,
  onDraftChange,
}: {
  draft: SessionProfile;
  serialPorts: string[];
  serialPortsRefreshing: boolean;
  serialPortsRefreshError: string;
  onRefreshSerialPorts: () => void;
  targetError?: string;
  baudRateError?: string;
  setTargetRef: (node: HTMLInputElement | HTMLSelectElement | null) => void;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  useLocale();
  const serial = draft.connection.kind === "serial" ? draft.connection : createSerialConnection();
  const update = (patch: Partial<typeof serial>) => onDraftChange({
    ...draft,
    kind: "serial",
    connection: { ...serial, ...patch },
  });

  return (
    <div className="session-quick-grid serial-quick-grid">
      <QuickField label={t("serial")} required error={targetError} className="serial-port-field" group>
        <div className="serial-port-picker">
          <select
            ref={setTargetRef}
            aria-label={t("serial")}
            aria-invalid={Boolean(targetError)}
            value={serial.port}
            onChange={(event) => update({ port: event.target.value })}
          >
            <option value="">{serialPorts.length ? t("select-serial-port") : t("no-serial-ports-found")}</option>
            {serialPortOptions(serial.port, serialPorts).map((option) => (
              <option key={option} value={option}>{option}</option>
            ))}
          </select>
          <button
            type="button"
            className="serial-port-refresh-button"
            title={t("refresh-device-serial-ports")}
            aria-label={t("refresh-serial-ports")}
            onClick={onRefreshSerialPorts}
            disabled={serialPortsRefreshing}
          >
            <RefreshCw size={14} className={serialPortsRefreshing ? "spin" : ""} />
          </button>
        </div>
      </QuickField>
      {serialPortsRefreshError ? <div className="serial-port-refresh-status" role="status">{localizeDiagnostic(serialPortsRefreshError)}</div> : null}
      <QuickField label={t("baud-rate")} required error={baudRateError} className="serial-baud-rate-field">
        <input
          type="number"
          inputMode="numeric"
          aria-label={t("baud-rate")}
          aria-invalid={Boolean(baudRateError)}
          min={serialConnectionBounds.baudRate.min}
          max={serialConnectionBounds.baudRate.max}
          list="quick-serial-baud-rate-options"
          value={Number.isFinite(serial.baudRate) ? serial.baudRate : ""}
          onChange={(event) => update({ baudRate: Number(event.target.value) })}
        />
        <datalist id="quick-serial-baud-rate-options">
          {COMMON_SERIAL_BAUD_RATES.map((baudRate) => <option key={baudRate} value={baudRate} />)}
        </datalist>
      </QuickField>
      <QuickField label={t("data-bits")}>
        <select aria-label={t("data-bits")} value={serial.dataBits} onChange={(event) => update({ dataBits: Number(event.target.value) })}>
          {[5, 6, 7, 8].map((value) => <option key={value} value={value}>{value}</option>)}
        </select>
      </QuickField>
      <QuickField label={t("stop-bits")}>
        <select aria-label={t("stop-bits")} value={serial.stopBits} onChange={(event) => update({ stopBits: Number(event.target.value) })}>
          <option value={1}>1</option>
          <option value={2}>2</option>
        </select>
      </QuickField>
      <QuickField label={t("parity")}>
        <select aria-label={t("parity")} value={serial.parity} onChange={(event) => update({ parity: event.target.value })}>
          <option value="none">{t("none")}</option>
          <option value="odd">{t("odd")}</option>
          <option value="even">{t("even")}</option>
        </select>
      </QuickField>
      <QuickField label={t("flow-control")}>
        <select aria-label={t("flow-control")} value={serial.flowControl} onChange={(event) => update({ flowControl: event.target.value })}>
          <option value="none">{t("none")}</option>
          <option value="software">{t("software")}</option>
          <option value="hardware">{t("hardware")}</option>
        </select>
      </QuickField>
    </div>
  );
}

function QuickShellFields({
  draft,
  setTargetRef,
  onDraftChange,
}: {
  draft: SessionProfile;
  setTargetRef: (node: HTMLInputElement | HTMLSelectElement | null) => void;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  useLocale();
  const shell = draft.connection.kind === "shell" ? draft.connection : createShellConnection();
  const update = (patch: Partial<typeof shell>) => onDraftChange({
    ...draft,
    kind: "shell",
    connection: { ...shell, ...patch },
  });

  return (
    <div className="session-quick-grid shell-quick-grid">
      <QuickField label={t("program")}>
        <input ref={setTargetRef} aria-label={t("shell-program")} placeholder={t("system-default-shell")} value={shell.program} onChange={(event) => update({ program: event.target.value })} />
      </QuickField>
      <div className="session-quick-field shell-arguments-field">
        <span className="session-quick-field-label">{t("arguments")}</span>
        <ShellArgumentsEditor args={shell.args} onChange={(args) => update({ args })} />
      </div>
      <QuickField label={t("directory")}>
        <input aria-label={t("shell-working-directory")} placeholder={t("current-user-s-directory")} value={shell.cwd ?? ""} onChange={(event) => update({ cwd: event.target.value || null })} />
      </QuickField>
    </div>
  );
}

function QuickSessionMetadata({ draft, onDraftChange }: { draft: SessionProfile; onDraftChange: (draft: SessionProfile) => void }) {
  useLocale();
  const [tagsText, setTagsText] = useState(() => draft.tags.join(", "));
  useEffect(() => setTagsText(draft.tags.join(", ")), [draft.id]);

  return (
    <section className="session-quick-metadata">
      <header className="session-quick-section-heading session-metadata-heading">
        <FolderTree size={15} />
        <h2>{t("session-information")}</h2>
      </header>
      <div className="session-quick-grid metadata-quick-grid">
        <QuickField label={t("name")}>
          <input
            aria-label={t("session-name")}
            value={draft.name}
            onChange={(event) => onDraftChange({ ...draft, name: normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_NAME_CHARACTERS) })}
          />
        </QuickField>
        <QuickField label={t("group-2")}>
          <input
            aria-label={t("session-group")}
            value={draft.group}
            onChange={(event) => onDraftChange({ ...draft, group: normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_GROUP_CHARACTERS) })}
          />
        </QuickField>
        <QuickField label={t("tags")}>
          <input
            aria-label={t("session-tags")}
            placeholder={t("comma-separated")}
            value={tagsText}
            onChange={(event) => {
              const nextText = normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_TAG_INPUT_CHARACTERS);
              setTagsText(nextText);
              onDraftChange({ ...draft, tags: nextText.split(",").map((item) => item.trim()).filter(Boolean) });
            }}
          />
        </QuickField>
      </div>
    </section>
  );
}

function SessionSettingsContent({
  activeProtocol,
  activeSection,
  draft,
  prepareProfile,
  serialPorts,
  serialPortsRefreshing,
  serialPortsRefreshError,
  onRefreshSerialPorts,
  onDraftChange,
  proxyPasswordUpdate,
  onProxyPasswordUpdateChange,
  writeBusy,
  onWriteStart,
  onSecretCreated,
  onWriteFinish,
  selectedIdentityId,
  onSelectedIdentityIdChange,
  onOpenClientKeyManager,
}: {
  activeProtocol: ProtocolTab;
  activeSection: string;
  draft: SessionProfile;
  prepareProfile: (profile: SessionProfile) => SessionProfile;
  serialPorts: string[];
  serialPortsRefreshing: boolean;
  serialPortsRefreshError: string;
  onRefreshSerialPorts: () => void;
  onDraftChange: (draft: SessionProfile) => void;
  proxyPasswordUpdate: ProxyPasswordUpdate;
  onProxyPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
  writeBusy: boolean;
  onWriteStart: () => number | null;
  onSecretCreated: (secretRef: string, token: number) => boolean;
  onWriteFinish: (token: number) => void;
  selectedIdentityId: string;
  onSelectedIdentityIdChange: (id: string) => void;
  onOpenClientKeyManager?: () => Promise<void>;
}) {
  useLocale();
  if (activeSection === "session") {
    return <SessionCommonOverviewFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeSection === "terminal") {
    return (
      <>
        <DialogField label={t("terminal-t")}>
          <input value={draft.terminal.term} maxLength={MAX_TERMINAL_NAME_BYTES} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, term: event.target.value } })} />
        </DialogField>
        <DialogField label={t("rows-r")}>
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.rows.min} max={TERMINAL_PROFILE_BOUNDS.rows.max} step={1} value={draft.terminal.rows} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, rows: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label={t("columns-c")}>
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.cols.min} max={TERMINAL_PROFILE_BOUNDS.cols.max} step={1} value={draft.terminal.cols} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, cols: Number(event.target.value) } })} />
        </DialogField>
        {draft.connection.kind === "serial" ? <p className="muted">{t("serial-uses-a-fixed-column-count-independent-of-window")}</p> : null}
        <DialogField label={t("scrollback-s")}>
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.scrollback.min} max={TERMINAL_PROFILE_BOUNDS.scrollback.max} step={1} value={draft.terminal.scrollback} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, scrollback: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label={t("font-f")}>
          <input value={draft.terminal.fontFamily} maxLength={MAX_TERMINAL_FONT_FAMILY_CHARACTERS} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, fontFamily: event.target.value } })} />
        </DialogField>
        <DialogField label={t("font-size-z")}>
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.fontSize.min} max={TERMINAL_PROFILE_BOUNDS.fontSize.max} step={1} value={draft.terminal.fontSize} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, fontSize: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label={t("theme-m")}>
          <select value={normalizeTerminalTheme(draft.terminal.theme)} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, theme: event.target.value } })}>
            {TERMINAL_THEME_OPTIONS.map((option) => <option key={option.value} value={option.value}>{t(option.label)}</option>)}
          </select>
        </DialogField>
        <DialogField label={t("background-opacity-o")}>
          <div className="terminal-opacity-control">
            <input
              type="range"
              min={TERMINAL_PROFILE_BOUNDS.backgroundOpacity.min}
              max={TERMINAL_PROFILE_BOUNDS.backgroundOpacity.max}
              step={5}
              value={draft.terminal.backgroundOpacity ?? TERMINAL_PROFILE_BOUNDS.backgroundOpacity.fallback}
              onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, backgroundOpacity: Number(event.target.value) } })}
            />
            <output>{draft.terminal.backgroundOpacity ?? TERMINAL_PROFILE_BOUNDS.backgroundOpacity.fallback}%</output>
          </div>
        </DialogField>
      </>
    );
  }

  if (activeSection === "logs") {
    return (
      <>
        <DialogField label={t("enabled-e")}>
          <select value={draft.logging.enabled ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, enabled: event.target.value === "on" } })}>
            <option value="on">{t("enabled")}</option>
            <option value="off">{t("close")}</option>
          </select>
        </DialogField>
        <DialogField label={t("raw-not-redacted-r")}>
          <select value={draft.logging.raw ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, raw: event.target.value === "on" } })}>
            <option value="on">{t("enabled")}</option>
            <option value="off">{t("close")}</option>
          </select>
        </DialogField>
        <DialogField label={t("ui-text-t")}>
          <select value={draft.logging.text ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, text: event.target.value === "on" } })}>
            <option value="on">{t("enabled")}</option>
            <option value="off">{t("close")}</option>
          </select>
        </DialogField>
        <DialogField label="JSONL:(J)">
          <select value={draft.logging.jsonl ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, jsonl: event.target.value === "on" } })}>
            <option value="on">{t("enabled")}</option>
            <option value="off">{t("close")}</option>
          </select>
        </DialogField>
        <DialogField label={t("sensitive-fields-s")}>
          <select value={draft.logging.redactSecrets ? "redact" : "plain"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, redactSecrets: event.target.value === "redact" } })}>
            <option value="redact">{t("hide-2")}</option>
            <option value="plain">{t("record-in-full")}</option>
          </select>
        </DialogField>
        <DialogField label={t("path-p")}>
          <input value={draft.logging.pathTemplate} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, pathTemplate: event.target.value } })} />
        </DialogField>
        <DialogField label={t("retention-days-d")}>
          <input type="number" min={0} max={3650} value={draft.logging.retentionDays ?? 0} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, retentionDays: Math.min(3650, Math.max(0, Math.trunc(Number(event.target.value) || 0))) } })} />
        </DialogField>
      </>
    );
  }

  if (activeSection === "triggers") {
    return <TriggerFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeSection === "transfers") {
    return <SessionTransferFields activeProtocol={activeProtocol} draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeProtocol === "Shell" && activeSection === "Shell") {
    return <ShellProcessFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if ((activeProtocol === "SSH" || activeProtocol === "Tmux") && (activeSection === "SSH" || activeSection === "Tmux")) {
    return <SshAdvancedFields section="connect" draft={draft} prepareProfile={prepareProfile} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} writeBusy={writeBusy} onWriteStart={onWriteStart} onSecretCreated={onSecretCreated} onWriteFinish={onWriteFinish} selectedIdentityId={selectedIdentityId} onSelectedIdentityIdChange={onSelectedIdentityIdChange} onOpenClientKeyManager={onOpenClientKeyManager} />;
  }

  if ((activeProtocol === "SSH" || activeProtocol === "Tmux") && ["proxy", "verification", "ssh-agent", "password", "public-key"].includes(activeSection)) {
    return <SshAdvancedFields section={activeSection} draft={draft} prepareProfile={prepareProfile} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} writeBusy={writeBusy} onWriteStart={onWriteStart} onSecretCreated={onSecretCreated} onWriteFinish={onWriteFinish} selectedIdentityId={selectedIdentityId} onSelectedIdentityIdChange={onSelectedIdentityIdChange} onOpenClientKeyManager={onOpenClientKeyManager} />;
  }

  if (activeProtocol === "Telnet" && activeSection === "Telnet") {
    return <TcpLikeAdvancedFields protocol="Telnet" section="connect" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if (activeProtocol === "Tcp" && activeSection === "Tcp") {
    return <TcpLikeAdvancedFields protocol="Tcp" section="connect" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if ((activeProtocol === "Telnet" || activeProtocol === "Tcp") && activeSection === "proxy") {
    return <TcpLikeAdvancedFields protocol={activeProtocol} section="proxy" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if (activeProtocol === "Serial" && activeSection === "serial") {
    return <SerialAdvancedFields draft={draft} serialPorts={serialPorts} serialPortsRefreshing={serialPortsRefreshing} serialPortsRefreshError={serialPortsRefreshError} onRefreshSerialPorts={onRefreshSerialPorts} onDraftChange={onDraftChange} />;
  }

  return null;
}

function SessionCommonOverviewFields({
  draft,
  onDraftChange,
}: {
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  useLocale();
  const [tagsText, setTagsText] = useState(() => draft.tags.join(", "));
  useEffect(() => {
    setTagsText(draft.tags.join(", "));
  }, [draft.id]);
  return (
    <>
      <DialogField label={t("name-n")}>
        <input value={draft.name} onChange={(event) => onDraftChange({ ...draft, name: normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_NAME_CHARACTERS) })} />
      </DialogField>
      <DialogField label={t("group-g")}>
        <input value={draft.group} onChange={(event) => onDraftChange({ ...draft, group: normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_GROUP_CHARACTERS) })} placeholder={t("nested-group-a-b-c")} />
      </DialogField>
      <DialogField label={t("tags-l")}>
        <input value={tagsText} onChange={(event) => {
          const nextText = normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_TAG_INPUT_CHARACTERS);
          setTagsText(nextText);
          onDraftChange({ ...draft, tags: nextText.split(",").map((item) => item.trim()).filter(Boolean) });
        }} />
      </DialogField>
    </>
  );
}

function SessionTransferFields({
  activeProtocol,
  draft,
  onDraftChange,
}: {
  activeProtocol: ProtocolTab;
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  useLocale();
  const update = (patch: Partial<SessionProfile["transfer"]>) => onDraftChange({
    ...draft,
    transfer: { ...draft.transfer, ...patch },
  });
  const sshLike = activeProtocol === "SSH" || activeProtocol === "Tmux";

  return (
    <>
      {sshLike ? <DialogToggleField label="SFTP:" checked={draft.transfer.sftp} onChange={(sftp) => update({ sftp })} /> : null}
      {sshLike ? <DialogToggleField label="SCP:" checked={draft.transfer.scp} onChange={(scp) => update({ scp })} /> : null}
      <DialogToggleField label="TFTP:" checked={draft.transfer.tftp !== false} onChange={(tftp) => update({ tftp })} />
      <DialogToggleField label="XModem:" checked={draft.transfer.xmodem} onChange={(xmodem) => update({ xmodem })} />
      <DialogToggleField label="YModem:" checked={draft.transfer.ymodem} onChange={(ymodem) => update({ ymodem })} />
      <DialogToggleField label="ZModem:" checked={draft.transfer.zmodem} onChange={(zmodem) => update({ zmodem })} />
      <DialogField label={t("rate-limit-b-s")}>
        <input type="number" min={0} value={draft.transfer.rateLimitBytesPerSecond ?? 0} onChange={(event) => update({ rateLimitBytesPerSecond: Number(event.target.value) > 0 ? Number(event.target.value) : null })} />
      </DialogField>
      <DialogField label={t("default-directory-d")}>
        <input value={draft.transfer.defaultLocalDir ?? ""} onChange={(event) => update({ defaultLocalDir: event.target.value || null })} />
      </DialogField>
    </>
  );
}

function ShellProcessFields({
  draft,
  onDraftChange,
}: {
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  useLocale();
  const shell = draft.connection.kind === "shell" ? draft.connection : createShellConnection();
  return (
    <>
      <DialogField label={t("program-p")}>
        <input value={shell.program} onChange={(event) => onDraftChange({ ...draft, kind: "shell", connection: { ...shell, program: event.target.value } })} />
      </DialogField>
      <div className="dialog-field shell-arguments-field">
        <span>{t("arguments-a")}</span>
        <ShellArgumentsEditor
          args={shell.args}
          onChange={(args) => onDraftChange({ ...draft, kind: "shell", connection: { ...shell, args } })}
        />
      </div>
      <DialogField label={t("directory-w")}>
        <input value={shell.cwd ?? ""} onChange={(event) => onDraftChange({ ...draft, kind: "shell", connection: { ...shell, cwd: event.target.value || null } })} />
      </DialogField>
    </>
  );
}

function TriggerFields({ draft, onDraftChange }: { draft: SessionProfile; onDraftChange: (draft: SessionProfile) => void }) {
  useLocale();
  function setTriggers(triggers: TriggerSpec[]) {
    onDraftChange({ ...draft, triggers });
  }

  function updateTrigger(index: number, patch: Partial<TriggerSpec>) {
    setTriggers(draft.triggers.map((trigger, triggerIndex) => (
      triggerIndex === index ? { ...trigger, ...patch } : trigger
    )));
  }

  function updateAction(triggerIndex: number, actionIndex: number, action: TriggerAction) {
    const trigger = draft.triggers[triggerIndex];
    updateTrigger(triggerIndex, {
      actions: trigger.actions.map((item, index) => (index === actionIndex ? action : item)),
    });
  }

  return (
    <div className="trigger-editor">
      {draft.triggers.map((trigger, triggerIndex) => {
        const matcherValue = trigger.matcher.type === "regex" ? trigger.matcher.pattern : trigger.matcher.text;
        return (
          <section className="trigger-item" key={trigger.id}>
            <header className="trigger-item-header">
              <label className="trigger-enabled">
                <input type="checkbox" checked={trigger.enabled} onChange={(event) => updateTrigger(triggerIndex, { enabled: event.target.checked })} />
                <span>{t("enable")}</span>
              </label>
              <input aria-label={t("trigger-name")} maxLength={MAX_TRIGGER_LABEL_CHARACTERS} value={trigger.label} onChange={(event) => updateTrigger(triggerIndex, { label: event.target.value })} />
              <button type="button" className="icon-button" title={t("delete-trigger")} aria-label={t("delete-trigger")} onClick={() => setTriggers(draft.triggers.filter((_, index) => index !== triggerIndex))}><Trash2 size={14} /></button>
            </header>
            <div className="trigger-matcher-row">
              <select
                aria-label={t("match-type")}
                value={trigger.matcher.type}
                onChange={(event) => updateTrigger(triggerIndex, {
                  matcher: event.target.value === "regex"
                    ? { type: "regex", pattern: matcherValue }
                    : { type: "contains", text: matcherValue, case_sensitive: false },
                })}
              >
                <option value="contains">{t("contains-text")}</option>
                <option value="regex">{t("regular-expression")}</option>
              </select>
              <input
                aria-label={t("match-pattern")}
                maxLength={MAX_TRIGGER_MATCHER_CHARACTERS}
                value={matcherValue}
                onChange={(event) => updateTrigger(triggerIndex, {
                  matcher: trigger.matcher.type === "regex"
                    ? { type: "regex", pattern: event.target.value }
                    : { ...trigger.matcher, text: event.target.value },
                })}
              />
              <label className="trigger-case-toggle">
                <input
                  type="checkbox"
                  checked={trigger.matcher.type === "contains" && trigger.matcher.case_sensitive}
                  disabled={trigger.matcher.type === "regex"}
                  onChange={(event) => {
                    if (trigger.matcher.type === "contains") {
                      updateTrigger(triggerIndex, { matcher: { ...trigger.matcher, case_sensitive: event.target.checked } });
                    }
                  }}
                />
                <span>{t("case-sensitive")}</span>
              </label>
            </div>
            <div className="trigger-action-list">
              {trigger.actions.map((action, actionIndex) => (
                <div className="trigger-action-row" key={`${trigger.id}-${actionIndex}`}>
                  <select
                    aria-label={t("action-type")}
                    value={action.type}
                    onChange={(event) => updateAction(triggerIndex, actionIndex, defaultTriggerAction(event.target.value as TriggerAction["type"]))}
                  >
                    <option value="timeline-mark">{t("timeline-marker")}</option>
                    <option value="notification">{t("notification")}</option>
                    <option value="highlight">{t("highlight")}</option>
                    <option value="send-text">{t("send-text")}</option>
                    <option value="local-command">{t("local-command")}</option>
                    <option value="custom-link">{t("custom-link")}</option>
                    <option value="sound">{t("sound")}</option>
                  </select>
                  {action.type === "sound" ? (
                    <select aria-label={t("sound")} value={action.name} onChange={(event) => updateAction(triggerIndex, actionIndex, { type: "sound", name: event.target.value })}>
                      <option value="bell">{t("ui-bell")}</option>
                      <option value="chime">{t("ui-chime")}</option>
                      <option value="alert">{t("ui-alert")}</option>
                    </select>
                  ) : (
                    <input
                      aria-label={t("action-parameters")}
                      maxLength={MAX_TRIGGER_ACTION_VALUE_CHARACTERS}
                      value={triggerActionValue(action)}
                      onChange={(event) => updateAction(triggerIndex, actionIndex, patchTriggerAction(action.type, event.target.value))}
                    />
                  )}
                  <button type="button" className="icon-button" title={t("delete-action")} aria-label={t("delete-action")} onClick={() => updateTrigger(triggerIndex, { actions: trigger.actions.filter((_, index) => index !== actionIndex) })}><Trash2 size={14} /></button>
                </div>
              ))}
              <button type="button" className="trigger-add-action" disabled={!canAddTriggerAction(trigger.actions.length)} title={canAddTriggerAction(trigger.actions.length) ? t("add-action") : t("at-most-16-actions-per-trigger")} onClick={() => updateTrigger(triggerIndex, { actions: [...trigger.actions, defaultTriggerAction("timeline-mark")] })}><Plus size={14} />{t("add-action")}</button>
            </div>
          </section>
        );
      })}
      <button type="button" className="trigger-add" disabled={!canAddTrigger(draft.triggers.length)} title={canAddTrigger(draft.triggers.length) ? t("add-trigger") : t("at-most-64-triggers-per-session")} onClick={() => setTriggers([...draft.triggers, createDefaultTrigger()])}><Plus size={14} />{t("add-trigger")}</button>
    </div>
  );
}

function SshAdvancedFields({
  section,
  draft,
  prepareProfile,
  onDraftChange,
  proxyPasswordUpdate,
  onProxyPasswordUpdateChange,
  writeBusy,
  onWriteStart,
  onSecretCreated,
  onWriteFinish,
  selectedIdentityId,
  onSelectedIdentityIdChange,
  onOpenClientKeyManager,
}: {
  section: string;
  draft: SessionProfile;
  prepareProfile: (profile: SessionProfile) => SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  proxyPasswordUpdate: ProxyPasswordUpdate;
  onProxyPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
  writeBusy: boolean;
  onWriteStart: () => number | null;
  onSecretCreated: (secretRef: string, token: number) => boolean;
  onWriteFinish: (token: number) => void;
  selectedIdentityId: string;
  onSelectedIdentityIdChange: (id: string) => void;
  onOpenClientKeyManager?: () => Promise<void>;
}) {
  useLocale();
  const ssh = draft.connection.kind === "ssh" || draft.connection.kind === "tmux" ? draft.connection : createSshConnection();
  const kind = draft.connection.kind === "tmux" ? "tmux" : "ssh";
  const [vaultPrivateKey, setVaultPrivateKey] = useState("");
  const [vaultStatus, setVaultStatus] = useState("");
  const [vaultBusy, setVaultBusy] = useState(false);
  const [secretStatus, setSecretStatus] = useState("");
  const [hostKeyScan, setHostKeyScan] = useState<HostKeyScanResult | null>(null);
  const [hostKeyStatus, setHostKeyStatus] = useState("");
  const [hostKeyBusy, setHostKeyBusy] = useState(false);
  const [sshHealth, setSshHealth] = useState<SshHealthReport | null>(null);
  const [sshHealthBusy, setSshHealthBusy] = useState(false);
  const [sshHealthError, setSshHealthError] = useState("");
  const [jumpSecretDrafts, setJumpSecretDrafts] = useState<Record<string, string>>({});
  const [jumpStatus, setJumpStatus] = useState("");
  const [identityManagerBusy, setIdentityManagerBusy] = useState(false);
  const effectiveIdentityId = selectedIdentityId || ssh.identityRefs[0]?.id || "";
  const requestGate = useRef(new KeyedRequestGate<"health" | "host-key">());
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const hostKeyRequestKey = hostKeyProfileRequestKey(prepareProfile(draft));
  const hostKeyRequestKeyRef = useRef(hostKeyRequestKey);
  hostKeyRequestKeyRef.current = hostKeyRequestKey;
  const sshHealthRequestKey = sshHealthProfileRequestKey(prepareProfile(draft));
  const sshHealthRequestKeyRef = useRef(sshHealthRequestKey);
  sshHealthRequestKeyRef.current = sshHealthRequestKey;

  useEffect(() => {
    requestGate.current.invalidate("health");
    setSshHealthBusy(false);
    setSshHealth(null);
    setSshHealthError("");
  }, [sshHealthRequestKey]);

  useEffect(() => {
    requestGate.current.invalidate("host-key");
    setHostKeyBusy(false);
    setHostKeyScan(null);
    setHostKeyStatus("");
  }, [hostKeyRequestKey]);

  useEffect(() => () => requestGate.current.invalidateAll(), []);

  if (section === "connect") {
    const updateSsh = (patch: Partial<typeof ssh>) => onDraftChange({
      ...draft,
      kind,
      connection: { ...ssh, ...patch, kind },
    });
    const updateJump = (index: number, patch: Partial<JumpHop>) => {
      const jumps = ssh.jumps.map((jump, jumpIndex) => (jumpIndex === index ? { ...jump, ...patch } : jump));
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, jumps } });
    };
    const addJump = () => {
      const next: JumpHop = { host: "", port: 22, username: ssh.username, passwordSecretRef: null, passphraseSecretRef: null, identityRef: null, hostKeyPolicy: null };
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, jumps: [...ssh.jumps, next] } });
    };
    const removeJump = (index: number) => {
      setJumpSecretDrafts((current) => removeJumpSecretDraftIndex(current, index));
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, jumps: ssh.jumps.filter((_, jumpIndex) => jumpIndex !== index) } });
    };
    const updateJumpPolicy = (index: number, patch: Partial<HostKeyPolicy>) => {
      const jump = ssh.jumps[index];
      if (!jump) return;
      updateJump(index, { hostKeyPolicy: { ...createJumpHostKeyPolicy(jump), ...(jump.hostKeyPolicy ?? {}), ...patch } });
    };
    const jumpSecretKey = (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => `${index}:${field}`;
    const setJumpSecretDraft = (index: number, field: "passwordSecretRef" | "passphraseSecretRef", value: string) => {
      setJumpSecretDrafts((current) => ({ ...current, [jumpSecretKey(index, field)]: value }));
    };
    const saveJumpSecret = async (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => {
      if (writeBusy) return;
      const jump = ssh.jumps[index];
      if (!jump) return;
      const secret = jumpSecretDrafts[jumpSecretKey(index, field)] ?? "";
      if (!secret.trim()) return;
      const writeToken = onWriteStart();
      if (writeToken === null) return;
      setJumpStatus("");
      try {
        const response = await invokeBackend<{ secretRef: string }>("save_secret", {
          request: { secretRef: null, secret, storage: "portable" },
        });
        if (!onSecretCreated(response.secretRef, writeToken)) {
          await invokeBackend("delete_secret", { secretRef: response.secretRef });
          return;
        }
        const patch: Partial<JumpHop> = field === "passwordSecretRef" ? { passwordSecretRef: response.secretRef } : { passphraseSecretRef: response.secretRef };
        updateJump(index, patch);
        setJumpSecretDrafts((current) => ({ ...current, [jumpSecretKey(index, field)]: "" }));
        setJumpStatus(t("jump-host-credentials-saved"));
      } catch (error) {
        setJumpStatus(formatError(error));
      } finally {
        onWriteFinish(writeToken);
      }
    };
    const deleteJumpSecret = (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => {
      const jump = ssh.jumps[index];
      const secretRef = field === "passwordSecretRef" ? jump?.passwordSecretRef : jump?.passphraseSecretRef;
      if (!secretRef) return;
      setJumpStatus("");
      const patch: Partial<JumpHop> = field === "passwordSecretRef" ? { passwordSecretRef: null } : { passphraseSecretRef: null };
      updateJump(index, patch);
      setJumpStatus(t("unreferenced-credentials-will-be-removed-after-saving-the-profile"));
    };
    return (
      <>
        <DialogField label={t("host-h")}>
          <input
            value={ssh.endpoint.host}
            onChange={(event) => updateSsh({ endpoint: { ...ssh.endpoint, host: event.target.value } })}
          />
        </DialogField>
        <DialogField label={t("username-u")}>
          <input value={ssh.username} autoComplete="username" onChange={(event) => updateSsh({ username: event.target.value })} />
        </DialogField>
        <DialogField label={t("port-p")}>
          <input type="number" min={1} max={65535} value={ssh.endpoint.port} onChange={(event) => updateSsh({ endpoint: { ...ssh.endpoint, port: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label={t("alias-a")}>
          <input value={ssh.hostKeyPolicy.alias ?? ""} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, alias: event.target.value || null } } })} />
        </DialogField>
        <DialogField label={t("ui-jump-host")} group>
          <div className="jump-list">
            {ssh.jumps.map((jump, index) => {
              const policy = jump.hostKeyPolicy ?? createJumpHostKeyPolicy(jump);
              return (
                <div className="jump-hop" key={index}>
                  <div className="jump-hop-row">
                    <span className="jump-hop-index">{index + 1}</span>
                    <input value={jump.host} onChange={(event) => updateJump(index, { host: event.target.value })} placeholder={t("ui-host-2")} />
                    <input type="number" value={jump.port} onChange={(event) => updateJump(index, { port: Number(event.target.value) || 22 })} aria-label={`Jump ${index + 1} port`} />
                    <input value={jump.username} onChange={(event) => updateJump(index, { username: event.target.value })} placeholder={t("ui-user")} />
                    <select
                      value={jump.identityRef ?? ""}
                      onChange={(event) => updateJump(index, { identityRef: event.target.value || null })}
                      aria-label={`Jump ${index + 1} client identity`}
                      title={t("select-a-client-identity-for-this-jump-host-leave")}
                    >
                      <option value="">{t("inherit-profile-identity")}</option>
                      {ssh.identityRefs.map((identity) => (
                        <option key={identity.id} value={identity.id}>
                          {identity.label} · {identitySourceLabel(identity.source)}
                        </option>
                      ))}
                    </select>
                    <button type="button" className="icon-button" onClick={() => removeJump(index)} title={t("delete-jump-host")} aria-label={t("delete-jump-host-2", [index + 1])}>
                      <Trash2 size={14} />
                    </button>
                  </div>
                  <div className="jump-hop-extra">
                    <input type="password" value={jumpSecretDrafts[jumpSecretKey(index, "passwordSecretRef")] ?? ""} onChange={(event) => setJumpSecretDraft(index, "passwordSecretRef", event.target.value)} placeholder={t("ui-password")} />
                    <button type="button" className="icon-button" onClick={() => void saveJumpSecret(index, "passwordSecretRef")} title={t("save-jump-host-password")} disabled={writeBusy || !(jumpSecretDrafts[jumpSecretKey(index, "passwordSecretRef")] ?? "").trim()}>
                      <Lock size={14} />
                    </button>
                    <input value={jump.passwordSecretRef ?? ""} onChange={(event) => updateJump(index, { passwordSecretRef: event.target.value || null })} placeholder={t("ui-password-secretref")} />
                    <input type="password" value={jumpSecretDrafts[jumpSecretKey(index, "passphraseSecretRef")] ?? ""} onChange={(event) => setJumpSecretDraft(index, "passphraseSecretRef", event.target.value)} placeholder={t("ui-passphrase")} />
                    <button type="button" className="icon-button" onClick={() => void saveJumpSecret(index, "passphraseSecretRef")} title={t("save-jump-host-passphrase")} disabled={writeBusy || !(jumpSecretDrafts[jumpSecretKey(index, "passphraseSecretRef")] ?? "").trim()}>
                      <Lock size={14} />
                    </button>
                    <input value={jump.passphraseSecretRef ?? ""} onChange={(event) => updateJump(index, { passphraseSecretRef: event.target.value || null })} placeholder={t("ui-passphrase-secretref")} />
                    <button type="button" className="icon-button" onClick={() => void deleteJumpSecret(index, "passwordSecretRef")} disabled={!jump.passwordSecretRef} title={t("delete-jump-host-password")}>
                      <X size={14} />
                    </button>
                    <button type="button" className="icon-button" onClick={() => void deleteJumpSecret(index, "passphraseSecretRef")} disabled={!jump.passphraseSecretRef} title={t("delete-jump-host-passphrase")}>
                      <X size={14} />
                    </button>
                  </div>
                  <div className="jump-hop-policy">
                    <select value={jump.hostKeyPolicy ? "custom" : "inherit"} onChange={(event) => updateJump(index, { hostKeyPolicy: event.target.value === "custom" ? createJumpHostKeyPolicy(jump) : null })}>
                      <option value="inherit">{t("inherit")}</option>
                      <option value="custom">{t("custom")}</option>
                    </select>
                    {jump.hostKeyPolicy ? (
                      <>
                        <select value={policy.mode} onChange={(event) => updateJumpPolicy(index, { mode: event.target.value as HostKeyPolicy["mode"] })}>
                          <option value="strict">{t("ui-strict")}</option>
                          <option value="trust-on-first-use">{t("ui-trust-on-first-use")}</option>
                          <option value="ask-every-time">{t("ui-ask-every-time")}</option>
                        </select>
                        <input value={policy.alias ?? ""} onChange={(event) => updateJumpPolicy(index, { alias: event.target.value || null })} placeholder={t("ui-host-key-alias")} />
                        <select value={policy.trustScope} onChange={(event) => updateJumpPolicy(index, { trustScope: event.target.value as HostKeyPolicy["trustScope"] })}>
                          <option value="profile">{t("ui-profile")}</option>
                          <option value="project">{t("ui-project")}</option>
                          <option value="user">{t("ui-user")}</option>
                        </select>
                        <label className="jump-hop-check">
                          <input type="checkbox" checked={policy.allowRotation} onChange={(event) => updateJumpPolicy(index, { allowRotation: event.target.checked })} />
                          <span>{t("rotate")}</span>
                        </label>
                        <label className="jump-hop-check">
                          <input type="checkbox" checked={policy.checkIp} onChange={(event) => updateJumpPolicy(index, { checkIp: event.target.checked })} />
                          <span>IP</span>
                        </label>
                      </>
                    ) : null}
                  </div>
                </div>
              );
            })}
            {jumpStatus ? <span className="settings-inline-status">{jumpStatus}</span> : null}
            <button type="button" className="settings-secondary-button jump-add-button" onClick={addJump}>
              <Plus size={14} />
              <span>{t("add-jump-host")}</span>
            </button>
          </div>
        </DialogField>
        <DialogToggleField label={t("ssh-keepalive")} checked={ssh.keepaliveEnabled} onChange={(keepaliveEnabled) => updateSsh({ keepaliveEnabled })} />
        {ssh.keepaliveEnabled ? (
          <>
            <DialogField label={t("probe-interval-s")}>
              <input
                type="number"
                min={sshConnectionBounds.keepaliveIntervalSeconds.min}
                max={sshConnectionBounds.keepaliveIntervalSeconds.max}
                value={ssh.keepaliveIntervalSeconds}
                onChange={(event) => updateSsh({ keepaliveIntervalSeconds: Number(event.target.value) })}
              />
            </DialogField>
            <DialogField label={t("unanswered-limit-0-no-automatic-disconnect")}>
              <input
                type="number"
                min={sshConnectionBounds.keepaliveMaxMissed.min}
                max={sshConnectionBounds.keepaliveMaxMissed.max}
                value={ssh.keepaliveMaxMissed}
                onChange={(event) => updateSsh({ keepaliveMaxMissed: Number(event.target.value) })}
              />
            </DialogField>
          </>
        ) : null}
        <DialogField label={t("ui-tcp-keepalive")}>
          <select
            value={ssh.tcpKeepaliveEnabled === null ? "system" : ssh.tcpKeepaliveEnabled ? "enabled" : "disabled"}
            onChange={(event) => updateSsh({
              tcpKeepaliveEnabled: event.target.value === "system"
                ? null
                : event.target.value === "enabled",
            })}
          >
            <option value="system">{t("system-default")}</option>
            <option value="enabled">{t("enabled")}</option>
            <option value="disabled">{t("close")}</option>
          </select>
        </DialogField>
        <DialogToggleField label={t("auto-reconnect")} checked={ssh.reconnect} onChange={(reconnect) => updateSsh({ reconnect })} />
        <DialogField label={t("reconnect-delay-ms")}>
          <input
            type="number"
            min={sshConnectionBounds.reconnectDelayMs.min}
            max={sshConnectionBounds.reconnectDelayMs.max}
            step={100}
            disabled={!ssh.reconnect}
            value={ssh.reconnectDelayMs}
            onChange={(event) => updateSsh({ reconnectDelayMs: Number(event.target.value) })}
          />
        </DialogField>
      </>
    );
  }

  if (section === "proxy") {
    return <ProxyAdvancedFields proxy={ssh.proxy} onChange={(proxy) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, proxy } })} passwordUpdate={proxyPasswordUpdate} onPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if (section === "verification") {
    const checkHealth = async () => {
      const token = requestGate.current.begin("health");
      if (token === null) return;
      const profile = prepareProfile(draftRef.current);
      const sessionId = profile.id;
      const requestKey = sshHealthProfileRequestKey(profile);
      setSshHealthBusy(true);
      setSshHealth(null);
      setSshHealthError("");
      try {
        const report = await invokeBackend<SshHealthReport>("check_ssh_health", {
          sessionId,
          probeSftp: true,
          expectedProfile: profile,
        });
        if (requestGate.current.isCurrent("health", token) && sshHealthRequestKeyRef.current === requestKey) {
          setSshHealth(report);
        }
      } catch (error) {
        if (requestGate.current.isCurrent("health", token) && sshHealthRequestKeyRef.current === requestKey) {
          setSshHealthError(formatError(error));
        }
      } finally {
        if (requestGate.current.finish("health", token)) setSshHealthBusy(false);
      }
    };
    const scanHostKey = async () => {
      if (writeBusy) return;
      const token = requestGate.current.begin("host-key");
      if (token === null) return;
      const requestKey = hostKeyRequestKeyRef.current;
      const profile = prepareProfile(draftRef.current);
      setHostKeyBusy(true);
      setHostKeyStatus("");
      setHostKeyScan(null);
      try {
        const result = await invokeBackend<HostKeyScanResult>("scan_ssh_host_key", {
          request: { profile, credentialHandle: null },
        });
        if (requestGate.current.isCurrent("host-key", token) && hostKeyRequestKeyRef.current === requestKey) {
          setHostKeyScan(result);
        }
      } catch (error) {
        if (requestGate.current.isCurrent("host-key", token) && hostKeyRequestKeyRef.current === requestKey) {
          setHostKeyStatus(formatError(error));
        }
      } finally {
        if (requestGate.current.finish("host-key", token)) setHostKeyBusy(false);
      }
    };
    const trustHostKey = async (decision: DraftHostKeyDecisionValue) => {
      if (!hostKeyScan || writeBusy) return;
      const token = requestGate.current.begin("host-key");
      if (token === null) return;
      const requestKey = hostKeyRequestKeyRef.current;
      const scan = hostKeyScan;
      const profile = prepareProfile(draftRef.current);
      setHostKeyBusy(true);
      setHostKeyStatus("");
      try {
        const response = await invokeBackend<DraftHostKeyDecisionResponse>("prepare_scanned_host_key_draft", {
          request: { profile, observation: scan.observation, decision },
        });
        if (!requestGate.current.isCurrent("host-key", token) || hostKeyRequestKeyRef.current !== requestKey) return;
        const currentDraft = draftRef.current;
        if (currentDraft.connection.kind !== "ssh" && currentDraft.connection.kind !== "tmux") return;
        onDraftChange({
          ...currentDraft,
          kind: currentDraft.connection.kind,
          connection: {
            ...currentDraft.connection,
            trustedHostKeys: response.trustedHostKeys,
          },
        });
        setHostKeyScan(null);
        setHostKeyStatus(t("added-to-profile-draft-takes-effect-after-saving-the", [response.trusted.fingerprintSha256]));
      } catch (error) {
        if (requestGate.current.isCurrent("host-key", token) && hostKeyRequestKeyRef.current === requestKey) {
          setHostKeyStatus(formatError(error));
        }
      } finally {
        if (requestGate.current.finish("host-key", token)) setHostKeyBusy(false);
      }
    };
    return (
      <>
        <DialogField label={t("ui-hostkey")}>
          <select value={ssh.hostKeyPolicy.mode} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, mode: event.target.value as "strict" | "trust-on-first-use" | "ask-every-time" } } })}>
            <option value="strict">{t("ui-strict")}</option>
            <option value="trust-on-first-use">{t("ui-trust-on-first-use")}</option>
            <option value="ask-every-time">{t("ui-ask-every-time")}</option>
          </select>
        </DialogField>
        <DialogField label={t("rotation-r")}>
          <select value={ssh.hostKeyPolicy.allowRotation ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, allowRotation: event.target.value === "on" } } })}>
            <option value="off">{t("block-changes")}</option>
            <option value="on">{t("allow-appending")}</option>
          </select>
        </DialogField>
        <DialogField label={t("verify-ip-i")}>
          <select value={ssh.hostKeyPolicy.checkIp ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, checkIp: event.target.value === "on" } } })}>
            <option value="off">{t("close")}</option>
            <option value="on">{t("enabled")}</option>
          </select>
        </DialogField>
        <DialogField label={t("trust-scope-s")}>
          <select value={ssh.hostKeyPolicy.trustScope} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, trustScope: event.target.value as "profile" | "project" | "user" } } })}>
            <option value="profile">{t("ui-profile")}</option>
            <option value="project">{t("ui-project")}</option>
            <option value="user">{t("ui-user")}</option>
          </select>
        </DialogField>
        <DialogField label={t("scan-2")} group>
          <div className="inline-actions">
            <button type="button" onClick={() => void scanHostKey()} disabled={hostKeyBusy || writeBusy}>{hostKeyBusy ? t("scanning-2") : t("scan-host-key")}</button>
            <span>{hostKeyScan ? describeHostKeyEvaluation(hostKeyScan) : hostKeyStatus}</span>
          </div>
        </DialogField>
        <DialogField label={t("health")} group>
          <div className="inline-actions ssh-health-check">
            <button type="button" aria-label={t("check-ssh-health")} onClick={() => void checkHealth()} disabled={sshHealthBusy}>
              <Activity size={14} />{sshHealthBusy ? t("checking") : t("check-ssh-health")}
            </button>
            <span className={sshHealth?.status === "healthy" ? "healthy" : sshHealth ? "degraded" : ""} title={sshHealth ? sshHealthDiagnostic(sshHealth) : localizeDiagnostic(sshHealthError)}>
              {sshHealth ? sshHealthSummary(sshHealth) : localizeDiagnostic(sshHealthError)}
            </span>
          </div>
        </DialogField>
        {hostKeyScan ? (
          <DialogField label={t("action-2")} group>
            <div className="inline-actions">
              <button type="button" onClick={() => void trustHostKey("append-to-profile")} disabled={hostKeyBusy || writeBusy}>{t("add-to-profile")}</button>
              <button type="button" onClick={() => void trustHostKey("replace-for-profile")} disabled={hostKeyBusy || writeBusy}>{t("replace-profile")}</button>
            </div>
          </DialogField>
        ) : null}
      </>
    );
  }

  if (section === "ssh-agent") {
    return (
      <>
        <DialogField label={t("ui-agent-a")}>
          <select value={ssh.agentPolicy.enabled ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, agentPolicy: { ...ssh.agentPolicy, enabled: event.target.value === "on" } } })}>
            <option value="off">{t("disable")}</option>
            <option value="on">{t("enable")}</option>
          </select>
        </DialogField>
        <DialogField label={t("ui-forward-f")}>
          <select value={ssh.agentPolicy.forwarding ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, agentPolicy: { ...ssh.agentPolicy, forwarding: event.target.value === "on" } } })}>
            <option value="off">{t("disable")}</option>
            <option value="on">{t("enable")}</option>
          </select>
        </DialogField>
        <DialogField label={t("ui-offer-o")}>
          <select value={ssh.agentPolicy.offerMode} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, agentPolicy: { ...ssh.agentPolicy, offerMode: event.target.value as "disabled" | "after-profile-keys" | "before-profile-keys" } } })}>
            <option value="disabled">{t("ui-disabled")}</option>
            <option value="after-profile-keys">{t("ui-after-profile-keys")}</option>
            <option value="before-profile-keys">{t("ui-before-profile-keys")}</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (section === "password") {
    const deleteSavedSecret = (field: "passwordSecretRef" | "passphraseSecretRef") => {
      const secretRef = ssh[field];
      if (!secretRef) return;
      setSecretStatus("");
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, [field]: null } });
      setSecretStatus(t("unreferenced-credentials-will-be-removed-after-saving-the-profile"));
    };
    return (
      <>
        <DialogField label={t("password-reference")}>
          <div className="inline-actions">
            <input value={ssh.passwordSecretRef ?? ""} readOnly placeholder={t("not-saved")} />
            <button type="button" onClick={() => void deleteSavedSecret("passwordSecretRef")} disabled={!ssh.passwordSecretRef}>{t("delete")}</button>
          </div>
        </DialogField>
        <DialogField label={t("passphrase-reference")}>
          <div className="inline-actions">
            <input value={ssh.passphraseSecretRef ?? ""} readOnly placeholder={t("not-saved")} />
            <button type="button" onClick={() => void deleteSavedSecret("passphraseSecretRef")} disabled={!ssh.passphraseSecretRef}>{t("delete")}</button>
          </div>
        </DialogField>
        <DialogField label={t("status-2")}>
          <input value={secretStatus} readOnly placeholder={t("a-reference-is-created-when-you-select-save-in")} />
        </DialogField>
      </>
    );
  }

  if (section === "public-key") {
    const identities = ssh.identityRefs;
    const selectedIdentity = identities.find((identity) => identity.id === effectiveIdentityId)
      ?? identities[0]
      ?? createIdentityRef();
    const selectedIdentityIndex = identities.findIndex((identity) => identity.id === selectedIdentity.id);
    const authOrderValue = ssh.identityPolicy.authOrder.join(">");
    const authOrderIsPreset = SSH_AUTH_ORDER_OPTIONS.some((option) => option === authOrderValue);
    const authOrderOptions: readonly string[] = authOrderIsPreset
      ? SSH_AUTH_ORDER_OPTIONS
      : [authOrderValue, ...SSH_AUTH_ORDER_OPTIONS];
    const updateIdentity = (patch: Partial<IdentityRef>) => {
      const identity = { ...selectedIdentity, ...patch };
      const nextIdentities = selectedIdentityIndex < 0
        ? [identity, ...identities]
        : identities.map((current, index) => index === selectedIdentityIndex ? identity : current);
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityRefs: nextIdentities } });
    };
    const addIdentity = () => {
      const identity = createIdentityRef();
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityRefs: [identity, ...identities] } });
      onSelectedIdentityIdChange(identity.id);
    };
    const removeIdentity = () => {
      if (selectedIdentityIndex < 0) return;
      const nextIdentities = identities.filter((_, index) => index !== selectedIdentityIndex);
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityRefs: nextIdentities } });
      onSelectedIdentityIdChange(nextIdentities[0]?.id ?? "");
    };
    const choosePrivateKey = async () => {
      const path = await chooseSshPrivateKeyPath(selectedIdentity.path ?? "");
      if (path) updateIdentity({ path, source: "system-file", secretRef: null });
    };
    const saveVaultPrivateKey = async () => {
      if (writeBusy || !vaultPrivateKey.trim()) return;
      const writeToken = onWriteStart();
      if (writeToken === null) return;
      setVaultBusy(true);
      setVaultStatus("");
      try {
        const response = await invokeBackend<{ secretRef: string }>("save_secret", {
          request: { secretRef: null, secret: vaultPrivateKey, storage: "portable" },
        });
        if (!onSecretCreated(response.secretRef, writeToken)) {
          await invokeBackend("delete_secret", { secretRef: response.secretRef });
          return;
        }
        updateIdentity({ source: "profile-vault", secretRef: response.secretRef, path: null });
        setVaultPrivateKey("");
        setVaultStatus(t("saved-to-stronghold"));
      } catch (error) {
        setVaultStatus(formatError(error));
      } finally {
        setVaultBusy(false);
        onWriteFinish(writeToken);
      }
    };
    const deleteVaultPrivateKey = () => {
      if (!selectedIdentity.secretRef) return;
      setVaultBusy(true);
      setVaultStatus("");
      updateIdentity({ secretRef: null });
      setVaultStatus(t("unreferenced-private-keys-will-be-removed-after-saving-the"));
      setVaultBusy(false);
    };
    return (
      <>
        <DialogField label={t("identity-i")}>
          <select value={ssh.identityPolicy.identitiesOnly ? "only" : "agent"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityPolicy: { ...ssh.identityPolicy, identitiesOnly: event.target.value === "only" } } })}>
            <option value="only">{t("ui-identitiesonly")}</option>
            <option value="agent">{t("ui-profile-agent")}</option>
          </select>
        </DialogField>
        <DialogField label={t("order-o")}>
          <select value={ssh.identityPolicy.authOrder.join(">")} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityPolicy: { ...ssh.identityPolicy, authOrder: event.target.value.split(">") as AuthMethod[] } } })}>
            {authOrderOptions.map((option, index) => (
              <option key={option} value={option}>
                {option.replaceAll(">", " > ")}{!authOrderIsPreset && index === 0 ? t("current-settings") : ""}
              </option>
            ))}
          </select>
        </DialogField>
        <DialogToggleField
          label={t("remember-successful-method-r")}
          checked={ssh.identityPolicy.recordSuccess}
          onChange={(recordSuccess) => onDraftChange({
            ...draft,
            kind,
            connection: {
              ...ssh,
              kind,
              identityPolicy: {
                ...ssh.identityPolicy,
                recordSuccess,
                lastSuccessful: recordSuccess ? ssh.identityPolicy.lastSuccessful : null,
              },
            },
          })}
        />
        <DialogField label={t("public-key-k-client-identity")}>
          <select value={selectedIdentity.source} onChange={(event) => updateIdentity({ source: event.target.value as IdentityRef["source"], ...(event.target.value === "system-file" ? {} : { path: null }), ...(event.target.value === "profile-vault" ? {} : { secretRef: null }) })} disabled={selectedIdentityIndex < 0} aria-label={t("client-identity-source")}>
            <option value="system-file">{t("local-private-key-file")}</option>
            <option value="profile-vault">{t("ui-profile-vault-stronghold")}</option>
            <option value="agent">{t("ssh-agent-identity")}</option>
            <option value="public-key-only">{t("public-key-information-only")}</option>
          </select>
        </DialogField>
        <div className="session-identity-selector" role="group" aria-label={t("client-identity-selection")}>
          <span>{t("identity-settings")}</span>
          <div className="inline-actions">
            <select value={selectedIdentity.id} onChange={(event) => onSelectedIdentityIdChange(event.target.value)} disabled={!identities.length} aria-label={t("client-identity-selection")}>
              {!identities.length ? <option value="">{t("no-identity-configured")}</option> : null}
              {identities.map((identity) => <option key={identity.id} value={identity.id}>{identity.label} · {identitySourceLabel(identity.source)}</option>)}
            </select>
            <button type="button" onClick={addIdentity} title={t("add-client-identity")}>{t("add")}</button>
            <button type="button" onClick={removeIdentity} disabled={selectedIdentityIndex < 0} title={t("remove-current-identity-reference")}>{t("remove")}</button>
          </div>
        </div>
        <div className="session-identity-hint" role="note">
          <strong>{identities.length ? t("identities-tried-in-order") : t("no-identities-available-for-public-key-authentication")}</strong>
          <span>{identities.length ? t("select-profile-vault-local-private-key-or-ssh-agent") : t("add-a-local-private-key-below-or-open-client")}</span>
          {onOpenClientKeyManager ? (
            <button type="button" onClick={async () => {
              setIdentityManagerBusy(true);
              try { await onOpenClientKeyManager(); } finally { setIdentityManagerBusy(false); }
            }} disabled={identityManagerBusy || writeBusy}>
              {identityManagerBusy ? t("opening-identity-manager") : t("manage-client-identities")}
            </button>
          ) : null}
        </div>
        <DialogField label={t("name-n")}>
          <input value={selectedIdentity.label} onChange={(event) => updateIdentity({ label: event.target.value })} disabled={selectedIdentityIndex < 0} />
        </DialogField>
        <DialogField label={t("private-key-file-f")}>
          <div className="inline-actions">
            <input value={selectedIdentity.path ?? ""} onChange={(event) => updateIdentity({ path: event.target.value || null, source: event.target.value ? "system-file" : selectedIdentity.source })} placeholder="~/.ssh/id_ed25519" disabled={selectedIdentity.source !== "system-file"} />
            <button type="button" onClick={() => void choosePrivateKey()} disabled={selectedIdentityIndex < 0} title={t("select-local-ssh-private-key-file")}>{t("select-2")}</button>
          </div>
        </DialogField>
        <DialogField label={t("ui-vault-ref")}>
          <input value={selectedIdentity.secretRef ? t("securely-saved-in-stronghold") : ""} readOnly placeholder={t("no-private-key-saved")} />
        </DialogField>
        {selectedIdentity.source === "profile-vault" ? (
          <DialogField label={t("private-key-content")}>
            <textarea value={vaultPrivateKey} onChange={(event) => setVaultPrivateKey(event.target.value)} placeholder={t("paste-an-openssh-private-key-only-secretref-is-retained")} />
          </DialogField>
        ) : null}
        {selectedIdentity.source === "profile-vault" ? (
          <DialogField label={t("vault")}>
            <div className="inline-actions">
              <button type="button" onClick={() => void saveVaultPrivateKey()} disabled={vaultBusy || writeBusy || !vaultPrivateKey.trim()}>{t("save-to-stronghold")}</button>
              <button type="button" onClick={() => void deleteVaultPrivateKey()} disabled={vaultBusy || !selectedIdentity.secretRef}>{t("delete")}</button>
              <span>{vaultStatus}</span>
            </div>
          </DialogField>
        ) : null}
        <DialogField label={t("fingerprint-p")}>
          <input value={selectedIdentity.fingerprintSha256 ?? ""} onChange={(event) => updateIdentity({ fingerprintSha256: event.target.value || null })} placeholder="SHA256:..." disabled={selectedIdentityIndex < 0} />
        </DialogField>
      </>
    );
  }

  return null;
}

function ProxyAdvancedFields({
  proxy,
  onChange,
  passwordUpdate,
  onPasswordUpdateChange,
}: {
  proxy: ProxyConfig;
  onChange: (proxy: ProxyConfig) => void;
  passwordUpdate: ProxyPasswordUpdate;
  onPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
}) {
  useLocale();
  const update = (patch: Partial<ProxyConfig>) => onChange({ ...proxy, ...patch });
  const password = passwordUpdate?.action === "set" ? passwordUpdate.password : "";
  const passwordPendingClear = passwordUpdate?.action === "clear";
  return (
    <>
      <DialogToggleField label={t("enable-proxy")} checked={proxy.enabled} onChange={(enabled) => update({ enabled })} />
      {proxy.enabled ? (
        <>
          <DialogField label={t("protocol")}>
            <select value={proxy.kind} onChange={(event) => update({ kind: event.target.value as ProxyConfig["kind"] })}>
              <option value="socks5">SOCKS5</option>
              <option value="http-connect">HTTP CONNECT</option>
            </select>
          </DialogField>
          <DialogField label={t("proxy-host")}>
            <input value={proxy.host} onChange={(event) => update({ host: event.target.value })} />
          </DialogField>
          <DialogField label={t("proxy-port")}>
            <input type="number" min={1} max={65535} value={proxy.port} onChange={(event) => update({ port: Number(event.target.value) })} />
          </DialogField>
          <DialogField label={t("proxy-username")}>
            <input value={proxy.username} autoComplete="username" onChange={(event) => update({ username: event.target.value })} />
          </DialogField>
          <DialogField label={t("proxy-password")}>
            <form className="proxy-password-control" onSubmit={(event) => event.preventDefault()}>
              <input type="text" name="username" autoComplete="username" value={proxy.username} readOnly hidden aria-hidden="true" tabIndex={-1} />
              <input
                type="password"
                name="password"
                autoComplete="new-password"
                value={password}
                placeholder={passwordPendingClear ? t("remove-after-saving") : proxy.passwordSecretRef ? t("securely-saved") : t("not-saved")}
                onChange={(event) => onPasswordUpdateChange(event.target.value ? { action: "set", password: event.target.value, storage: "portable" } : null)}
              />
              <button
                type="button"
                className="icon-button"
                title={t("remove-saved-proxy-password")}
                aria-label={t("remove-saved-proxy-password")}
                disabled={passwordPendingClear || (!proxy.passwordSecretRef && passwordUpdate?.action !== "set")}
                onClick={() => onPasswordUpdateChange({ action: "clear" })}
              >
                <X size={14} />
              </button>
            </form>
          </DialogField>
        </>
      ) : null}
    </>
  );
}

function TcpLikeAdvancedFields({
  protocol,
  section,
  draft,
  onDraftChange,
  proxyPasswordUpdate,
  onProxyPasswordUpdateChange,
}: {
  protocol: "Telnet" | "Tcp";
  section: string;
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  proxyPasswordUpdate: ProxyPasswordUpdate;
  onProxyPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
}) {
  useLocale();
  const kind = protocol === "Telnet" ? "telnet" : "tcp";
  const tcp = draft.connection.kind === kind ? draft.connection : createTcpConnection(kind);

  if (section === "connect") {
    const updateTcp = (patch: Partial<typeof tcp>) => onDraftChange({
      ...draft,
      kind,
      connection: { ...tcp, ...patch, kind },
    });
    return (
      <>
        <DialogField label={t("host-h")}>
          <input value={tcp.host} onChange={(event) => updateTcp({ host: event.target.value })} />
        </DialogField>
        <DialogField label={t("port-p")}>
          <input type="number" min={1} max={65535} value={tcp.port} onChange={(event) => updateTcp({ port: Number(event.target.value) })} />
        </DialogField>
        <DialogToggleField label={t("auto-reconnect")} checked={tcp.reconnect} onChange={(reconnect) => updateTcp({ reconnect })} />
        <DialogField label={t("reconnect-delay-ms")}>
          <input
            type="number"
            min={tcpConnectionBounds.reconnectDelayMs.min}
            max={tcpConnectionBounds.reconnectDelayMs.max}
            step={100}
            disabled={!tcp.reconnect}
            value={tcp.reconnectDelayMs}
            onChange={(event) => updateTcp({ reconnectDelayMs: Number(event.target.value) })}
          />
        </DialogField>
        <DialogToggleField label={t("ui-tcp-keepalive")} checked={tcp.keepaliveEnabled} onChange={(keepaliveEnabled) => updateTcp({ keepaliveEnabled })} />
        {tcp.keepaliveEnabled ? (
          <>
            <DialogField label={t("idle-time-s")}>
              <input
                type="number"
                min={tcpConnectionBounds.keepaliveIdleSeconds.min}
                max={tcpConnectionBounds.keepaliveIdleSeconds.max}
                value={tcp.keepaliveIdleSeconds}
                onChange={(event) => updateTcp({ keepaliveIdleSeconds: Number(event.target.value) })}
              />
            </DialogField>
            <DialogField label={t("probe-interval-s")}>
              <input
                type="number"
                min={tcpConnectionBounds.keepaliveIntervalSeconds.min}
                max={tcpConnectionBounds.keepaliveIntervalSeconds.max}
                value={tcp.keepaliveIntervalSeconds}
                onChange={(event) => updateTcp({ keepaliveIntervalSeconds: Number(event.target.value) })}
              />
            </DialogField>
            <DialogField label={t("failure-count")}>
              <input
                type="number"
                min={tcpConnectionBounds.keepaliveRetries.min}
                max={tcpConnectionBounds.keepaliveRetries.max}
                value={tcp.keepaliveRetries}
                onChange={(event) => updateTcp({ keepaliveRetries: Number(event.target.value) })}
              />
            </DialogField>
          </>
        ) : null}
        {protocol === "Telnet" ? (
          <>
            <DialogToggleField label="BINARY:" checked={tcp.telnetBinary} onChange={(telnetBinary) => updateTcp({ telnetBinary })} />
            <DialogToggleField label="NAWS:" checked={tcp.telnetNaws} onChange={(telnetNaws) => updateTcp({ telnetNaws })} />
          </>
        ) : null}
        <DialogToggleField label="TLS/SSL:" checked={tcp.tlsEnabled} onChange={(tlsEnabled) => updateTcp({ tlsEnabled })} />
        {tcp.tlsEnabled ? (
          <>
            <DialogField label={t("ui-tls-server-name")}>
              <input
                value={tcp.tlsServerName ?? ""}
                placeholder={tcp.host}
                maxLength={253}
                onChange={(event) => updateTcp({ tlsServerName: event.target.value || null })}
              />
            </DialogField>
            <DialogToggleField
              label={t("accept-invalid-certificates-unsafe")}
              checked={tcp.tlsAcceptInvalidCert}
              onChange={(tlsAcceptInvalidCert) => updateTcp({ tlsAcceptInvalidCert })}
            />
          </>
        ) : null}
      </>
    );
  }

  return <ProxyAdvancedFields proxy={tcp.proxy} onChange={(proxy) => onDraftChange({ ...draft, kind, connection: { ...tcp, kind, proxy } })} passwordUpdate={proxyPasswordUpdate} onPasswordUpdateChange={onProxyPasswordUpdateChange} />;
}

function SerialAdvancedFields({
  draft,
  serialPorts,
  serialPortsRefreshing,
  serialPortsRefreshError,
  onRefreshSerialPorts,
  onDraftChange,
}: {
  draft: SessionProfile;
  serialPorts: string[];
  serialPortsRefreshing: boolean;
  serialPortsRefreshError: string;
  onRefreshSerialPorts: () => void;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  useLocale();
  const serial = draft.connection.kind === "serial" ? draft.connection : createSerialConnection();
  const update = (patch: Partial<ReturnType<typeof createSerialConnection>>) => onDraftChange({ ...draft, kind: "serial", connection: { ...serial, ...patch } });

  return (
    <>
      <DialogField label={t("serial-port-s")}>
        <div className="serial-port-picker">
          <select value={serial.port} onChange={(event) => update({ port: event.target.value })}>
            <option value="">{serialPorts.length ? t("select-serial-port") : t("no-serial-ports-found")}</option>
            {serialPortOptions(serial.port, serialPorts).map((option) => (
              <option key={option} value={option}>{option}</option>
            ))}
          </select>
          <button
            type="button"
            className="serial-port-refresh-button"
            title={t("refresh-device-serial-ports")}
            aria-label={t("refresh-serial-ports")}
            onClick={onRefreshSerialPorts}
            disabled={serialPortsRefreshing}
          >
            <RefreshCw size={14} className={serialPortsRefreshing ? "spin" : ""} />
          </button>
        </div>
      </DialogField>
      {serialPortsRefreshError ? <div className="serial-port-refresh-status" role="status">{localizeDiagnostic(serialPortsRefreshError)}</div> : null}
      <DialogField label={t("baud-rate-b")}>
        <input
          type="number"
          min={serialConnectionBounds.baudRate.min}
          max={serialConnectionBounds.baudRate.max}
          step={1}
          list="serial-baud-rate-options"
          value={serial.baudRate}
          onChange={(event) => update({ baudRate: Number(event.target.value) })}
        />
        <datalist id="serial-baud-rate-options">
          {COMMON_SERIAL_BAUD_RATES.map((baudRate) => <option key={baudRate} value={baudRate} />)}
        </datalist>
      </DialogField>
      <DialogField label={t("data-bits-d")}>
        <select value={serial.dataBits} onChange={(event) => update({ dataBits: Number(event.target.value) })}>
          <option value={5}>5</option>
          <option value={6}>6</option>
          <option value={7}>7</option>
          <option value={8}>8</option>
        </select>
      </DialogField>
      <DialogField label={t("stop-bits-s")}>
        <select value={serial.stopBits} onChange={(event) => update({ stopBits: Number(event.target.value) })}>
          <option value={1}>1</option>
          <option value={2}>2</option>
        </select>
      </DialogField>
      <DialogField label={t("parity-p")}>
        <select value={serial.parity} onChange={(event) => update({ parity: event.target.value })}>
          <option value="none">{t("none")}</option>
          <option value="odd">{t("odd")}</option>
          <option value="even">{t("even")}</option>
        </select>
      </DialogField>
      <DialogField label={t("flow-control-f")}>
        <select value={serial.flowControl} onChange={(event) => update({ flowControl: event.target.value })}>
          <option value="none">{t("none")}</option>
          <option value="software">{t("software")}</option>
          <option value="hardware">{t("hardware")}</option>
        </select>
      </DialogField>
      <DialogField label="DTR:(D)">
        <select value={serial.dtr ? "on" : "off"} onChange={(event) => update({ dtr: event.target.value === "on" })}>
          <option value="off">{t("close")}</option>
          <option value="on">{t("enabled")}</option>
        </select>
      </DialogField>
      <DialogField label="RTS:(R)">
        <select value={serial.rts ? "on" : "off"} onChange={(event) => update({ rts: event.target.value === "on" })}>
          <option value="off">{t("close")}</option>
          <option value="on">{t("enabled")}</option>
        </select>
      </DialogField>
      <DialogToggleField label={t("auto-reconnect")} checked={serial.reconnect} onChange={(reconnect) => update({ reconnect })} />
      <DialogField label={t("reconnect-delay-ms")}>
        <input
          type="number"
          min={serialConnectionBounds.reconnectDelayMs.min}
          max={serialConnectionBounds.reconnectDelayMs.max}
          step={100}
          disabled={!serial.reconnect}
          value={serial.reconnectDelayMs}
          onChange={(event) => update({ reconnectDelayMs: Number(event.target.value) })}
        />
      </DialogField>
      <DialogToggleField label={t("receive-idle-timeout")} checked={serial.receiveIdleTimeoutEnabled} onChange={(receiveIdleTimeoutEnabled) => update({ receiveIdleTimeoutEnabled })} />
      {serial.receiveIdleTimeoutEnabled ? (
        <DialogField label={t("idle-limit-s")}>
          <input
            type="number"
            min={serialConnectionBounds.receiveIdleTimeoutSeconds.min}
            max={serialConnectionBounds.receiveIdleTimeoutSeconds.max}
            value={serial.receiveIdleTimeoutSeconds}
            onChange={(event) => update({ receiveIdleTimeoutSeconds: Number(event.target.value) })}
          />
        </DialogField>
      ) : null}
    </>
  );
}

function DialogFrame({
  title,
  className,
  dataSessionProtocol,
  dataSessionSection,
  onClose,
  closeDisabled = false,
  children,
}: {
  title: string;
  className: string;
  dataSessionProtocol?: string;
  dataSessionSection?: string;
  onClose: () => void;
  closeDisabled?: boolean;
  children: ReactNode;
}) {
  useLocale();
  return (
    <div className="dialog-backdrop">
      <section
        className={`wind-dialog ${className}`}
        data-session-protocol={dataSessionProtocol}
        data-session-section={dataSessionSection}
      >
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{title}</strong>
          <button type="button" aria-label={t("close")} title={t("close")} onClick={onClose} disabled={closeDisabled}><X size={22} /></button>
        </header>
        {children}
      </section>
    </div>
  );
}

function DialogField({ label, children, group = false }: { label: string; children: ReactNode; group?: boolean }) {
  useLocale();
  if (group) {
    return (
      <div className="dialog-field" role="group" aria-label={label}>
        <span>{label}</span>
        {children}
      </div>
    );
  }
  return (
    <label className="dialog-field">
      <span>{label}</span>
      {children}
    </label>
  );
}

function DialogToggleField({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  useLocale();
  return (
    <label className="dialog-field dialog-toggle-field">
      <span>{label}</span>
      <button type="button" className={checked ? "switch-toggle on" : "switch-toggle"} onClick={() => onChange(!checked)} aria-pressed={checked}>
        <span />
      </button>
    </label>
  );
}

function sshHealthSummary(report: SshHealthReport) {
  const label = report.status === "healthy" ? t("healthy") : report.status === "degraded" ? t("partially-degraded") : t("unresponsive");
  const timings = [
    report.transportRoundTripMs == null ? null : `SSH ${report.transportRoundTripMs} ms`,
    report.channelRoundTripMs == null ? null : `Channel ${report.channelRoundTripMs} ms`,
    report.sftpRoundTripMs == null ? null : `SFTP ${report.sftpRoundTripMs} ms`,
  ].filter(Boolean);
  const connection = `${report.backend} · ${sshAuthenticationLabel(report.authenticationMethod)}`;
  return timings.length ? `${label} · ${connection} · ${timings.join(" · ")}` : `${label} · ${connection}`;
}

function sshHealthDiagnostic(report: SshHealthReport) {
  return [report.transportError, report.terminalError, report.channelError, report.sftpError]
    .filter((value): value is string => Boolean(value))
    .join("；") || sshHealthSummary(report);
}

function sshAuthenticationLabel(method: AuthMethod) {
  switch (method) {
    case "public-key": return t("public-key");
    case "keyboard-interactive": return t("keyboard-interactive");
    case "password": return t("password");
    case "gssapi-with-mic": return "GSSAPI";
    case "none": return t("no-authentication");
  }
}

function identitySourceLabel(source: IdentityRef["source"]): string {
  switch (source) {
    case "profile-vault": return "Stronghold";
    case "system-file": return t("local-file");
    case "agent": return "ssh-agent";
    case "public-key-only": return t("public-key-only");
  }
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
