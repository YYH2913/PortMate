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
  const left = Math.max(8, Math.min(state.x, window.innerWidth - 252));
  const top = Math.max(8, Math.min(state.y, window.innerHeight - 560));
  return (
    <div
      className="portmate-context-menu workspace-view-context-menu"
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
      <span className="context-section-label">标签颜色</span>
      <div className="workspace-view-color-grid" role="group" aria-label="标签颜色">
        {colors.map((color) => (
          <button
            key={color.value}
            type="button"
            className={view.color === color.value ? "active" : ""}
            title={color.label}
            aria-label={color.label}
            aria-pressed={view.color === color.value}
            onClick={() => onColor(color.value)}
          >
            <span style={{ background: color.value }} />
          </button>
        ))}
        <button
          type="button"
          className={!view.color ? "active clear" : "clear"}
          title="清除颜色"
          aria-label="清除颜色"
          aria-pressed={!view.color}
          onClick={() => onColor("")}
        >
          <Ban size={13} />
        </button>
      </div>
      <Divider />
      <MenuButton label="复制视图" disabled={!canDuplicate} onClick={onDuplicate} />
      <MenuButton label="重命名视图" onClick={onRename} />
      <Divider />
      <MenuButton label="复制会话名称" onClick={() => onAction("copy-name")} />
      <MenuButton label="复制会话 URL" onClick={() => onAction("copy-url")} />
      <Divider />
      <MenuButton label="重新连接会话" disabled={sessionStatus === "connecting" || sessionStatus === "reconnecting"} onClick={() => onAction("reconnect")} />
      <MenuButton label="保存会话配置" onClick={() => onAction("save")} />
      <MenuButton label="导出终端文本" onClick={() => onAction("export-buffer")} />
      <MenuButton label="导出终端文本到..." onClick={() => onAction("export-buffer-to")} />
      <MenuButton label="导出选中文本" onClick={() => onAction("export-selection")} />
      <Divider />
      <MenuButton label="水平拆分视图" onClick={() => onAction("split-horizontal")} />
      <MenuButton label="垂直拆分视图" onClick={() => onAction("split-vertical")} />
      <MenuButton label="移动视图到分组" disabled={!canMove} onClick={() => onAction("move-group")} />
      <ContextSubmenu label="移到新分组" disabled={!canMoveToNewGroup}>
        <MenuButton label="左侧" onClick={() => onAction("move-new-left")} />
        <MenuButton label="右侧" onClick={() => onAction("move-new-right")} />
        <MenuButton label="上方" onClick={() => onAction("move-new-up")} />
        <MenuButton label="下方" onClick={() => onAction("move-new-down")} />
      </ContextSubmenu>
      <MenuButton label="移到新窗口" disabled={!canDetach} onClick={() => onAction("detach-pane")} />
      <Divider />
      <MenuButton label="关闭视图" disabled={!canClose} onClick={() => onAction("close")} />
      <MenuButton label="关闭其他视图" disabled={!canCloseOther} onClick={() => onAction("close-other")} />
      <MenuButton label="关闭右侧视图" disabled={!canCloseRight} onClick={() => onAction("close-right")} />
      <MenuButton label="重新打开已关闭视图" disabled={!canReopen} onClick={() => onAction("reopen")} />
      <MenuButton label="关闭窗格" disabled={!canClosePane} onClick={() => onAction("close-pane")} />
      <Divider />
      <MenuButton label="合并当前分组" disabled={!canMerge} onClick={() => onAction("merge-group")} />
      <ContextSubmenu label="交换窗格" disabled={!Object.values(canSwap).some(Boolean)}>
        <MenuButton label="向上" disabled={!canSwap.up} onClick={() => onAction("swap-up")} />
        <MenuButton label="向下" disabled={!canSwap.down} onClick={() => onAction("swap-down")} />
        <MenuButton label="向左" disabled={!canSwap.left} onClick={() => onAction("swap-left")} />
        <MenuButton label="向右" disabled={!canSwap.right} onClick={() => onAction("swap-right")} />
      </ContextSubmenu>
      <MenuButton label="切换窗格缩放" disabled={!canZoom} onClick={() => onAction("toggle-zoom")} />
      <Divider />
      <MenuButton label="会话设置..." onClick={() => onAction("settings")} />
    </div>
  );
}

function MenuButton({ label, disabled, onClick }: { label: string; disabled?: boolean; onClick: () => void }) {
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
  return <div className="context-divider" />;
}
