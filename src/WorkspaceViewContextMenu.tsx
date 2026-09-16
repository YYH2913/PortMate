import { t, useLocale } from "./i18n";
import { Ban } from "lucide-react";
import type { ReactNode } from "react";
import type { WorkspaceView } from "./workspace-state";
import type { SessionStatus } from "./types";

export type WorkspaceViewContextAction =
  | "copy-name"
  | "copy-url"
  | "reconnect"
  | "save"
  | "export-buffer"
  | "export-buffer-to"
  | "export-selection"
  | "split-horizontal"
  | "split-vertical"
  | "move-group"
  | "move-new-left"
  | "move-new-right"
  | "move-new-up"
  | "move-new-down"
  | "detach-pane"
  | "merge-group"
  | "swap-up"
  | "swap-down"
  | "swap-left"
  | "swap-right"
  | "toggle-zoom"
  | "close"
  | "close-other"
  | "close-right"
  | "reopen"
  | "close-pane"
  | "settings";

export default function WorkspaceViewContextMenu({
  state,
  view,
  sessionStatus,
  connectionBusy = false,
  profileBusy = false,
  exportBusy = false,
  label,
  colors,
  canDuplicate,
  canClose,
  canCloseOther,
  canCloseRight,
  canMove,
  canMoveToNewGroup,
  canDetach,
  canClosePane,
  canMerge,
  canSwap,
  canZoom,
  canReopen,
  onColor,
  onDuplicate,
  onRename,
  onAction,
}: {
  state: { x: number; y: number };
  view: WorkspaceView;
  sessionStatus: SessionStatus;
  connectionBusy?: boolean;
  profileBusy?: boolean;
  exportBusy?: boolean;
  label: string;
  colors: readonly { label: string; value: string }[];
  canDuplicate: boolean;
  canClose: boolean;
  canCloseOther: boolean;
  canCloseRight: boolean;
  canMove: boolean;
  canMoveToNewGroup: boolean;
  canDetach: boolean;
  canClosePane: boolean;
  canMerge: boolean;
  canSwap: Readonly<Record<"up" | "down" | "left" | "right", boolean>>;
  canZoom: boolean;
  canReopen: boolean;
  onColor: (color: string) => void;
  onDuplicate: () => void;
  onRename: () => void;
  onAction: (action: WorkspaceViewContextAction) => void;
}) {
  useLocale();
  const left = Math.max(8, Math.min(state.x, window.innerWidth - 252));
  const top = Math.max(8, Math.min(state.y, window.innerHeight - 560));
  return (
    <div
      className="portmate-context-menu workspace-view-context-menu"
      aria-label={t("view-menu")}
      tabIndex={-1}
      style={{ left, top }}
      onClick={(event) => event.stopPropagation()}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
    >
      <div className="workspace-view-context-title" title={label}>
        <span className={view.color ? "tab-mark colored" : "tab-mark"} style={view.color ? { background: view.color } : undefined} />
        <strong>{label}</strong>
      </div>
      <span className="context-section-label">{t("tab-color")}</span>
      <div className="workspace-view-color-grid" role="group" aria-label={t("tab-color")}>
        {colors.map((color) => (
          <button
            key={color.value}
            type="button"
            className={view.color === color.value ? "active" : ""}
            title={t(color.label)}
            aria-label={t(color.label)}
            aria-pressed={view.color === color.value}
            onClick={() => onColor(color.value)}
          >
            <span style={{ background: color.value }} />
          </button>
        ))}
        <button
          type="button"
          className={!view.color ? "active clear" : "clear"}
          title={t("clear-color")}
          aria-label={t("clear-color")}
          aria-pressed={!view.color}
          onClick={() => onColor("")}
        >
          <Ban size={13} />
        </button>
      </div>
      <Divider />
      <MenuButton label={t("duplicate-view")} disabled={!canDuplicate} onClick={onDuplicate} />
      <MenuButton label={t("rename-view")} onClick={onRename} />
      <Divider />
      <MenuButton label={t("copy-session-name")} onClick={() => onAction("copy-name")} />
      <MenuButton label={t("copy-session-url")} onClick={() => onAction("copy-url")} />
      <Divider />
      <MenuButton label={t("reconnect-session")} disabled={connectionBusy || sessionStatus === "connecting" || sessionStatus === "reconnecting"} onClick={() => onAction("reconnect")} />
      <MenuButton label={t("save-session-settings")} disabled={profileBusy} onClick={() => onAction("save")} />
      <MenuButton label={t("export-terminal-text")} disabled={exportBusy} onClick={() => onAction("export-buffer")} />
      <MenuButton label={t("export-terminal-text-to")} disabled={exportBusy} onClick={() => onAction("export-buffer-to")} />
      <MenuButton label={t("export-selected-text")} disabled={exportBusy} onClick={() => onAction("export-selection")} />
      <Divider />
      <MenuButton label={t("split-view-horizontally")} onClick={() => onAction("split-horizontal")} />
      <MenuButton label={t("split-view-vertically")} onClick={() => onAction("split-vertical")} />
      <MenuButton label={t("move-view-to-group")} disabled={!canMove} onClick={() => onAction("move-group")} />
      <ContextSubmenu label={t("move-to-new-group")} disabled={!canMoveToNewGroup}>
        <MenuButton label={t("left")} onClick={() => onAction("move-new-left")} />
        <MenuButton label={t("right")} onClick={() => onAction("move-new-right")} />
        <MenuButton label={t("above")} onClick={() => onAction("move-new-up")} />
        <MenuButton label={t("below")} onClick={() => onAction("move-new-down")} />
      </ContextSubmenu>
      <MenuButton label={t("move-to-new-window")} disabled={!canDetach} onClick={() => onAction("detach-pane")} />
      <Divider />
      <MenuButton label={t("close-view")} disabled={!canClose} onClick={() => onAction("close")} />
      <MenuButton label={t("close-other-views")} disabled={!canCloseOther} onClick={() => onAction("close-other")} />
      <MenuButton label={t("close-views-to-the-right")} disabled={!canCloseRight} onClick={() => onAction("close-right")} />
      <MenuButton label={t("reopen-closed-view")} disabled={!canReopen} onClick={() => onAction("reopen")} />
      <MenuButton label={t("close-pane-3")} disabled={!canClosePane} onClick={() => onAction("close-pane")} />
      <Divider />
      <MenuButton label={t("merge-current-group")} disabled={!canMerge} onClick={() => onAction("merge-group")} />
      <ContextSubmenu label={t("swap-panes")} disabled={!Object.values(canSwap).some(Boolean)}>
        <MenuButton label={t("up")} disabled={!canSwap.up} onClick={() => onAction("swap-up")} />
        <MenuButton label={t("down")} disabled={!canSwap.down} onClick={() => onAction("swap-down")} />
        <MenuButton label={t("left-2")} disabled={!canSwap.left} onClick={() => onAction("swap-left")} />
        <MenuButton label={t("right-2")} disabled={!canSwap.right} onClick={() => onAction("swap-right")} />
      </ContextSubmenu>
      <MenuButton label={t("toggle-pane-zoom")} disabled={!canZoom} onClick={() => onAction("toggle-zoom")} />
      <Divider />
      <MenuButton label={t("session-settings-2")} disabled={profileBusy} onClick={() => onAction("settings")} />
    </div>
  );
}

function MenuButton({ label, disabled, onClick }: { label: string; disabled?: boolean; onClick: () => void }) {
  useLocale();
  return (
    <button type="button" className="context-menu-row" disabled={disabled} onClick={onClick}>
      <span className="context-check" />
      <span className="context-label">{label}</span>
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
      <button type="button" className="context-menu-row" aria-label={label} disabled={disabled}>
        <span className="context-check" />
        <span className="context-label">{label}</span>
        <span className="context-arrow">›</span>
      </button>
      {!disabled ? <div className="context-submenu-panel">{children}</div> : null}
    </div>
  );
}

function Divider() {
  useLocale();
  return <div className="context-divider" />;
}
