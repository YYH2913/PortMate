import { useEffect, useRef } from "react";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Lock, PanelLeftOpen } from "lucide-react";
import { isBackendAvailable } from "./api";
import type { ScreenLockMarker } from "./screen-lock-state";

export default function ChildWindowScreenLockOverlay({
  marker,
  ownerWindowId,
}: {
  marker: ScreenLockMarker;
  ownerWindowId: string;
}) {
  const buttonRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => buttonRef.current?.focus({ preventScroll: true }));
    return () => window.cancelAnimationFrame(frame);
  }, []);

  async function focusOwnerWorkspace() {
    try {
      if (isBackendAvailable()) {
        const owner = await WebviewWindow.getByLabel(ownerWindowId);
        await owner?.setFocus();
        return;
      }
      window.opener?.focus();
    } catch {
      window.opener?.focus();
    }
  }

  const reason = marker.reason === "idle" ? "空闲超时" : marker.reason === "startup" ? "启动保护" : "手动锁定";
  return (
    <div
      className="screen-lock-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby="child-window-screen-lock-title"
      onKeyDown={(event) => {
        if (event.key === "Escape" || event.key === "Tab") {
          event.preventDefault();
          event.stopPropagation();
          buttonRef.current?.focus();
        }
      }}
    >
      <section className="screen-lock-panel">
        <div className="screen-lock-brand">
          <span className="screen-lock-icon"><Lock size={20} /></span>
          <span>PortMate</span>
        </div>
        <div className="screen-lock-heading">
          <h1 id="child-window-screen-lock-title">屏幕已锁定</h1>
          <span>{reason} · {new Date(marker.lockedAt).toLocaleTimeString()}</span>
        </div>
        <div className="screen-lock-rule" />
        <p className="screen-lock-message">请在来源工作区完成解锁</p>
        <button ref={buttonRef} className="screen-lock-primary" type="button" onClick={() => void focusOwnerWorkspace()}>
          <PanelLeftOpen size={15} />
          <span>切换到来源工作区</span>
        </button>
      </section>
    </div>
  );
}
