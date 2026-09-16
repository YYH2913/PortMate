import { useEffect, useRef } from "react";
import { hasActiveModalLayer, MODAL_LAYER_ACTIVATED_EVENT } from "./modal-interaction-boundary";
import { workspaceDockIds, workspaceDockPanelIds } from "./workspace-panel-state";
import type { WorkspaceDockId, WorkspaceDockPanelId } from "./workspace-panel-state";
import type { WorkspacePaneDirection } from "./workspace-state";

export type WorkspaceDockDropAnchor = { panel: WorkspaceDockPanelId; after: boolean };
type Source = { kind: "dock"; panel: WorkspaceDockPanelId }
  | { kind: "view"; paneId: string; viewId: string };
type Target = { kind: "dock"; dock: WorkspaceDockId; anchor?: WorkspaceDockDropAnchor }
  | { kind: "view"; paneId: string; index: number }
  | { kind: "split"; paneId: string; edge: WorkspacePaneDirection };
type Options = {
  onDockDragChange: (panel: WorkspaceDockPanelId | null) => void;
  onDockDrop: (panel: WorkspaceDockPanelId, dock: WorkspaceDockId, anchor?: WorkspaceDockDropAnchor) => void;
  onViewDrop: (sourcePaneId: string, viewId: string, targetPaneId: string, index: number) => void;
  onSplitDrop: (sourcePaneId: string, viewId: string, targetPaneId: string, edge: WorkspacePaneDirection) => void;
};

export function workspacePointerAfter(clientX: number, left: number, width: number, rtl: boolean): boolean {
  return rtl ? clientX < left + width / 2 : clientX >= left + width / 2;
}

export function workspacePointerDropZone(
  rect: Pick<DOMRect, "left" | "top" | "width" | "height">, x: number, y: number,
): WorkspacePaneDirection | "center" {
  const dx = Math.min(1, Math.max(0, (x - rect.left) / Math.max(1, rect.width)));
  const dy = Math.min(1, Math.max(0, (y - rect.top) / Math.max(1, rect.height)));
  const edges: [WorkspacePaneDirection, number][] = [["left", dx], ["right", 1 - dx], ["up", dy], ["down", 1 - dy]];
  const nearest = edges.reduce((best, edge) => edge[1] < best[1] ? edge : best);
  return nearest[1] <= 0.24 ? nearest[0] : "center";
}

/** Internal gestures cannot use HTML DnD while Windows WebView owns native file drops. */
export function useWorkspacePointerDrag(options: Options): void {
  const latest = useRef(options);
  latest.current = options;
  useEffect(() => {
    let drag: { source: Source; element: HTMLElement; root: HTMLElement; pointerId: number; x: number; y: number; pointerX: number; pointerY: number; active: boolean } | null = null;
    let suppressClickUntil = 0;
    let scrollFrame: number | null = null;
    let lastScrollTime = 0;
    const stopAutoScroll = () => {
      if (scrollFrame !== null) window.cancelAnimationFrame(scrollFrame);
      scrollFrame = null;
      lastScrollTime = 0;
    };
    const clearIndicators = () => {
      document.querySelectorAll<HTMLElement>("[data-panel-drop-position], [data-drop-position], [data-view-drop-zone], [data-pointer-dock-target]").forEach(element => {
        delete element.dataset.panelDropPosition;
        delete element.dataset.dropPosition;
        delete element.dataset.viewDropZone;
        delete element.dataset.pointerDockTarget;
      });
    };
    const finish = () => {
      const previous = drag;
      drag = null;
      stopAutoScroll();
      if (previous) {
        previous.root.classList.remove("workspace-pointer-dragging");
        try {
          if (previous.root.hasPointerCapture(previous.pointerId)) previous.root.releasePointerCapture(previous.pointerId);
        } catch { /* Capture may be released by the OS before pointercancel. */ }
        if (previous.active) {
          suppressClickUntil = performance.now() + 500;
          latest.current.onDockDragChange(null);
        }
      }
      clearIndicators();
    };
    function targetAt(x: number, y: number): Target | null {
      if (!drag || x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) return null;
      const element = document.elementFromPoint(x, y);
      if (!element || !drag.root.contains(element) || element.closest("[inert]")) return null;
      if (drag.source.kind === "dock") {
        const target = element.closest<HTMLElement>("[data-dock-target], .workspace-dock[data-dock]");
        const dock = target?.dataset.dockTarget ?? target?.dataset.dock;
        if (!workspaceDockIds.includes(dock as WorkspaceDockId)) return null;
        const tab = element.closest<HTMLElement>(".workspace-dock-tab[data-panel]");
        if (tab && workspaceDockPanelIds.includes(tab.dataset.panel as WorkspaceDockPanelId)) {
          const rect = tab.getBoundingClientRect();
          const after = workspacePointerAfter(x, rect.left, rect.width, getComputedStyle(tab).direction === "rtl");
          tab.dataset.panelDropPosition = after ? "after" : "before";
          return { kind: "dock", dock: dock as WorkspaceDockId, anchor: { panel: tab.dataset.panel as WorkspaceDockPanelId, after } };
        }
        if (target) target.dataset.pointerDockTarget = "true";
        return { kind: "dock", dock: dock as WorkspaceDockId };
      }
      const pane = element.closest<HTMLElement>(".terminal-pane[data-pane-id]");
      if (!pane?.dataset.paneId) return null;
      const tabs = [...pane.querySelectorAll<HTMLElement>(":scope > header .workspace-pane-tab[data-view-id]")];
      const tab = element.closest<HTMLElement>(".workspace-pane-tab[data-view-id]");
      if (tab) {
        const rect = tab.getBoundingClientRect();
        const after = workspacePointerAfter(x, rect.left, rect.width, getComputedStyle(tab).direction === "rtl");
        tab.dataset.dropPosition = after ? "after" : "before";
        const index = tabs.indexOf(tab);
        return index < 0 ? null : { kind: "view", paneId: pane.dataset.paneId, index: index + Number(after) };
      }
      const tabStrip = element.closest<HTMLElement>(".workspace-pane-tabs");
      if (tabStrip) {
        tabStrip.dataset.dropPosition = "end";
        return { kind: "view", paneId: pane.dataset.paneId, index: tabs.length };
      }
      const edge = workspacePointerDropZone(pane.getBoundingClientRect(), x, y);
      pane.dataset.viewDropZone = edge;
      return edge === "center" ? { kind: "view", paneId: pane.dataset.paneId, index: tabs.length }
        : { kind: "split", paneId: pane.dataset.paneId, edge };
    }
    function autoScrollTarget(): { strip: HTMLElement; velocity: number } | null {
      if (!drag?.active) return null;
      const { pointerX: x, pointerY: y, root, source } = drag;
      if (x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) return null;
      const element = document.elementFromPoint(x, y);
      if (!element || !root.contains(element) || element.closest("[inert]")) return null;
      const strip = element.closest<HTMLElement>(source.kind === "dock" ? ".workspace-dock-tabs" : ".workspace-pane-tabs");
      if (!strip || strip.scrollWidth <= strip.clientWidth) return null;
      const rect = strip.getBoundingClientRect();
      const left = Math.max(0, rect.left);
      const right = Math.min(innerWidth, rect.right);
      const edgeWidth = Math.min(36, (right - left) / 4);
      if (edgeWidth <= 0 || y < rect.top || y > rect.bottom) return null;
      const direction = x - left < edgeWidth ? -1 : right - x < edgeWidth ? 1 : 0;
      if (!direction) return null;
      const distance = direction < 0 ? x - left : right - x;
      const velocity = direction * (120 + 780 * Math.max(0, 1 - distance / edgeWidth));
      return { strip, velocity };
    }
    function autoScroll(time: number) {
      scrollFrame = null;
      if (!drag?.active || !drag.element.isConnected) { finish(); return; }
      const target = autoScrollTarget();
      if (!target) { lastScrollTime = 0; return; }
      const elapsed = lastScrollTime ? Math.min(32, Math.max(0, time - lastScrollTime)) : 1000 / 60;
      if (elapsed < 8) { scrollFrame = window.requestAnimationFrame(autoScroll); return; }
      lastScrollTime = time;
      const previous = target.strip.scrollLeft;
      // scrollLeft uses physical deltas in modern LTR and RTL engines (RTL's
      // leftward range is negative). Clamp long frames to avoid large jumps.
      target.strip.scrollLeft += target.velocity * elapsed / 1000;
      if (target.strip.scrollLeft === previous) { lastScrollTime = 0; return; }
      clearIndicators();
      targetAt(drag.pointerX, drag.pointerY);
      scrollFrame = window.requestAnimationFrame(autoScroll);
    }
    function updateAutoScroll() {
      if (!autoScrollTarget()) { stopAutoScroll(); return; }
      if (scrollFrame === null) {
        lastScrollTime = 0;
        scrollFrame = window.requestAnimationFrame(autoScroll);
      }
    }
    const down = (event: PointerEvent) => {
      // A fresh gesture is never the compatibility click emitted by the last drag.
      // Reset even outside a tab so an immediate toolbar/resize click is not swallowed.
      suppressClickUntil = 0;
      if (event.button !== 0 || !event.isPrimary || hasActiveModalLayer()) return;
      const element = event.target instanceof Element ? event.target.closest<HTMLElement>(".workspace-dock-tab-label, .workspace-pane-tab-label") : null;
      const root = element?.closest<HTMLElement>(".wind-root");
      if (!element || !root) return;
      finish();
      suppressClickUntil = 0;
      let source: Source;
      const panel = element.closest<HTMLElement>(".workspace-dock-tab")?.dataset.panel;
      if (panel && workspaceDockPanelIds.includes(panel as WorkspaceDockPanelId)) source = { kind: "dock", panel: panel as WorkspaceDockPanelId };
      else {
        const paneId = element.closest<HTMLElement>(".terminal-pane")?.dataset.paneId;
        const viewId = element.closest<HTMLElement>(".workspace-pane-tab")?.dataset.viewId;
        if (!paneId || !viewId) return;
        source = { kind: "view", paneId, viewId };
      }
      drag = { source, element, root, pointerId: event.pointerId, x: event.clientX, y: event.clientY, pointerX: event.clientX, pointerY: event.clientY, active: false };
    };
    const move = (event: PointerEvent) => {
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (!(event.buttons & 1) || !drag.element.isConnected) { finish(); return; }
      if (!drag.active && Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 5) return;
      if (!drag.active) {
        drag.active = true;
        drag.root.classList.add("workspace-pointer-dragging");
        try { drag.root.setPointerCapture(event.pointerId); } catch { /* Window listeners still track the gesture. */ }
        if (drag.source.kind === "dock") latest.current.onDockDragChange(drag.source.panel);
      }
      event.preventDefault();
      event.stopPropagation();
      drag.pointerX = event.clientX;
      drag.pointerY = event.clientY;
      clearIndicators();
      targetAt(event.clientX, event.clientY);
      updateAutoScroll();
    };
    const up = (event: PointerEvent) => {
      if (!drag || drag.pointerId !== event.pointerId) return;
      const source = drag.source;
      const active = drag.active;
      const target = active && drag.element.isConnected ? targetAt(event.clientX, event.clientY) : null;
      if (active) { event.preventDefault(); event.stopPropagation(); }
      finish();
      if (source.kind === "dock" && target?.kind === "dock") latest.current.onDockDrop(source.panel, target.dock, target.anchor);
      else if (source.kind === "view" && target?.kind === "view") latest.current.onViewDrop(source.paneId, source.viewId, target.paneId, target.index);
      else if (source.kind === "view" && target?.kind === "split") latest.current.onSplitDrop(source.paneId, source.viewId, target.paneId, target.edge);
    };
    const cancel = (event: PointerEvent) => { if (drag?.pointerId === event.pointerId) finish(); };
    const lostCapture = (event: PointerEvent) => { if (drag?.active && drag.pointerId === event.pointerId && event.target === drag.root) finish(); };
    const key = (event: KeyboardEvent) => { if (drag && event.key === "Escape") { event.preventDefault(); event.stopPropagation(); finish(); } };
    const click = (event: MouseEvent) => {
      // Keyboard and assistive-technology activation have no pointer compatibility
      // click to suppress; cancellation must not swallow the next Enter/Space.
      if (event.detail === 0) return;
      if (performance.now() < suppressClickUntil) { suppressClickUntil = 0; event.preventDefault(); event.stopPropagation(); }
    };
    const nativeDrag = (event: DragEvent) => { if (drag) { event.preventDefault(); event.stopPropagation(); } };
    document.addEventListener("pointerdown", down, true);
    window.addEventListener("pointermove", move, { capture: true, passive: false });
    window.addEventListener("pointerup", up, true);
    window.addEventListener("pointercancel", cancel, true);
    window.addEventListener("lostpointercapture", lostCapture, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("blur", finish);
    window.addEventListener(MODAL_LAYER_ACTIVATED_EVENT, finish);
    document.addEventListener("click", click, true);
    document.addEventListener("dragstart", nativeDrag, true);
    return () => {
      finish();
      document.removeEventListener("pointerdown", down, true);
      window.removeEventListener("pointermove", move, true);
      window.removeEventListener("pointerup", up, true);
      window.removeEventListener("pointercancel", cancel, true);
      window.removeEventListener("lostpointercapture", lostCapture, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("blur", finish);
      window.removeEventListener(MODAL_LAYER_ACTIVATED_EVENT, finish);
      document.removeEventListener("click", click, true);
      document.removeEventListener("dragstart", nativeDrag, true);
    };
  }, []);
}
