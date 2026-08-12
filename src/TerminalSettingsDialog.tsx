import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";
import { Ban, FolderOpen, RotateCcw, X } from "lucide-react";
import { isBackendAvailable } from "./api";
import {
  MAX_SCREEN_LOCK_TIMEOUT_MINUTES,
  MIN_SCREEN_LOCK_TIMEOUT_MINUTES,
  normalizeScreenLockTimeoutMinutes,
} from "./screen-lock-state";
import { allSyncProtocols, normalizeSyncInputSettings } from "./sync-input-state";
import type { SyncInputSettings, SyncNewlineMode } from "./sync-input-state";
import { terminalStartupSessionOptions } from "./terminal-settings-state";
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

const MAX_COMMAND_HISTORY_LIMIT = 10_000;
const MAX_COMMAND_HISTORY_RETENTION_DAYS = 3_650;

const terminalSettingPages = [
  "应用",
  "安全",
  "快捷键",
  "自动补全",
  "命令历史",
  "鼠标",
  "同步输入",
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
  const [activeItem, setActiveItem] = useState("应用");
  const [prefs, setPrefs] = useState<TerminalPrefs>(initialPrefs);
  const [syncDraft, setSyncDraft] = useState(syncSettings);
  const [workspaceKeymapDraft, setWorkspaceKeymapDraft] = useState(workspaceKeymap);
  const [settingsError, setSettingsError] = useState("");
  const updatePref = <K extends keyof TerminalPrefs>(key: K, value: TerminalPrefs[K]) => setPrefs((current) => ({ ...current, [key]: value }));
  const keymapConflictCount = workspaceKeymapConflicts(workspaceKeymapDraft).length;

  async function selectTerminalExportDirectory() {
    setSettingsError("");
    try {
      const selected = await chooseTerminalExportDirectory(prefs.terminalTextExportDirectory);
      if (selected !== null) updatePref("terminalTextExportDirectory", selected);
    } catch (error) {
      setSettingsError(error instanceof Error ? error.message : String(error));
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

  return (
    <DialogFrame title="终端设置" className="terminal-settings-dialog" onClose={onClose}>
      <nav className="settings-tabs" role="tablist" aria-label="终端设置页面">
        {terminalSettingPages.map((page) => (
          <button key={page} type="button" role="tab" aria-selected={activeItem === page} className={activeItem === page ? "active" : ""} onClick={() => setActiveItem(page)}>
              {page}
            </button>
        ))}
      </nav>
      <section className="settings-content" role="tabpanel">
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
          canChooseExportDirectory={isBackendAvailable()}
          onChooseExportDirectory={() => void selectTerminalExportDirectory()}
          onExportDirectoryChange={(value) => {
            setSettingsError("");
            updatePref("terminalTextExportDirectory", value);
          }}
        />
      </section>
      <div className="dialog-footer">
        <div className={keymapConflictCount || settingsError ? "dialog-note error" : "dialog-note"}>
          {keymapConflictCount ? `${keymapConflictCount} 组快捷键冲突` : settingsError}
        </div>
        <div className="dialog-actions inline">
          <button onClick={savePrefs} disabled={keymapConflictCount > 0}>保存</button>
          <button onClick={onClose}>取消</button>
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
  onChooseExportDirectory: () => void;
  onExportDirectoryChange: (value: string) => void;
}) {
  switch (activeItem) {
    case "应用":
      return (
        <>
          <SettingsSection title="启动">
            <SettingRadio label="无会话(N)" checked={prefs.startupMode === "none"} onChange={() => updatePref("startupMode", "none")} name="startup-mode" />
            <SettingRadio label="上次会话(L)" checked={prefs.startupMode === "last"} onChange={() => updatePref("startupMode", "last")} name="startup-mode" />
            <SettingRadio label="指定一个会话或一组会话(S)" checked={prefs.startupMode === "specific"} onChange={() => updatePref("startupMode", "specific")} name="startup-mode" />
            {[0, 1, 2, 3].map((index) => (
              <SettingSelect
                key={index}
                label={`会话 ${index + 1}:`}
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
          <SettingsSection title="终端文本导出">
            <SettingPath
              label="默认目录:"
              value={prefs.terminalTextExportDirectory}
              placeholder="PortMate 默认 exports 目录"
              canBrowse={canChooseExportDirectory}
              onBrowse={onChooseExportDirectory}
              onChange={onExportDirectoryChange}
            />
          </SettingsSection>
        </>
      );
    case "安全":
      return (
        <SettingsSection title="安全">
          <SettingCheck label="空闲后锁屏" checked={prefs.lockOnIdle} onChange={(value) => updatePref("lockOnIdle", value)} />
          <SettingInput
            label="锁屏超时（分钟）"
            type="number"
            value={prefs.lockScreenTimeoutMinutes}
            min={MIN_SCREEN_LOCK_TIMEOUT_MINUTES}
            max={MAX_SCREEN_LOCK_TIMEOUT_MINUTES}
            step={1}
            onChange={(value) => updatePref("lockScreenTimeoutMinutes", normalizeScreenLockTimeoutMinutes(value))}
          />
          <SettingCheck label="启动时锁屏" checked={prefs.requireMasterPassword} onChange={(value) => updatePref("requireMasterPassword", value)} />
        </SettingsSection>
      );
    case "快捷键":
      return <WorkspaceKeymapSettings keymap={workspaceKeymap} onChange={onWorkspaceKeymapChange} />;
    case "同步输入":
      return (
        <>
          <SettingsSection title="目标协议">
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
          <SettingsSection title="输入变换">
            <label className="setting-row">
              <span>换行策略:</span>
              <select value={syncSettings.newlineMode} onChange={(event) => onSyncSettingsChange({ ...syncSettings, newlineMode: event.target.value as SyncNewlineMode })}>
                <option value="protocol">按协议</option>
                <option value="preserve">保持原样</option>
                <option value="lf">LF</option>
                <option value="crlf">CRLF</option>
              </select>
            </label>
            <SettingInput label="目标间延迟(ms):" type="number" value={syncSettings.delayMs} onChange={(value) => onSyncSettingsChange({ ...syncSettings, delayMs: Math.min(5000, Math.max(0, Math.trunc(Number(value) || 0))) })} />
            <SettingInput label="批量发送前缀:" value={syncSettings.prefix} onChange={(value) => onSyncSettingsChange({ ...syncSettings, prefix: value.slice(0, 1024) })} />
            <SettingInput label="批量发送后缀:" value={syncSettings.suffix} onChange={(value) => onSyncSettingsChange({ ...syncSettings, suffix: value.slice(0, 1024) })} />
          </SettingsSection>
        </>
      );
    case "自动补全":
      return (
        <>
          <SettingsSection title="完成">
            <SettingCheck label="启用自动补全(A)" checked={prefs.completionEnabled} onChange={(value) => updatePref("completionEnabled", value)} />
            <SettingCheck label="OneKey 终端提示补全(K)" checked={prefs.oneKeyCompletionEnabled} onChange={(value) => updatePref("oneKeyCompletionEnabled", value)} />
            <div className="settings-subtitle">自动完成命令使用：</div>
            <SettingCheck label="命令名称(N)" checked={prefs.completionCommandNames} onChange={(value) => updatePref("completionCommandNames", value)} />
            <SettingCheck label="命令选项(O)" checked={prefs.completionCommandOptions} onChange={(value) => updatePref("completionCommandOptions", value)} />
            <SettingCheck label="子命令与参数(P)" checked={prefs.completionCommandArgs} onChange={(value) => updatePref("completionCommandArgs", value)} />
            <SettingCheck label="历史命令(H)" checked={prefs.completionHistory} onChange={(value) => updatePref("completionHistory", value)} />
            <SettingCheck label="快速命令(Q)" checked={prefs.completionQuickCommands} onChange={(value) => updatePref("completionQuickCommands", value)} />
            <SettingSelect label="输入后开始自动补全:(S)" value={prefs.completionTriggerChars} options={["1 字符", "2 字符", "3 字符"]} onChange={(value) => updatePref("completionTriggerChars", value)} />
          </SettingsSection>
          <SettingsSection title="外观">
            <SettingCheck label="自动多色交互命令行" checked={prefs.semanticHighlightingEnabled} onChange={(value) => updatePref("semanticHighlightingEnabled", value)} />
            <SettingSelect label="完成列表高度:(H)" value={prefs.completionListHeight} options={["5 行", "7 行", "10 行"]} onChange={(value) => updatePref("completionListHeight", value)} />
            <SettingSelect label="预览最佳匹配项:(P)" value={prefs.completionPreviewMode} options={["无处", "输入框", "列表顶部"]} onChange={(value) => updatePref("completionPreviewMode", value)} />
          </SettingsSection>
        </>
      );
    case "命令历史":
      return (
        <>
          <SettingsSection title="容量">
            <SettingInput label="保留历史天数:(D)" type="number" min={0} max={MAX_COMMAND_HISTORY_RETENTION_DAYS} step={1} value={prefs.historyRetentionDays} onChange={(value) => updatePref("historyRetentionDays", value)} />
            <SettingInput label="历史大小:(H)" type="number" min={1} max={MAX_COMMAND_HISTORY_LIMIT} step={1} value={prefs.historyLimit} onChange={(value) => updatePref("historyLimit", value)} />
          </SettingsSection>
          <SettingsSection title="存储">
            <SettingCheck label="将命令历史保存到磁盘(S)" checked={prefs.historyEnabled} onChange={(value) => updatePref("historyEnabled", value)} />
            <SettingButtonRow label="已保存的命令历史:">
              <button className="settings-secondary-button" type="button" onClick={onClearCommandHistory}>
                清除(C)
              </button>
            </SettingButtonRow>
          </SettingsSection>
        </>
      );
    case "鼠标":
      return (
        <SettingsSection title="鼠标">
          <SettingCheck label="允许终端应用接收鼠标事件" checked={prefs.mouseReporting} onChange={(value) => updatePref("mouseReporting", value)} />
          <SettingCheck label="选择即复制" checked={prefs.mouseCopyOnSelect} onChange={(value) => updatePref("mouseCopyOnSelect", value)} />
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
    <SettingsSection title="快捷键">
      <div className="workspace-keymap">
        <header className="workspace-keymap-header">
          <span>命令</span>
          <span>按键</span>
          <button
            type="button"
            title="恢复全部默认快捷键"
            aria-label="恢复全部默认快捷键"
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
                <strong>{command.label}</strong>
                {conflictLabels ? <small>与 {conflictLabels}{conflict?.kind === "prefix" ? " 前缀冲突" : " 冲突"}</small> : invalid ? <small>每段需要修饰键</small> : null}
              </span>
              <button
                type="button"
                className={capturing === command.id ? "workspace-key-capture capturing" : "workspace-key-capture"}
                aria-pressed={capturing === command.id}
                title={capturing === command.id ? "录入快捷键" : formattedBinding}
                onClick={() => beginCapture(command.id)}
                onBlur={() => stopCapture(command.id)}
                onKeyDown={(event) => captureBinding(event, command.id)}
              >
                {pendingBinding ? `${formatWorkspaceKeyBinding(pendingBinding)}  →  …` : capturing === command.id ? "等待第 1 键" : formattedBinding}
              </button>
              <button
                type="button"
                className="workspace-key-disable"
                title={`禁用 ${command.label} 快捷键`}
                aria-label={`禁用 ${command.label} 快捷键`}
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
                title={`恢复 ${command.label} 默认快捷键`}
                aria-label={`恢复 ${command.label} 默认快捷键`}
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
  return (
    <div className="dialog-backdrop">
      <section className={`wind-dialog ${className}`}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{title}</strong>
          <button onClick={onClose}><X size={22} /></button>
        </header>
        {children}
      </section>
    </div>
  );
}

function SettingsSection({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section className="settings-section">
      {title ? <h2>{title}</h2> : null}
      <div className="settings-box">{children}</div>
    </section>
  );
}

function SettingRadio({ label, checked, name, onChange }: { label: string; checked: boolean; name: string; onChange: () => void }) {
  return (
    <label className="setting-radio">
      <input type="radio" name={name} checked={checked} onChange={onChange} />
      <span>{label}</span>
    </label>
  );
}

function SettingCheck({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className="setting-check">
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      <span>{label}</span>
    </label>
  );
}

function SettingInput({ label, value, type = "text", min, max, step, onChange }: { label: string; value: string | number; type?: string; min?: number; max?: number; step?: number; onChange: (value: string) => void }) {
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
  onBrowse,
  onChange,
}: {
  label: string;
  value: string;
  placeholder: string;
  canBrowse: boolean;
  onBrowse: () => void;
  onChange: (value: string) => void;
}) {
  return (
    <label className="setting-row terminal-export-path-setting">
      <span>{label}</span>
      <span className="setting-path-control">
        <input
          aria-label="终端文本默认导出目录"
          value={value}
          maxLength={MAX_TERMINAL_EXPORT_DIRECTORY_CHARACTERS}
          placeholder={placeholder}
          onChange={(event) => onChange(event.target.value)}
        />
        <button
          type="button"
          aria-label="选择终端文本导出目录"
          title={canBrowse ? "选择目录" : "目录选择仅在桌面版可用"}
          disabled={!canBrowse}
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
  return (
    <label className="setting-row">
      <span>{label}</span>
      <select aria-label={label} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)}>
        {options.map((option) => {
          const optionValue = typeof option === "string" ? option : option.value;
          const optionLabel = typeof option === "string" ? option || "未指定" : option.label;
          return <option key={optionValue || "blank"} value={optionValue}>
            {optionLabel}
          </option>;
        })}
      </select>
    </label>
  );
}

function SettingButtonRow({ label, children }: { label: string; children: ReactNode }) {
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
    completionTriggerChars: "1 字符",
    completionListHeight: "7 行",
    completionPreviewMode: "无处",
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
