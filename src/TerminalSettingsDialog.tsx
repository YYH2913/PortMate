import { useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";
import { Ban, FolderOpen, RotateCcw, Search, X } from "lucide-react";
import { isBackendAvailable } from "./api";
import { KeyedRequestGate } from "./keyed-request-gate";
import {
  MAX_SCREEN_LOCK_TIMEOUT_MINUTES,
  MIN_SCREEN_LOCK_TIMEOUT_MINUTES,
  normalizeScreenLockTimeoutMinutes,
} from "./screen-lock-state";
import { allSyncProtocols, normalizeSyncInputSettings } from "./sync-input-state";
import type { SyncInputSettings, SyncNewlineMode } from "./sync-input-state";
import { terminalStartupSessionOptions } from "./terminal-settings-state";
import { terminalSettingsDraftHasUnsavedChanges } from "./terminal-settings-draft-state";
import {
  chooseTerminalExportDirectory,
  MAX_TERMINAL_EXPORT_DIRECTORY_CHARACTERS,
} from "./terminal-export-path";
import type { SessionKind, SessionSummary } from "./types";
import {
  defaultWorkspaceKeymap,
  formatWorkspaceKeyBinding,
  normalizeWorkspaceKeymap,
  WORKSPACE_KEY_CHORD_TIMEOUT_MS,
  WORKSPACE_KEYMAP_STORAGE_KEY,
  workspaceHotkeyCommands,
  workspaceKeyBindingFromEvent,
  workspaceKeymapConflicts,
} from "./workspace-hotkeys";
import type { WorkspaceHotkeyCommandId, WorkspaceKeymap } from "./workspace-hotkeys";
import LanguageSelector from "./LanguageSelector";
import { t, useLocale, localizeDiagnostic } from "./i18n";

const MAX_COMMAND_HISTORY_LIMIT = 10_000;
const MAX_COMMAND_HISTORY_RETENTION_DAYS = 3_650;

const terminalSettingPages = [
  "application",
  "keyboard-shortcuts",
  "mouse",
  "autocomplete",
  "command-history-settings",
  "synchronized-input",
  "security",
] as const;

const sessionKindLabels: Record<SessionKind, string> = {
  ssh: "SSH",
  tmux: "Tmux",
  serial: "Serial",
  shell: "Shell",
  telnet: "Telnet",
  tcp: "Raw TCP",
};

type TerminalPrefs = ReturnType<typeof createTerminalPrefs>;

export default function TerminalSettingsDialog({
  initialPrefs,
  normalizePrefs,
  sessions,
  syncSettings,
  workspaceKeymap,
  onPrefsChange,
  onClearCommandHistory,
  onSyncSettingsChange,
  onWorkspaceKeymapChange,
  onClose,
}: {
  initialPrefs: TerminalPrefs;
  normalizePrefs: (value: unknown) => TerminalPrefs;
  sessions: readonly SessionSummary[];
  syncSettings: SyncInputSettings;
  workspaceKeymap: WorkspaceKeymap;
  onPrefsChange: (prefs: TerminalPrefs) => void;
  onClearCommandHistory: () => void;
  onSyncSettingsChange: (settings: SyncInputSettings) => void;
  onWorkspaceKeymapChange: (keymap: WorkspaceKeymap) => void;
  onClose: () => void;
}) {
  const { locale } = useLocale();
  const [activeItem, setActiveItem] = useState("application");
  const [pageQuery, setPageQuery] = useState("");
  const [prefs, setPrefs] = useState<TerminalPrefs>(initialPrefs);
  const [syncDraft, setSyncDraft] = useState(syncSettings);
  const [workspaceKeymapDraft, setWorkspaceKeymapDraft] = useState(workspaceKeymap);
  const [settingsError, setSettingsError] = useState("");
  const [directoryBusy, setDirectoryBusy] = useState(false);
  const directoryRequestGate = useRef(new KeyedRequestGate<"directory">());
  const updatePref = <K extends keyof TerminalPrefs>(key: K, value: TerminalPrefs[K]) => setPrefs((current) => ({ ...current, [key]: value }));
  const keymapConflictCount = workspaceKeymapConflicts(workspaceKeymapDraft).length;
  const visiblePages = useMemo(() => {
    const needle = pageQuery.trim().toLocaleLowerCase();
    if (!needle) return [...terminalSettingPages];
    return terminalSettingPages.filter((page) => t(page).toLocaleLowerCase().includes(needle));
  }, [locale, pageQuery]);
  const dirty = terminalSettingsDraftHasUnsavedChanges(
    { prefs, syncSettings: syncDraft, workspaceKeymap: workspaceKeymapDraft },
    { prefs: initialPrefs, syncSettings, workspaceKeymap },
  );

  useEffect(() => {
    if (visiblePages.length && !visiblePages.includes(activeItem as typeof terminalSettingPages[number])) {
      setActiveItem(visiblePages[0]);
    }
  }, [activeItem, visiblePages]);

  useEffect(() => () => {
    directoryRequestGate.current.invalidateAll();
  }, []);

  async function selectTerminalExportDirectory() {
    const gate = directoryRequestGate.current;
    const token = gate.begin("directory");
    if (token === null) return;
    setDirectoryBusy(true);
    setSettingsError("");
    try {
      const selected = await chooseTerminalExportDirectory(prefs.terminalTextExportDirectory);
      if (!gate.isCurrent("directory", token)) return;
      if (selected !== null) updatePref("terminalTextExportDirectory", selected);
    } catch (error) {
      if (gate.isCurrent("directory", token)) setSettingsError(error instanceof Error ? error.message : String(error));
    } finally {
      if (gate.finish("directory", token)) setDirectoryBusy(false);
    }
  }

  function savePrefs() {
    if (keymapConflictCount) return;
    const normalizedKeymap = normalizeWorkspaceKeymap(workspaceKeymapDraft);
    const normalizedPrefs = normalizePrefs(prefs);
    saveLocalValue("portmate.terminalPrefs", normalizedPrefs);
    saveLocalValue(WORKSPACE_KEYMAP_STORAGE_KEY, normalizedKeymap);
    onPrefsChange(normalizedPrefs);
    onSyncSettingsChange(normalizeSyncInputSettings(syncDraft));
    onWorkspaceKeymapChange(normalizedKeymap);
    onClose();
  }

  function closeDialog() {
    if (dirty && !window.confirm(t("terminal-settings-have-unsaved-changes-closing-will-discard-them"))) return;
    onClose();
  }

  return (
    <DialogFrame title={t("terminal-settings")} className="terminal-settings-dialog" onClose={closeDialog}>
      <nav className="settings-nav">
        <label className="settings-search">
          <Search size={14} aria-hidden="true" />
          <input
            type="search"
            value={pageQuery}
            placeholder={t("search-settings")}
            aria-label={t("search-settings")}
            onChange={(event) => setPageQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape" && pageQuery) {
                event.preventDefault();
                setPageQuery("");
              }
            }}
          />
        </label>
        <div className="settings-tabs" role="tablist" aria-label={t("terminal-settings-pages")}>
          {visiblePages.length ? visiblePages.map((page) => (
            <button key={page} type="button" role="tab" aria-selected={activeItem === page} className={activeItem === page ? "active" : ""} onClick={() => setActiveItem(page)}>
              {t(page)}
            </button>
          )) : (
            <p className="settings-search-empty">{t("no-matching-settings")}</p>
          )}
        </div>
      </nav>
      <section className="settings-content" role="tabpanel">
        <header className="session-settings-pane-title">
          <h2>{t(activeItem)}</h2>
        </header>
        <TerminalSettingsContent
          activeItem={activeItem}
          prefs={prefs}
          sessions={sessions}
          workspaceKeymap={workspaceKeymapDraft}
          updatePref={updatePref}
          onClearCommandHistory={onClearCommandHistory}
          onWorkspaceKeymapChange={setWorkspaceKeymapDraft}
          syncSettings={syncDraft}
          onSyncSettingsChange={setSyncDraft}
          canChooseExportDirectory={isBackendAvailable() && !directoryBusy}
          exportDirectoryBusy={directoryBusy}
          onChooseExportDirectory={() => void selectTerminalExportDirectory()}
          onExportDirectoryChange={(value) => {
            setSettingsError("");
            updatePref("terminalTextExportDirectory", value);
          }}
        />
      </section>
      <div className="dialog-footer">
        <div className={keymapConflictCount || settingsError ? "dialog-note error" : "dialog-note"}>
          {keymapConflictCount ? t("keyboard-shortcut-conflicts", [keymapConflictCount]) : localizeDiagnostic(settingsError)}
        </div>
        <div className="dialog-actions inline">
          <button onClick={savePrefs} disabled={keymapConflictCount > 0}>{t("save")}</button>
          <button onClick={closeDialog}>{t("cancel")}</button>
        </div>
      </div>
    </DialogFrame>
  );
}

function TerminalSettingsContent({
  activeItem,
  prefs,
  sessions,
  workspaceKeymap,
  updatePref,
  onClearCommandHistory,
  onWorkspaceKeymapChange,
  syncSettings,
  onSyncSettingsChange,
  canChooseExportDirectory,
  exportDirectoryBusy,
  onChooseExportDirectory,
  onExportDirectoryChange,
}: {
  activeItem: string;
  prefs: TerminalPrefs;
  sessions: readonly SessionSummary[];
  workspaceKeymap: WorkspaceKeymap;
  updatePref: <K extends keyof TerminalPrefs>(key: K, value: TerminalPrefs[K]) => void;
  onClearCommandHistory: () => void;
  onWorkspaceKeymapChange: (keymap: WorkspaceKeymap) => void;
  syncSettings: SyncInputSettings;
  onSyncSettingsChange: (settings: SyncInputSettings) => void;
  canChooseExportDirectory: boolean;
  exportDirectoryBusy: boolean;
  onChooseExportDirectory: () => void;
  onExportDirectoryChange: (value: string) => void;
}) {
  useLocale();
  switch (activeItem) {
    case "application":
      return (
        <>
          <SettingsSection title={t("interface-language")}>
            <LanguageSelector />
            <p className="dialog-note">{t("applies-immediately-to-all-windows-unsupported-languages-use-english")}</p>
          </SettingsSection>
          <SettingsSection title={t("start")}>
            <SettingRadio label={t("no-sessions-n")} checked={prefs.startupMode === "none"} onChange={() => updatePref("startupMode", "none")} name="startup-mode" />
            <SettingRadio label={t("previous-sessions-l")} checked={prefs.startupMode === "last"} onChange={() => updatePref("startupMode", "last")} name="startup-mode" />
            <SettingRadio label={t("specific-session-or-session-group-s")} checked={prefs.startupMode === "specific"} onChange={() => updatePref("startupMode", "specific")} name="startup-mode" />
            {[0, 1, 2, 3].map((index) => (
              <SettingSelect
                key={index}
                label={t("session-2", [index + 1])}
                value={prefs.startupSessions[index] ?? ""}
                options={terminalStartupSessionOptions(sessions, prefs.startupSessions[index])}
                disabled={prefs.startupMode !== "specific"}
                onChange={(value) => {
                  const next = [...prefs.startupSessions];
                  next[index] = value;
                  updatePref("startupSessions", next);
                }}
              />
            ))}
          </SettingsSection>
          <SettingsSection title={t("terminal-text-export")}>
            <SettingPath
              label={t("default-directory")}
              value={prefs.terminalTextExportDirectory}
              placeholder={t("portmate-default-exports-directory")}
              canBrowse={canChooseExportDirectory}
              disabled={exportDirectoryBusy}
              onBrowse={onChooseExportDirectory}
              onChange={onExportDirectoryChange}
            />
          </SettingsSection>
        </>
      );
    case "security":
      return (
        <SettingsSection title={t("security")}>
          <SettingCheck label={t("lock-screen-after-idle-time")} checked={prefs.lockOnIdle} onChange={(value) => updatePref("lockOnIdle", value)} />
          <SettingInput
            label={t("lock-timeout-minutes")}
            type="number"
            value={prefs.lockScreenTimeoutMinutes}
            min={MIN_SCREEN_LOCK_TIMEOUT_MINUTES}
            max={MAX_SCREEN_LOCK_TIMEOUT_MINUTES}
            step={1}
            onChange={(value) => updatePref("lockScreenTimeoutMinutes", normalizeScreenLockTimeoutMinutes(value))}
          />
          <SettingCheck label={t("lock-screen-on-startup")} checked={prefs.requireMasterPassword} onChange={(value) => updatePref("requireMasterPassword", value)} />
        </SettingsSection>
      );
    case "keyboard-shortcuts":
      return <WorkspaceKeymapSettings keymap={workspaceKeymap} onChange={onWorkspaceKeymapChange} />;
    case "synchronized-input":
      return (
        <>
          <SettingsSection title={t("target-protocols")}>
            {allSyncProtocols.map((protocol) => (
              <SettingCheck
                key={protocol}
                label={sessionKindLabels[protocol]}
                checked={syncSettings.protocols.includes(protocol)}
                onChange={(checked) => onSyncSettingsChange({
                  ...syncSettings,
                  protocols: checked
                    ? [...syncSettings.protocols, protocol]
                    : syncSettings.protocols.filter((item) => item !== protocol),
                })}
              />
            ))}
          </SettingsSection>
          <SettingsSection title={t("input-transformation")}>
            <label className="setting-row">
              <span>{t("newline-policy")}</span>
              <select value={syncSettings.newlineMode} onChange={(event) => onSyncSettingsChange({ ...syncSettings, newlineMode: event.target.value as SyncNewlineMode })}>
                <option value="protocol">{t("protocol-default")}</option>
                <option value="preserve">{t("preserve")}</option>
                <option value="lf">LF</option>
                <option value="crlf">CRLF</option>
              </select>
            </label>
            <SettingInput label={t("delay-between-targets-ms")} type="number" value={syncSettings.delayMs} onChange={(value) => onSyncSettingsChange({ ...syncSettings, delayMs: Math.min(5000, Math.max(0, Math.trunc(Number(value) || 0))) })} />
            <SettingInput label={t("batch-prefix")} value={syncSettings.prefix} onChange={(value) => onSyncSettingsChange({ ...syncSettings, prefix: value.slice(0, 1024) })} />
            <SettingInput label={t("batch-suffix")} value={syncSettings.suffix} onChange={(value) => onSyncSettingsChange({ ...syncSettings, suffix: value.slice(0, 1024) })} />
          </SettingsSection>
        </>
      );
    case "autocomplete":
      return (
        <>
          <SettingsSection title={t("completion")}>
            <SettingCheck label={t("enable-autocomplete-a")} checked={prefs.completionEnabled} onChange={(value) => updatePref("completionEnabled", value)} />
            <SettingCheck label={t("onekey-terminal-prompt-completion-k")} checked={prefs.oneKeyCompletionEnabled} onChange={(value) => updatePref("oneKeyCompletionEnabled", value)} />
            <div className="settings-subtitle">{t("autocomplete-sources")}</div>
            <SettingCheck label={t("command-names-n")} checked={prefs.completionCommandNames} onChange={(value) => updatePref("completionCommandNames", value)} />
            <SettingCheck label={t("command-options-o")} checked={prefs.completionCommandOptions} onChange={(value) => updatePref("completionCommandOptions", value)} />
            <SettingCheck label={t("subcommands-and-arguments-p")} checked={prefs.completionCommandArgs} onChange={(value) => updatePref("completionCommandArgs", value)} />
            <SettingCheck label={t("command-history-h")} checked={prefs.completionHistory} onChange={(value) => updatePref("completionHistory", value)} />
            <SettingCheck label={t("quick-commands-q")} checked={prefs.completionQuickCommands} onChange={(value) => updatePref("completionQuickCommands", value)} />
            <SettingSelect label={t("start-autocomplete-after-s")} value={String(prefs.completionTriggerChars)} options={[1, 2, 3].map(value => ({ value: String(value), label: t(value === 1 ? "1-character" : `${value}-characters`) }))} onChange={(value) => updatePref("completionTriggerChars", Number(value))} />
          </SettingsSection>
          <SettingsSection title={t("appearance")}>
            <SettingCheck label={t("multicolor-commands-and-output")} checked={prefs.semanticHighlightingEnabled} onChange={(value) => updatePref("semanticHighlightingEnabled", value)} />
            <SettingSelect label={t("completion-list-height-h")} value={String(prefs.completionListHeight)} options={[5, 7, 10].map(value => ({ value: String(value), label: t(`${value}-rows`) }))} onChange={(value) => updatePref("completionListHeight", Number(value))} />
            <SettingSelect label={t("preview-best-match-p")} value={prefs.completionPreviewMode} options={[{ value: "none", label: t("no-preview") }, { value: "input", label: t("input-field") }, { value: "top", label: t("top-of-list") }]} onChange={(value) => updatePref("completionPreviewMode", value)} />
          </SettingsSection>
        </>
      );
    case "command-history-settings":
      return (
        <>
          <SettingsSection title={t("capacity")}>
            <SettingInput label={t("history-retention-days-d")} type="number" min={0} max={MAX_COMMAND_HISTORY_RETENTION_DAYS} step={1} value={prefs.historyRetentionDays} onChange={(value) => updatePref("historyRetentionDays", value)} />
            <SettingInput label={t("history-size-h")} type="number" min={1} max={MAX_COMMAND_HISTORY_LIMIT} step={1} value={prefs.historyLimit} onChange={(value) => updatePref("historyLimit", value)} />
          </SettingsSection>
          <SettingsSection title={t("storage")}>
            <SettingCheck label={t("save-command-history-to-disk-s")} checked={prefs.historyEnabled} onChange={(value) => updatePref("historyEnabled", value)} />
            <SettingButtonRow label={t("saved-command-history")}>
              <button className="settings-secondary-button" type="button" onClick={onClearCommandHistory}>{t("clear-c")}</button>
            </SettingButtonRow>
          </SettingsSection>
        </>
      );
    case "mouse":
      return (
        <SettingsSection title={t("mouse")}>
          <SettingCheck label={t("allow-terminal-applications-to-receive-mouse-events")} checked={prefs.mouseReporting} onChange={(value) => updatePref("mouseReporting", value)} />
          <SettingCheck label={t("copy-on-selection")} checked={prefs.mouseCopyOnSelect} onChange={(value) => updatePref("mouseCopyOnSelect", value)} />
        </SettingsSection>
      );
    default:
      return null;
  }
}

function WorkspaceKeymapSettings({
  keymap,
  onChange,
}: {
  keymap: WorkspaceKeymap;
  onChange: (keymap: WorkspaceKeymap) => void;
}) {
  useLocale();
  const [capturing, setCapturing] = useState<WorkspaceHotkeyCommandId | null>(null);
  const [capturePrefix, setCapturePrefix] = useState<{ commandId: WorkspaceHotkeyCommandId; binding: string } | null>(null);
  const [captureError, setCaptureError] = useState<WorkspaceHotkeyCommandId | null>(null);
  const captureTimerRef = useRef<number | null>(null);
  const conflicts = workspaceKeymapConflicts(keymap);
  const labels = Object.fromEntries(workspaceHotkeyCommands.map((command) => [command.id, command.label])) as Record<WorkspaceHotkeyCommandId, string>;

  useEffect(() => () => {
    if (captureTimerRef.current !== null) window.clearTimeout(captureTimerRef.current);
  }, []);

  function updateBinding(commandId: WorkspaceHotkeyCommandId, binding: string) {
    onChange({ ...keymap, [commandId]: binding });
  }

  function stopCapture(commandId?: WorkspaceHotkeyCommandId) {
    if (captureTimerRef.current !== null) window.clearTimeout(captureTimerRef.current);
    captureTimerRef.current = null;
    setCapturing((current) => !commandId || current === commandId ? null : current);
    setCapturePrefix((current) => !commandId || current?.commandId === commandId ? null : current);
    setCaptureError((current) => !commandId || current === commandId ? null : current);
  }

  function beginCapture(commandId: WorkspaceHotkeyCommandId) {
    stopCapture();
    setCapturing(commandId);
  }

  function captureBinding(event: ReactKeyboardEvent<HTMLButtonElement>, commandId: WorkspaceHotkeyCommandId) {
    if (capturing !== commandId) return;
    event.preventDefault();
    event.stopPropagation();
    if (isPlainEscape(event.nativeEvent)) {
      stopCapture(commandId);
      return;
    }
    if ((event.code === "Backspace" || event.code === "Delete") && !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
      updateBinding(commandId, "");
      stopCapture(commandId);
      return;
    }
    if (event.repeat || isModifierKeyEvent(event.nativeEvent)) return;
    const binding = workspaceKeyBindingFromEvent(event);
    if (!binding) {
      setCaptureError(commandId);
      return;
    }
    if (capturePrefix?.commandId === commandId) {
      updateBinding(commandId, `${capturePrefix.binding} ${binding}`);
      stopCapture(commandId);
      return;
    }
    updateBinding(commandId, binding);
    setCapturePrefix({ commandId, binding });
    setCaptureError(null);
    captureTimerRef.current = window.setTimeout(() => stopCapture(commandId), WORKSPACE_KEY_CHORD_TIMEOUT_MS);
  }

  return (
    <SettingsSection title={t("keyboard-shortcuts")}>
      <div className="workspace-keymap">
        <header className="workspace-keymap-header">
          <span>{t("command")}</span>
          <span>{t("keys")}</span>
          <button
            type="button"
            title={t("restore-all-default-shortcuts")}
            aria-label={t("restore-all-default-shortcuts")}
            onClick={() => {
              onChange({ ...defaultWorkspaceKeymap });
              stopCapture();
            }}
          >
            <RotateCcw size={14} />
          </button>
        </header>
        {workspaceHotkeyCommands.map((command) => {
          const conflict = conflicts.find((item) => item.commandIds.includes(command.id));
          const conflictLabels = conflict?.commandIds.filter((id) => id !== command.id).map((id) => labels[id]).join("、");
          const invalid = captureError === command.id;
          const pendingBinding = capturePrefix?.commandId === command.id ? capturePrefix.binding : "";
          const formattedBinding = formatWorkspaceKeyBinding(keymap[command.id]);
          return (
            <div key={command.id} className={`workspace-keymap-row ${conflict ? "conflict" : ""}`}>
              <span className="workspace-keymap-command">
                <strong>{t(command.label)}</strong>
                {conflictLabels ? <small>{t("with")}{conflictLabels}{conflict?.kind === "prefix" ? t("prefix-conflict") : t("conflict")}</small> : invalid ? <small>{t("each-chord-requires-a-modifier-key")}</small> : null}
              </span>
              <button
                type="button"
                className={capturing === command.id ? "workspace-key-capture capturing" : "workspace-key-capture"}
                aria-pressed={capturing === command.id}
                title={capturing === command.id ? t("record-shortcut") : formattedBinding}
                onClick={() => beginCapture(command.id)}
                onBlur={() => stopCapture(command.id)}
                onKeyDown={(event) => captureBinding(event, command.id)}
              >
                {pendingBinding ? `${formatWorkspaceKeyBinding(pendingBinding)}  →  …` : capturing === command.id ? t("waiting-for-the-first-key") : formattedBinding}
              </button>
              <button
                type="button"
                className="workspace-key-disable"
                title={t("disable-shortcut-for", [command.label])}
                aria-label={t("disable-shortcut-for", [command.label])}
                disabled={!keymap[command.id]}
                onClick={() => {
                  updateBinding(command.id, "");
                  stopCapture(command.id);
                }}
              >
                <Ban size={13} />
              </button>
              <button
                type="button"
                className="workspace-key-reset"
                title={t("restore-default-shortcut-for", [command.label])}
                aria-label={t("restore-default-shortcut-for", [command.label])}
                disabled={keymap[command.id] === command.defaultBinding}
                onClick={() => {
                  updateBinding(command.id, command.defaultBinding);
                  stopCapture(command.id);
                }}
              >
                <RotateCcw size={13} />
              </button>
            </div>
          );
        })}
      </div>
    </SettingsSection>
  );
}

function DialogFrame({
  title,
  className,
  onClose,
  children,
}: {
  title: string;
  className: string;
  onClose: () => void;
  children: ReactNode;
}) {
  useLocale();
  return (
    <div className="dialog-backdrop">
      <section className={`wind-dialog ${className}`}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{title}</strong>
          <button type="button" title={t("close-2", [title])} aria-label={t("close-2", [title])} onClick={onClose}><X size={22} /></button>
        </header>
        {children}
      </section>
    </div>
  );
}

function SettingsSection({ title, children }: { title?: string; children: ReactNode }) {
  useLocale();
  return (
    <section className="settings-section">
      {title ? <h2>{title}</h2> : null}
      <div className="settings-box">{children}</div>
    </section>
  );
}

function SettingRadio({ label, checked, name, onChange }: { label: string; checked: boolean; name: string; onChange: () => void }) {
  useLocale();
  return (
    <label className="setting-radio">
      <input type="radio" name={name} checked={checked} onChange={onChange} />
      <span>{label}</span>
    </label>
  );
}

function SettingCheck({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  useLocale();
  return (
    <label className="setting-check">
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      <span>{label}</span>
    </label>
  );
}

function SettingInput({ label, value, type = "text", min, max, step, onChange }: { label: string; value: string | number; type?: string; min?: number; max?: number; step?: number; onChange: (value: string) => void }) {
  useLocale();
  return (
    <label className="setting-row">
      <span>{label}</span>
      <input type={type} value={value} min={min} max={max} step={step} onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}

function SettingPath({
  label,
  value,
  placeholder,
  canBrowse,
  disabled,
  onBrowse,
  onChange,
}: {
  label: string;
  value: string;
  placeholder: string;
  canBrowse: boolean;
  disabled: boolean;
  onBrowse: () => void;
  onChange: (value: string) => void;
}) {
  useLocale();
  return (
    <label className="setting-row terminal-export-path-setting">
      <span>{label}</span>
      <span className="setting-path-control">
        <input
          aria-label={t("default-terminal-text-export-directory")}
          value={value}
          disabled={disabled}
          maxLength={MAX_TERMINAL_EXPORT_DIRECTORY_CHARACTERS}
          placeholder={placeholder}
          onChange={(event) => onChange(event.target.value)}
        />
        <button
          type="button"
          aria-label={t("choose-terminal-text-export-directory")}
          title={disabled ? t("choosing-directory") : canBrowse ? t("choose-directory") : t("directory-selection-is-available-only-in-the-desktop-app")}
          disabled={disabled || !canBrowse}
          onClick={onBrowse}
        >
          <FolderOpen size={16} />
        </button>
      </span>
    </label>
  );
}

type SettingSelectOption = string | { value: string; label: string };

function SettingSelect({ label, value, options, disabled = false, onChange }: { label: string; value: string; options: readonly SettingSelectOption[]; disabled?: boolean; onChange: (value: string) => void }) {
  useLocale();
  return (
    <label className="setting-row">
      <span>{label}</span>
      <select aria-label={label} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)}>
        {options.map((option) => {
          const optionValue = typeof option === "string" ? option : option.value;
          const optionLabel = typeof option === "string" ? option || t("unspecified") : option.label;
          return <option key={optionValue || "blank"} value={optionValue}>
            {typeof option === "string" ? t(optionLabel) : optionLabel}
          </option>;
        })}
      </select>
    </label>
  );
}

function SettingButtonRow({ label, children }: { label: string; children: ReactNode }) {
  useLocale();
  return (
    <div className="setting-row">
      <span>{label}</span>
      <div className="setting-row-actions">{children}</div>
    </div>
  );
}

function createTerminalPrefs() {
  return {
    startupMode: "last",
    startupSessions: ["", "", "", ""],
    terminalTextExportDirectory: "",
    lockOnIdle: false,
    lockScreenTimeoutMinutes: 30,
    requireMasterPassword: false,
    completionEnabled: true,
    oneKeyCompletionEnabled: true,
    semanticHighlightingEnabled: true,
    completionCommandNames: true,
    completionCommandOptions: true,
    completionCommandArgs: true,
    completionHistory: true,
    completionQuickCommands: true,
    completionTriggerChars: 1,
    completionListHeight: 7,
    completionPreviewMode: "none",
    historyEnabled: true,
    historyRetentionDays: "30",
    historyLimit: "10000",
    mouseReporting: true,
    mouseCopyOnSelect: true,
  };
}

function isModifierKeyEvent(event: KeyboardEvent) {
  return ["Alt", "Control", "Meta", "Shift"].includes(event.key);
}

function isPlainEscape(event: KeyboardEvent) {
  return event.code === "Escape" && !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
}

function saveLocalValue<T>(key: string, value: T) {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Settings remain active for the current process when persistence is unavailable.
  }
}
