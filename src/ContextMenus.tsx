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
  const left = Math.max(8, Math.min(state.x, window.innerWidth - 318));
  const top = Math.max(8, Math.min(state.y, window.innerHeight - 580));
  const sessionId = active?.profile.id ?? state.sessionId;
  const disabled = !active;
  const status = active?.runtime.status;
  const reconnectDisabled = connectionBusy || !status || status === "connecting" || status === "reconnecting";
  const disconnectDisabled = connectionBusy || !status || sessionConnectionAction(status) !== "disconnect";

  return (
    <div className="portmate-context-menu" aria-label="会话菜单" tabIndex={-1} style={{ left, top }} onClick={(event) => event.stopPropagation()} onContextMenu={(event) => event.preventDefault()}>
      <ContextSubmenu label="设置标签页颜色(C)" disabled={disabled}>
        <div className="context-color-grid">
          {colors.map((color) => (
            <button key={color.value} type="button" onClick={() => onColor(color.value)}>
              <span style={{ background: color.value }} />
              {color.label}
            </button>
          ))}
        </div>
      </ContextSubmenu>
      <ContextMenuButton label={syncInput ? "关闭同步输入(S)" : "开启同步输入(S)"} checked={syncInput} onClick={() => onAction("sync-toggle", sessionId)} />
      <ContextMenuButton label="粘贴(P)" shortcut="Ctrl+V" disabled={disabled} onClick={() => onAction("paste", sessionId)} />
      <ContextMenuButton label="重命名会话(R)" disabled={disabled || profileBusy} onClick={() => onAction("rename", sessionId)} />
      <ContextMenuButton label="复制会话(D)" shortcut="Ctrl+Shift+D" disabled={disabled} onClick={() => onAction("duplicate", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label="复制会话名称(N)" disabled={disabled} onClick={() => onAction("copy-name", sessionId)} />
      <ContextMenuButton label="复制会话 URL(U)" disabled={disabled} onClick={() => onAction("copy-url", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label="重新连接会话(R)" shortcut="Return" disabled={reconnectDisabled} onClick={() => onAction("reconnect", sessionId)} />
      <ContextMenuButton label="保存会话(S)" shortcut="Ctrl+Shift+S" disabled={disabled || profileBusy} onClick={() => onAction("save", sessionId)} />
      <ContextMenuButton label="水平拆分视图(H)" shortcut="Alt+H" disabled={disabled} onClick={() => onAction("split-h", sessionId)} />
      <ContextMenuButton label="垂直拆分视图(V)" shortcut="Alt+V" disabled={disabled} onClick={() => onAction("split-v", sessionId)} />
      <ContextMenuButton label="移动视图到分组(M)" disabled={disabled || profileBusy} onClick={() => onAction("move-group", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label="断开会话(C)" disabled={disconnectDisabled} onClick={() => onAction("close", sessionId)} />
      <ContextMenuButton label="断开所有会话(A)" disabled={!active} onClick={() => onAction("close-all", sessionId)} />
      <ContextMenuButton label="断开所有非活动会话(I)" disabled={!active} onClick={() => onAction("close-inactive", sessionId)} />
      <ContextMenuButton label="断开右侧会话(R)" disabled={!active} onClick={() => onAction("close-side", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label="会话设置...(S)" disabled={disabled || profileBusy} onClick={() => onAction("settings", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label="删除会话 Profile" disabled={disabled || profileBusy} danger onClick={() => onAction("delete-profile", sessionId)} />
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
  const left = Math.max(8, Math.min(state.x, window.innerWidth - 252));
  const top = Math.max(8, Math.min(state.y, window.innerHeight - 460));
  return (
    <div className="portmate-context-menu terminal-context-menu" aria-label="终端菜单" tabIndex={-1} style={{ left, top }} onClick={(event) => event.stopPropagation()} onContextMenu={(event) => event.preventDefault()}>
      <ContextMenuButton label="复制" shortcut="Ctrl+Shift+C" disabled={!state.hasSelection} onClick={() => onAction("copy")} />
      <ContextMenuButton label="粘贴" shortcut="Ctrl+V" onClick={() => onAction("paste")} />
      <ContextMenuButton label="查找" shortcut="Ctrl+Shift+F" onClick={() => onAction("find")} />
      <ContextMenuButton label="在线搜索" onClick={() => onAction("search-online")} />
      <ContextDivider />
      <ContextMenuButton label="清除回滚" shortcut="Ctrl+Shift+L" onClick={() => onAction("clear-scrollback")} />
      <ContextMenuButton label="清除屏幕" shortcut="Ctrl+L" disabled={state.alternate} onClick={() => onAction("clear-screen")} />
      <ContextMenuButton label="清除屏幕和回滚" disabled={state.alternate} onClick={() => onAction("clear-all")} />
      <ContextDivider />
      <ContextMenuButton label="选择全部" shortcut="Ctrl+Shift+A" onClick={() => onAction("select-all")} />
      <ContextMenuButton label="清除选择" disabled={!state.hasSelection} onClick={() => onAction("clear-selection")} />
      <ContextDivider />
      <ContextMenuButton label="导出终端文本" disabled={exportBusy} onClick={() => onAction("export-buffer")} />
      <ContextMenuButton label="导出终端文本到..." disabled={exportBusy} onClick={() => onAction("export-buffer-to")} />
      <ContextMenuButton label="导出选中文本" disabled={exportBusy || !state.hasSelection} onClick={() => onAction("export-selection")} />
      <ContextDivider />
      <ContextMenuButton label="管理触发器..." onClick={() => onAction("triggers")} />
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
  return <div className="context-divider" />;
}
