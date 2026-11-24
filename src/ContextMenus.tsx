import { t, useLocale } from "./i18n";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";
import { Ban } from "lucide-react";
import { sessionConnectionAction } from "./session-runtime-state";
import {
  computeSubmenuPosition,
  isMacPlatform,
  useMeasuredMenuPosition,
  type ViewportPoint,
} from "./context-menu-position";
import type { SessionSummary } from "./types";

const useIsomorphicLayoutEffect = typeof document !== "undefined" ? useLayoutEffect : useEffect;

export type SessionContextAction =
  | "start"
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
  const { ref, style } = useMeasuredMenuPosition(state.x, state.y, 310, 560);
  const sessionId = active?.profile.id ?? state.sessionId;
  const disabled = !active;
  const status = active?.runtime.status;
  const startDisabled = connectionBusy || !status || status === "connecting" || status === "connected" || status === "reconnecting";
  const reconnectDisabled = connectionBusy || !status || status === "connecting" || status === "reconnecting";
  const disconnectDisabled = connectionBusy || !status || sessionConnectionAction(status) !== "disconnect";
  const isMac = isMacPlatform();

  return (
    <div
      ref={ref}
      className="portmate-context-menu"
      aria-label={t("session-menu")}
      tabIndex={-1}
      style={style}
      onClick={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
    >
      <ContextSubmenu label={t("set-tab-color")} disabled={disabled}>
        <div className="context-color-grid">
          {colors.map((color) => (
            <button key={color.value} type="button" onClick={() => onColor(color.value)}>
              <span style={{ background: color.value }} />
              {t(color.label)}
            </button>
          ))}
          <button type="button" className="context-color-clear" onClick={() => onColor("")}>
            <Ban size={13} />
            {t("clear-color")}
          </button>
        </div>
      </ContextSubmenu>
      <ContextMenuButton
        label={syncInput ? t("disable-synchronized-input") : t("enable-synchronized-input")}
        checked={syncInput}
        disabled={!active || status !== "connected"}
        onClick={() => onAction("sync-toggle", sessionId)}
      />
      <ContextMenuButton
        label={t("paste")}
        shortcut={isMac ? "Cmd+V" : "Ctrl+V"}
        disabled={!active || status !== "connected"}
        onClick={() => onAction("paste", sessionId)}
      />
      <ContextMenuButton label={t("rename-session")} disabled={disabled || profileBusy} onClick={() => onAction("rename", sessionId)} />
      <ContextMenuButton label={t("duplicate-session")} disabled={disabled} onClick={() => onAction("duplicate", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("copy-session-name")} disabled={disabled} onClick={() => onAction("copy-name", sessionId)} />
      <ContextMenuButton label={t("copy-session-url")} disabled={disabled} onClick={() => onAction("copy-url", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("start-session")} disabled={startDisabled} onClick={() => onAction("start", sessionId)} />
      <ContextMenuButton label={t("reconnect-session")} disabled={reconnectDisabled} onClick={() => onAction("reconnect", sessionId)} />
      <ContextMenuButton label={t("save-session")} disabled={disabled || profileBusy} onClick={() => onAction("save", sessionId)} />
      <ContextMenuButton label={t("split-view-horizontally")} disabled={disabled} onClick={() => onAction("split-h", sessionId)} />
      <ContextMenuButton label={t("split-view-vertically")} disabled={disabled} onClick={() => onAction("split-v", sessionId)} />
      <ContextMenuButton label={t("move-session-to-folder")} disabled={disabled || profileBusy} onClick={() => onAction("move-group", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("disconnect-session")} disabled={disconnectDisabled} onClick={() => onAction("close", sessionId)} />
      <ContextMenuButton label={t("disconnect-all-sessions")} disabled={!active} onClick={() => onAction("close-all", sessionId)} />
      <ContextMenuButton label={t("disconnect-inactive-sessions")} disabled={!active} onClick={() => onAction("close-inactive", sessionId)} />
      <ContextMenuButton label={t("disconnect-sessions-below")} disabled={!active} onClick={() => onAction("close-side", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("session-settings-2")} disabled={disabled || profileBusy} onClick={() => onAction("settings", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label={t("delete-session-profile")} disabled={disabled || profileBusy} danger onClick={() => onAction("delete-profile", sessionId)} />
    </div>
  );
}

export function TerminalContextMenu({
  state,
  exportBusy = false,
  showTriggers = true,
  onAction,
}: {
  state: { x: number; y: number; alternate: boolean; hasSelection: boolean };
  exportBusy?: boolean;
  showTriggers?: boolean;
  onAction: (action: TerminalContextAction) => void;
}) {
  useLocale();
  const { ref, style } = useMeasuredMenuPosition(state.x, state.y, 244, 420);
  const mod = isMacPlatform() ? "Cmd" : "Ctrl";
  return (
    <div
      ref={ref}
      className="portmate-context-menu terminal-context-menu"
      aria-label={t("terminal-menu")}
      tabIndex={-1}
      style={style}
      onClick={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
    >
      <ContextMenuButton label={t("copy")} shortcut={`${mod}+Shift+C`} disabled={!state.hasSelection} onClick={() => onAction("copy")} />
      <ContextMenuButton label={t("paste")} shortcut={`${mod}+V`} onClick={() => onAction("paste")} />
      <ContextMenuButton label={t("find")} shortcut={`${mod}+F`} onClick={() => onAction("find")} />
      <ContextMenuButton label={t("search-online")} onClick={() => onAction("search-online")} />
      <ContextDivider />
      <ContextMenuButton label={t("clear-scrollback")} shortcut={`${mod}+Shift+L`} onClick={() => onAction("clear-scrollback")} />
      <ContextMenuButton label={t("clear-screen")} shortcut={`${mod}+L`} disabled={state.alternate} onClick={() => onAction("clear-screen")} />
      <ContextMenuButton label={t("clear-screen-and-scrollback")} disabled={state.alternate} onClick={() => onAction("clear-all")} />
      <ContextDivider />
      <ContextMenuButton label={t("select-all")} shortcut={`${mod}+Shift+A`} onClick={() => onAction("select-all")} />
      <ContextMenuButton label={t("clear-selection")} disabled={!state.hasSelection} onClick={() => onAction("clear-selection")} />
      <ContextDivider />
      <ContextMenuButton label={t("export-terminal-text")} disabled={exportBusy} onClick={() => onAction("export-buffer")} />
      <ContextMenuButton label={t("export-terminal-text-to")} disabled={exportBusy} onClick={() => onAction("export-buffer-to")} />
      <ContextMenuButton label={t("export-selected-text")} disabled={exportBusy || !state.hasSelection} onClick={() => onAction("export-selection")} />
      {showTriggers ? (
        <>
          <ContextDivider />
          <ContextMenuButton label={t("manage-triggers")} onClick={() => onAction("triggers")} />
        </>
      ) : null}
    </div>
  );
}

export function ContextMenuButton({
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

export function ContextSubmenu({
  label,
  disabled,
  children,
}: {
  label: string;
  disabled?: boolean;
  children: ReactNode;
}) {
  useLocale();
  const [open, setOpen] = useState(false);
  const [panelPosition, setPanelPosition] = useState<ViewportPoint | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  useIsomorphicLayoutEffect(() => {
    if (!open || disabled) return;
    const trigger = triggerRef.current;
    const panel = panelRef.current;
    if (!trigger || !panel) return;
    const menu = trigger.closest(".portmate-context-menu");
    const update = () => {
      const next = computeSubmenuPosition(trigger.getBoundingClientRect(), panel.offsetWidth, panel.offsetHeight,
        window.innerWidth, window.innerHeight, document.documentElement.dir === "rtl");
      setPanelPosition((current) => (current && current.left === next.left && current.top === next.top ? current : next));
    };
    update();
    menu?.addEventListener("scroll", update);
    window.addEventListener("resize", update);
    return () => {
      menu?.removeEventListener("scroll", update);
      window.removeEventListener("resize", update);
    };
  }, [open, disabled]);

  function openAndFocusFirst() {
    if (disabled) return;
    setOpen(true);
    requestAnimationFrame(() => {
      panelRef.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus({ preventScroll: true });
    });
  }

  function handleTriggerKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>) {
    if (disabled) return;
    const rtl = typeof document !== "undefined" && document.documentElement.dir === "rtl";
    const openKey = rtl ? "ArrowLeft" : "ArrowRight";
    if (event.key === openKey || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      event.stopPropagation();
      openAndFocusFirst();
    }
  }

  function handlePanelKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    const rtl = typeof document !== "undefined" && document.documentElement.dir === "rtl";
    const closeKey = rtl ? "ArrowRight" : "ArrowLeft";
    if (event.key === closeKey || event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
      requestAnimationFrame(() => triggerRef.current?.focus({ preventScroll: true }));
    }
  }

  const className = [
    "context-submenu",
    disabled ? "disabled" : "",
    open && !disabled ? "open" : "",
  ].filter(Boolean).join(" ");

  return (
    <div
      className={className}
      onMouseEnter={() => { if (!disabled) setOpen(true); }}
      onMouseLeave={() => setOpen(false)}
    >
      <button
        ref={triggerRef}
        type="button"
        className="context-menu-row"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open && !disabled}
        disabled={disabled}
        onClick={(event) => {
          event.stopPropagation();
          openAndFocusFirst();
        }}
        onKeyDown={handleTriggerKeyDown}
      >
        <span className="context-check" />
        <span className="context-label">{label}</span>
        <span className="context-arrow" aria-hidden="true">›</span>
      </button>
      {!disabled ? (
        <div
          ref={panelRef}
          className="context-submenu-panel"
          role="menu"
          hidden={!open}
          style={open && panelPosition ? { position: "fixed", left: panelPosition.left, top: panelPosition.top } : undefined}
          onKeyDown={handlePanelKeyDown}
        >
          {children}
        </div>
      ) : null}
    </div>
  );
}

export function ContextDivider() {
  useLocale();
  return <div className="context-divider" />;
}
