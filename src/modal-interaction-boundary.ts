import { useEffect } from "react";

const MODAL_LAYER_SELECTOR = ".dialog-backdrop, .mcp-approval-backdrop, .screen-lock-overlay";
const MODAL_PANEL_SELECTOR = '[role="dialog"], [role="alertdialog"], .wind-dialog, .mcp-approval-dialog';
const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not(:disabled)",
  "input:not(:disabled)",
  "select:not(:disabled)",
  "textarea:not(:disabled)",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

export function activeModalLayer(root: Document = document): HTMLElement | null {
  const layers = [...root.querySelectorAll<HTMLElement>(MODAL_LAYER_SELECTOR)]
    .filter((layer) => modalLayerVisible(layer));
  let active: HTMLElement | null = null;
  let activeZIndex = Number.NEGATIVE_INFINITY;
  for (const layer of layers) {
    const zIndex = modalLayerZIndex(layer);
    if (!active || zIndex >= activeZIndex) {
      active = layer;
      activeZIndex = zIndex;
    }
  }
  return active;
}

export function hasActiveModalLayer(root: Document | undefined = typeof document === "undefined" ? undefined : document): boolean {
  return Boolean(root && activeModalLayer(root));
}

export function useModalInteractionBoundary() {
  useEffect(() => {
    let currentLayer: HTMLElement | null = null;
    let lastOutsideFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    let syncFrame: number | null = null;

    const focusLayer = (layer: HTMLElement) => {
      const panel = modalPanel(layer);
      if (!panel) return;
      if (!panel.hasAttribute("role")) panel.setAttribute("role", "dialog");
      panel.setAttribute("aria-modal", "true");
      if (!panel.hasAttribute("tabindex")) panel.tabIndex = -1;
      panel.focus({ preventScroll: true });
    };

    const syncLayer = () => {
      syncFrame = null;
      const nextLayer = activeModalLayer();
      document.body.toggleAttribute("data-portmate-modal-open", Boolean(nextLayer));
      for (const layer of document.querySelectorAll<HTMLElement>(MODAL_LAYER_SELECTOR)) {
        layer.dataset.modalLayer = layer === nextLayer ? "active" : "inactive";
      }
      if (nextLayer) {
        clearSelectionOutside(nextLayer);
        if (!nextLayer.contains(document.activeElement)) focusLayer(nextLayer);
      } else if (currentLayer && lastOutsideFocus?.isConnected) {
        lastOutsideFocus.focus({ preventScroll: true });
      }
      currentLayer = nextLayer;
    };

    const scheduleSync = () => {
      if (syncFrame !== null) return;
      syncFrame = window.requestAnimationFrame(syncLayer);
    };

    const blockOutsideLayer = (event: Event): boolean => {
      const layer = activeModalLayer();
      if (!layer || eventTargetInside(layer, event.target)) return false;
      if (event.cancelable) event.preventDefault();
      event.stopImmediatePropagation();
      focusLayer(layer);
      return true;
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (blockOutsideLayer(event)) return;
      const layer = activeModalLayer();
      if (!layer || event.key !== "Tab") return;
      const panel = modalPanel(layer);
      const controls = panel ? modalFocusableElements(panel) : [];
      if (!controls.length) {
        event.preventDefault();
        event.stopImmediatePropagation();
        focusLayer(layer);
        return;
      }
      const currentIndex = controls.indexOf(document.activeElement as HTMLElement);
      const wrapsBackward = event.shiftKey && currentIndex <= 0;
      const wrapsForward = !event.shiftKey && (currentIndex < 0 || currentIndex === controls.length - 1);
      if (!wrapsBackward && !wrapsForward) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      controls[wrapsBackward ? controls.length - 1 : 0].focus({ preventScroll: true });
    };

    const handleFocusIn = (event: FocusEvent) => {
      const layer = activeModalLayer();
      if (!layer) {
        if (event.target instanceof HTMLElement) lastOutsideFocus = event.target;
        return;
      }
      if (!eventTargetInside(layer, event.target)) focusLayer(layer);
    };

    const handleSelectionChange = () => {
      const layer = activeModalLayer();
      if (layer) clearSelectionOutside(layer);
    };

    const observer = new MutationObserver(scheduleSync);
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "hidden", "style", "aria-hidden"],
    });
    window.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("pointerdown", blockOutsideLayer, true);
    window.addEventListener("mousedown", blockOutsideLayer, true);
    window.addEventListener("click", blockOutsideLayer, true);
    window.addEventListener("selectstart", blockOutsideLayer, true);
    document.addEventListener("focusin", handleFocusIn, true);
    document.addEventListener("selectionchange", handleSelectionChange);
    scheduleSync();
    return () => {
      observer.disconnect();
      if (syncFrame !== null) window.cancelAnimationFrame(syncFrame);
      window.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("pointerdown", blockOutsideLayer, true);
      window.removeEventListener("mousedown", blockOutsideLayer, true);
      window.removeEventListener("click", blockOutsideLayer, true);
      window.removeEventListener("selectstart", blockOutsideLayer, true);
      document.removeEventListener("focusin", handleFocusIn, true);
      document.removeEventListener("selectionchange", handleSelectionChange);
      document.body.removeAttribute("data-portmate-modal-open");
    };
  }, []);
}

function modalLayerVisible(layer: HTMLElement): boolean {
  if (layer.hidden || layer.getAttribute("aria-hidden") === "true") return false;
  const style = window.getComputedStyle(layer);
  return style.display !== "none" && style.visibility !== "hidden";
}

function modalLayerZIndex(layer: HTMLElement): number {
  const parsed = Number.parseInt(window.getComputedStyle(layer).zIndex, 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

function modalPanel(layer: HTMLElement): HTMLElement | null {
  return layer.matches(MODAL_PANEL_SELECTOR)
    ? layer
    : layer.querySelector<HTMLElement>(MODAL_PANEL_SELECTOR);
}

function modalFocusableElements(panel: HTMLElement): HTMLElement[] {
  return [...panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)].filter((element) => {
    if (element.hidden || element.closest("[hidden], [inert], [aria-hidden=\"true\"]")) return false;
    const style = window.getComputedStyle(element);
    return style.display !== "none" && style.visibility !== "hidden" && element.getClientRects().length > 0;
  });
}

function eventTargetInside(layer: HTMLElement, target: EventTarget | null): boolean {
  return target instanceof Node && layer.contains(target);
}

function clearSelectionOutside(layer: HTMLElement) {
  const selection = document.getSelection();
  if (!selection || !selection.rangeCount) return;
  if (selection.anchorNode && layer.contains(selection.anchorNode)
    && selection.focusNode && layer.contains(selection.focusNode)) return;
  selection.removeAllRanges();
}
