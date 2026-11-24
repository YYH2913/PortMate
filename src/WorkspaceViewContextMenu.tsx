import { t, useLocale } from "./i18n";
import {
  ArrowDown,
  ArrowLeft,
  ArrowLeftRight,
  ArrowRight,
  ArrowUp,
  Ban,
  Columns2,
  Copy,
  Download,
  ExternalLink,
  FileDown,
  FileText,
  FolderInput,
  LayoutGrid,
  Link2,
  Maximize2,
  Pencil,
  Play,
  Power,
  Radio,
  RefreshCw,
  RotateCcw,
  Rows2,
  Save,
  Settings,
  X,
} from "lucide-react";
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
      <span className="context-section-label">{t("context-section-edit")}</span>
      <ContextMenuButton
        label={syncInput ? t("disable-synchronized-input") : t("enable-synchronized-input")}
        icon={Radio}
        checked={syncInput}
        disabled={sessionStatus !== "connected"}
        onClick={() => onAction("sync-toggle")}
      />
      <ContextMenuButton label={t("duplicate-view")} icon={Copy} disabled={!canDuplicate} onClick={onDuplicate} />
      <ContextMenuButton label={t("rename-session")} icon={Pencil} disabled={profileBusy} onClick={onRename} />
      <ContextMenuButton label={t("copy-session-name")} icon={FileText} onClick={() => onAction("copy-name")} />
      <ContextMenuButton label={t("copy-session-url")} icon={Link2} onClick={() => onAction("copy-url")} />
      <ContextDivider />
      <span className="context-section-label">{t("context-section-session")}</span>
      <ContextMenuButton label={t("start-session")} icon={Play} disabled={startDisabled} onClick={() => onAction("start")} />
      <ContextMenuButton label={t("reconnect-session")} icon={RefreshCw} disabled={reconnectDisabled} onClick={() => onAction("reconnect")} />
      <ContextMenuButton label={t("disconnect-session")} icon={Power} disabled={disconnectDisabled} danger onClick={() => onAction("disconnect")} />
      <ContextMenuButton label={t("save-session-settings")} icon={Save} disabled={profileBusy} onClick={() => onAction("save")} />
      <ContextDivider />
      <span className="context-section-label">{t("context-section-export")}</span>
      <ContextMenuButton label={t("export-terminal-text")} icon={Download} disabled={exportBusy} onClick={() => onAction("export-buffer")} />
      <ContextMenuButton label={t("export-terminal-text-to")} icon={FileDown} disabled={exportBusy} onClick={() => onAction("export-buffer-to")} />
      <ContextMenuButton label={t("export-selected-text")} icon={FileText} disabled={exportBusy || !hasSelection} onClick={() => onAction("export-selection")} />
      <ContextDivider />
      <span className="context-section-label">{t("context-section-layout")}</span>
      <ContextMenuButton label={t("split-view-horizontally")} icon={Columns2} onClick={() => onAction("split-horizontal")} />
      <ContextMenuButton label={t("split-view-vertically")} icon={Rows2} onClick={() => onAction("split-vertical")} />
      <ContextMenuButton label={t("move-view-to-group")} icon={FolderInput} disabled={!canMove} onClick={() => onAction("move-group")} />
      <ContextSubmenu label={t("move-to-new-group")} icon={LayoutGrid} disabled={!canMoveToNewGroup}>
        <ContextMenuButton label={t("left")} icon={ArrowLeft} onClick={() => onAction("move-new-left")} />
        <ContextMenuButton label={t("right")} icon={ArrowRight} onClick={() => onAction("move-new-right")} />
        <ContextMenuButton label={t("above")} icon={ArrowUp} onClick={() => onAction("move-new-up")} />
        <ContextMenuButton label={t("below")} icon={ArrowDown} onClick={() => onAction("move-new-down")} />
      </ContextSubmenu>
      <ContextMenuButton label={t("move-to-new-window")} icon={ExternalLink} disabled={!canDetach} onClick={() => onAction("detach-pane")} />
      <ContextDivider />
      <span className="context-section-label">{t("context-section-tabs")}</span>
      <ContextMenuButton label={t("close-view")} icon={X} disabled={!canClose} onClick={() => onAction("close")} />
      <ContextMenuButton label={t("close-other-views")} icon={X} disabled={!canCloseOther} onClick={() => onAction("close-other")} />
      <ContextMenuButton label={t("close-views-to-the-right")} icon={X} disabled={!canCloseRight} onClick={() => onAction("close-right")} />
      <ContextMenuButton label={t("reopen-closed-view")} icon={RotateCcw} disabled={!canReopen} onClick={() => onAction("reopen")} />
      <ContextMenuButton label={t("close-pane-3")} icon={X} disabled={!canClosePane} danger onClick={() => onAction("close-pane")} />
      <ContextDivider />
      <span className="context-section-label">{t("context-section-panes")}</span>
      <ContextMenuButton label={t("merge-current-group")} icon={LayoutGrid} disabled={!canMerge} onClick={() => onAction("merge-group")} />
      <ContextSubmenu label={t("swap-panes")} icon={ArrowLeftRight} disabled={!Object.values(canSwap).some(Boolean)}>
        <ContextMenuButton label={t("up")} icon={ArrowUp} disabled={!canSwap.up} onClick={() => onAction("swap-up")} />
        <ContextMenuButton label={t("down")} icon={ArrowDown} disabled={!canSwap.down} onClick={() => onAction("swap-down")} />
        <ContextMenuButton label={t("left-2")} icon={ArrowLeft} disabled={!canSwap.left} onClick={() => onAction("swap-left")} />
        <ContextMenuButton label={t("right-2")} icon={ArrowRight} disabled={!canSwap.right} onClick={() => onAction("swap-right")} />
      </ContextSubmenu>
      <ContextMenuButton label={t("toggle-pane-zoom")} icon={Maximize2} disabled={!canZoom} onClick={() => onAction("toggle-zoom")} />
      <ContextDivider />
      <ContextMenuButton label={t("session-settings-2")} icon={Settings} disabled={profileBusy} onClick={() => onAction("settings")} />
    </div>
  );
}
