import { t, useLocale } from "./i18n";
import { Ban } from "lucide-react";
import { ContextDivider, ContextMenuButton, ContextSubmenu } from "./ContextMenus";
import { useMeasuredMenuPosition } from "./context-menu-position";
import { sessionConnectionAction } from "./session-runtime-state";
import type { WorkspaceView } from "./workspace-state";
import type { SessionStatus } from "./types";

export type WorkspaceViewContextAction =
  | "start"
  | "disconnect"
  | "sync-toggle"
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
  syncInput = false,
  connectionBusy = false,
  profileBusy = false,
  exportBusy = false,
  hasSelection = false,
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
  syncInput?: boolean;
  connectionBusy?: boolean;
  profileBusy?: boolean;
  exportBusy?: boolean;
  hasSelection?: boolean;
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
  const { ref, style } = useMeasuredMenuPosition(state.x, state.y, 244, 560);
  const startDisabled = connectionBusy || sessionStatus === "connecting" || sessionStatus === "connected" || sessionStatus === "reconnecting";
  const reconnectDisabled = connectionBusy || sessionStatus === "connecting" || sessionStatus === "reconnecting";
  const disconnectDisabled = connectionBusy || sessionConnectionAction(sessionStatus) !== "disconnect";

  return (
    <div
      ref={ref}
      className="portmate-context-menu workspace-view-context-menu"
      aria-label={t("view-menu")}
      tabIndex={-1}
      style={style}
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
      <ContextDivider />
      <ContextMenuButton
        label={syncInput ? t("disable-synchronized-input") : t("enable-synchronized-input")}
        checked={syncInput}
        disabled={sessionStatus !== "connected"}
        onClick={() => onAction("sync-toggle")}
      />
      <ContextMenuButton label={t("duplicate-view")} disabled={!canDuplicate} onClick={onDuplicate} />
      <ContextMenuButton label={t("rename-session")} disabled={profileBusy} onClick={onRename} />
      <ContextDivider />
      <ContextMenuButton label={t("copy-session-name")} onClick={() => onAction("copy-name")} />
      <ContextMenuButton label={t("copy-session-url")} onClick={() => onAction("copy-url")} />
      <ContextDivider />
      <ContextMenuButton label={t("start-session")} disabled={startDisabled} onClick={() => onAction("start")} />
      <ContextMenuButton label={t("reconnect-session")} disabled={reconnectDisabled} onClick={() => onAction("reconnect")} />
      <ContextMenuButton label={t("disconnect-session")} disabled={disconnectDisabled} onClick={() => onAction("disconnect")} />
      <ContextMenuButton label={t("save-session-settings")} disabled={profileBusy} onClick={() => onAction("save")} />
      <ContextMenuButton label={t("export-terminal-text")} disabled={exportBusy} onClick={() => onAction("export-buffer")} />
      <ContextMenuButton label={t("export-terminal-text-to")} disabled={exportBusy} onClick={() => onAction("export-buffer-to")} />
      <ContextMenuButton label={t("export-selected-text")} disabled={exportBusy || !hasSelection} onClick={() => onAction("export-selection")} />
      <ContextDivider />
      <ContextMenuButton label={t("split-view-horizontally")} onClick={() => onAction("split-horizontal")} />
      <ContextMenuButton label={t("split-view-vertically")} onClick={() => onAction("split-vertical")} />
      <ContextMenuButton label={t("move-view-to-group")} disabled={!canMove} onClick={() => onAction("move-group")} />
      <ContextSubmenu label={t("move-to-new-group")} disabled={!canMoveToNewGroup}>
        <ContextMenuButton label={t("left")} onClick={() => onAction("move-new-left")} />
        <ContextMenuButton label={t("right")} onClick={() => onAction("move-new-right")} />
        <ContextMenuButton label={t("above")} onClick={() => onAction("move-new-up")} />
        <ContextMenuButton label={t("below")} onClick={() => onAction("move-new-down")} />
      </ContextSubmenu>
      <ContextMenuButton label={t("move-to-new-window")} disabled={!canDetach} onClick={() => onAction("detach-pane")} />
      <ContextDivider />
      <ContextMenuButton label={t("close-view")} disabled={!canClose} onClick={() => onAction("close")} />
      <ContextMenuButton label={t("close-other-views")} disabled={!canCloseOther} onClick={() => onAction("close-other")} />
      <ContextMenuButton label={t("close-views-to-the-right")} disabled={!canCloseRight} onClick={() => onAction("close-right")} />
      <ContextMenuButton label={t("reopen-closed-view")} disabled={!canReopen} onClick={() => onAction("reopen")} />
      <ContextMenuButton label={t("close-pane-3")} disabled={!canClosePane} onClick={() => onAction("close-pane")} />
      <ContextDivider />
      <ContextMenuButton label={t("merge-current-group")} disabled={!canMerge} onClick={() => onAction("merge-group")} />
      <ContextSubmenu label={t("swap-panes")} disabled={!Object.values(canSwap).some(Boolean)}>
        <ContextMenuButton label={t("up")} disabled={!canSwap.up} onClick={() => onAction("swap-up")} />
        <ContextMenuButton label={t("down")} disabled={!canSwap.down} onClick={() => onAction("swap-down")} />
        <ContextMenuButton label={t("left-2")} disabled={!canSwap.left} onClick={() => onAction("swap-left")} />
        <ContextMenuButton label={t("right-2")} disabled={!canSwap.right} onClick={() => onAction("swap-right")} />
      </ContextSubmenu>
      <ContextMenuButton label={t("toggle-pane-zoom")} disabled={!canZoom} onClick={() => onAction("toggle-zoom")} />
      <ContextDivider />
      <ContextMenuButton label={t("session-settings-2")} disabled={profileBusy} onClick={() => onAction("settings")} />
    </div>
  );
}
