import { t, useLocale } from "./i18n";
import type { ReactNode } from "react";
import { sessionConnectionAction } from "./session-runtime-state";
import type { SessionSummary } from "./types";

export type SessionContextAction =
  | "sync-toggle"
  | "paste"
  | "rename"
  | "duplicate"
  | "copy-name"
  | "copy-url"
  | "reconnect"
  | "save"
  | "split-h"
  | "split-v"
  | "move-group"
  | "close"
  | "close-all"
  | "close-inactive"
  | "close-side"
  | "settings"
  | "delete-profile";

export type TerminalContextAction =
  | "copy"
  | "paste"
  | "find"
  | "search-online"
  | "clear-scrollback"
  | "clear-screen"
  | "clear-all"
  | "select-all"
  | "clear-selection"
  | "export-buffer"
  | "export-buffer-to"
  | "export-selection"
  | "triggers";

export function SessionContextMenu({
  state,
  active,
  connectionBusy = false,
  profileBusy = false,
  syncInput,
  colors,
  onAction,
  onColor,
}: {
  state: { x: number; y: number; sessionId: string | null };
  active?: SessionSummary;
  connectionBusy?: boolean;
  profileBusy?: boolean;
  syncInput: boolean;
  colors: readonly { label: string; value: string }[];
  onAction: (action: SessionContextAction, sessionId?: string | null) => void;
  onColor: (color: string) => void;
}) {
  useLocale();
  const left = Math.max(8, Math.min(state.x, window.innerWidth - 318));
  const top = Math.max(8, Math.min(state.y, window.innerHeight - 580));
  const sessionId = active?.profile.id ?? state.sessionId;
  const disabled = !active;
  const status = active?.runtime.status;
  const reconnectDisabled = connectionBusy || !status || status === "connecting" || status === "reconnecting";
  const disconnectDisabled = connectionBusy || !status || sessionConnectionAction(status) !== "disconnect";

  return (
    <div className="portmate-context-menu" aria-label={t("session-menu")} tabIndex={-1} style={{ left, top }} onClick={(event) => event.stopPropagation()} onContextMenu={(event) => event.preventDefault()}>
      <ContextSubmenu label={t("set-tab-color-c")} disabled={disabled}>
        <div className="context-color-grid">
          {colors.map((color) => (
            <button key={color.value} type="button" onClick={() => onColor(color.value)}>
              <span style={{ background: color.value }} />
              {t(color.label)}
            </button>
          ))}
        </div>
      </ContextSubmenu>
      <ContextMenuButton label={syncInput ? t("disable-synchronized-input-s") : t("enable-synchronized-input-s")} checked={syncInput} onClick={() => onAction("sync-toggle", sessionId)} />
      <ContextMenuButton label={t("paste-p")} shortcut="Ctrl+V" disabled={disabled} onClick={() => onAction("paste", sessionId)} />
      <ContextMenuButton label={t("rename-session-r")} disabled={disabled || profileBusy} onClick={() => onAction("rename", sessionId)} />
      <ContextMenuButton label={t("duplicate-session-d")} shortcut="Ctrl+Shift+D" disabled={disabled} onClick={() => onAction("duplicate", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("copy-session-name-n")} disabled={disabled} onClick={() => onAction("copy-name", sessionId)} />
      <ContextMenuButton label={t("copy-session-url-u")} disabled={disabled} onClick={() => onAction("copy-url", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("reconnect-session-r")} shortcut="Return" disabled={reconnectDisabled} onClick={() => onAction("reconnect", sessionId)} />
      <ContextMenuButton label={t("save-session-s")} shortcut="Ctrl+Shift+S" disabled={disabled || profileBusy} onClick={() => onAction("save", sessionId)} />
      <ContextMenuButton label={t("split-view-horizontally-h")} shortcut="Alt+H" disabled={disabled} onClick={() => onAction("split-h", sessionId)} />
      <ContextMenuButton label={t("split-view-vertically-v")} shortcut="Alt+V" disabled={disabled} onClick={() => onAction("split-v", sessionId)} />
      <ContextMenuButton label={t("move-view-to-group-m")} disabled={disabled || profileBusy} onClick={() => onAction("move-group", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("disconnect-session-c")} disabled={disconnectDisabled} onClick={() => onAction("close", sessionId)} />
      <ContextMenuButton label={t("disconnect-all-sessions-a")} disabled={!active} onClick={() => onAction("close-all", sessionId)} />
      <ContextMenuButton label={t("disconnect-inactive-sessions-i")} disabled={!active} onClick={() => onAction("close-inactive", sessionId)} />
      <ContextMenuButton label={t("disconnect-sessions-to-the-right-r")} disabled={!active} onClick={() => onAction("close-side", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("session-settings-s")} disabled={disabled || profileBusy} onClick={() => onAction("settings", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("delete-session-profile")} disabled={disabled || profileBusy} danger onClick={() => onAction("delete-profile", sessionId)} />
    </div>
  );
}

export function TerminalContextMenu({
  state,
  exportBusy = false,
  onAction,
}: {
  state: { x: number; y: number; alternate: boolean; hasSelection: boolean };
  exportBusy?: boolean;
  onAction: (action: TerminalContextAction) => void;
}) {
  useLocale();
  const left = Math.max(8, Math.min(state.x, window.innerWidth - 252));
  const top = Math.max(8, Math.min(state.y, window.innerHeight - 460));
  return (
    <div className="portmate-context-menu terminal-context-menu" aria-label={t("terminal-menu")} tabIndex={-1} style={{ left, top }} onClick={(event) => event.stopPropagation()} onContextMenu={(event) => event.preventDefault()}>
      <ContextMenuButton label={t("copy")} shortcut="Ctrl+Shift+C" disabled={!state.hasSelection} onClick={() => onAction("copy")} />
      <ContextMenuButton label={t("paste")} shortcut="Ctrl+V" onClick={() => onAction("paste")} />
      <ContextMenuButton label={t("find")} shortcut="Ctrl+Shift+F" onClick={() => onAction("find")} />
      <ContextMenuButton label={t("search-online")} onClick={() => onAction("search-online")} />
      <ContextDivider />
      <ContextMenuButton label={t("clear-scrollback")} shortcut="Ctrl+Shift+L" onClick={() => onAction("clear-scrollback")} />
      <ContextMenuButton label={t("clear-screen")} shortcut="Ctrl+L" disabled={state.alternate} onClick={() => onAction("clear-screen")} />
      <ContextMenuButton label={t("clear-screen-and-scrollback")} disabled={state.alternate} onClick={() => onAction("clear-all")} />
      <ContextDivider />
      <ContextMenuButton label={t("select-all")} shortcut="Ctrl+Shift+A" onClick={() => onAction("select-all")} />
      <ContextMenuButton label={t("clear-selection")} disabled={!state.hasSelection} onClick={() => onAction("clear-selection")} />
      <ContextDivider />
      <ContextMenuButton label={t("export-terminal-text")} disabled={exportBusy} onClick={() => onAction("export-buffer")} />
      <ContextMenuButton label={t("export-terminal-text-to")} disabled={exportBusy} onClick={() => onAction("export-buffer-to")} />
      <ContextMenuButton label={t("export-selected-text")} disabled={exportBusy || !state.hasSelection} onClick={() => onAction("export-selection")} />
      <ContextDivider />
      <ContextMenuButton label={t("manage-triggers")} onClick={() => onAction("triggers")} />
    </div>
  );
}

function ContextMenuButton({
  label,
  shortcut,
  disabled,
  checked,
  danger,
  onClick,
}: {
  label: string;
  shortcut?: string;
  disabled?: boolean;
  checked?: boolean;
  danger?: boolean;
  onClick?: () => void;
}) {
  useLocale();
  return (
    <button type="button" className={danger ? "context-menu-row danger" : "context-menu-row"} disabled={disabled} onClick={onClick}>
      <span className={checked ? "context-check active" : "context-check"}>{checked ? "✓" : ""}</span>
      <span className="context-label">{label}</span>
      {shortcut ? <span className="context-shortcut">{shortcut}</span> : null}
    </button>
  );
}

function ContextSubmenu({
  label,
  disabled,
  children,
}: {
  label: string;
  disabled?: boolean;
  children: ReactNode;
}) {
  useLocale();
  return (
    <div className={disabled ? "context-submenu disabled" : "context-submenu"}>
      <button type="button" className="context-menu-row" disabled={disabled}>
        <span className="context-check" />
        <span className="context-label">{label}</span>
        <span className="context-arrow">›</span>
      </button>
      {!disabled ? <div className="context-submenu-panel">{children}</div> : null}
    </div>
  );
}

function ContextDivider() {
  useLocale();
  return <div className="context-divider" />;
}
