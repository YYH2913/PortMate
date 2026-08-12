import { useEffect } from "react";

const MODAL_LAYER_SELECTOR = ".dialog-backdrop, .mcp-approval-backdrop, .screen-lock-overlay";
const POPUP_LAYER_SELECTOR = ".portmate-context-menu, .menu-popover";
const MODAL_PANEL_SELECTOR = '[role="dialog"], [role="alertdialog"], .wind-dialog, .mcp-approval-dialog';
const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not(:disabled)",
  "input:not(:disabled)",
  "select:not(:disabled)",
  "textarea:not(:disabled)",
  '[tabindex]:not([tabindex="-1"])',
].join(",");
const BLOCKED_OUTSIDE_EVENTS = [
  "keydown",
  "keyup",
  "keypress",
  "beforeinput",
  "copy",
  "cut",
  "paste",
  "pointerdown",
  "pointerup",
  "pointermove",
  "pointercancel",
  "mousedown",
  "mouseup",
  "click",
  "dblclick",
  "auxclick",
  "contextmenu",
  "selectstart",
  "dragstart",
  "dragover",
  "drop",
  "touchstart",
  "touchend",
  "wheel",
] as const;

export const INTERACTION_LAYER_DISMISS_EVENT = "portmate-dismiss-interaction-layer";

type InteractionLayer = {
  element: HTMLElement;
  modal: boolean;
};

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

export function activeInteractionLayer(root: Document = document): HTMLElement | null {
  return resolveActiveInteractionLayer(root)?.element ?? null;
}

export function hasActiveInteractionLayer(root: Document | undefined = typeof document === "undefined" ? undefined : document): boolean {
  return Boolean(root && resolveActiveInteractionLayer(root));
}

export function useModalInteractionBoundary() {
  useEffect(() => {
    let currentLayer: HTMLElement | null = null;
    let lastOutsideFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    let syncFrame: number | null = null;
    const inertState = new Map<HTMLElement, boolean>();

    const restoreInertState = () => {
      for (const [element, inert] of inertState) {
        if (element.isConnected) element.inert = inert;
      }
      inertState.clear();
    };

    const isolateLayer = (layer: HTMLElement) => {
      restoreInertState();
      let branch: HTMLElement = layer;
      let parent = branch.parentElement;
      while (parent) {
        for (const sibling of parent.children) {
          if (!(sibling instanceof HTMLElement) || sibling === branch || sibling.contains(branch)) continue;
          if (!inertState.has(sibling)) inertState.set(sibling, sibling.inert);
          sibling.inert = true;
        }
        branch = parent;
        parent = parent.parentElement;
      }
    };

    const focusLayer = (layer: InteractionLayer) => {
      const panel = interactionPanel(layer);
      if (!panel) return;
      if (!panel.hasAttribute("role")) panel.setAttribute("role", layer.modal ? "dialog" : "menu");
      if (layer.modal) panel.setAttribute("aria-modal", "true");
      if (!panel.hasAttribute("tabindex")) panel.tabIndex = -1;
      const firstControl = !layer.modal ? interactionFocusableElements(panel)[0] : null;
      (firstControl ?? panel).focus({ preventScroll: true });
    };

    const syncLayer = () => {
      syncFrame = null;
      const next = resolveActiveInteractionLayer(document);
      const nextLayer = next?.element ?? null;
      document.body.toggleAttribute("data-portmate-modal-open", Boolean(next?.modal));
      document.body.toggleAttribute("data-portmate-interaction-layer-open", Boolean(next));
      for (const layer of document.querySelectorAll<HTMLElement>(MODAL_LAYER_SELECTOR)) {
        layer.dataset.modalLayer = layer === nextLayer ? "active" : "inactive";
      }
      for (const layer of document.querySelectorAll<HTMLElement>(POPUP_LAYER_SELECTOR)) {
        layer.dataset.interactionLayer = layer === nextLayer ? "active" : "inactive";
      }
      if (next && nextLayer) {
        if (next.modal) isolateLayer(nextLayer);
        else restoreInertState();
        if (next.modal) clearSelectionOutside(nextLayer);
        if (!nextLayer.contains(document.activeElement)) focusLayer(next);
      } else if (currentLayer && lastOutsideFocus?.isConnected) {
        restoreInertState();
        lastOutsideFocus.focus({ preventScroll: true });
      } else {
        restoreInertState();
      }
      currentLayer = nextLayer;
    };

    const scheduleSync = () => {
      if (syncFrame !== null) return;
      syncFrame = window.requestAnimationFrame(syncLayer);
    };

    const dismissPopup = (layer: InteractionLayer) => {
      if (!layer.modal) window.dispatchEvent(new Event(INTERACTION_LAYER_DISMISS_EVENT));
    };

    const blockOutsideLayer = (event: Event): boolean => {
      const layer = resolveActiveInteractionLayer(document);
      if (!layer || eventTargetInside(layer.element, event.target)) return false;
      if (!layer.modal && event.type === "wheel") {
        dismissPopup(layer);
        return false;
      }
      if (event.cancelable) event.preventDefault();
      event.stopImmediatePropagation();
      if (["click", "contextmenu", "wheel"].includes(event.type)) {
        dismissPopup(layer);
      } else {
        focusLayer(layer);
      }
      return true;
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (blockOutsideLayer(event)) return;
      const layer = resolveActiveInteractionLayer(document);
      if (!layer) return;
      if (!layer.modal && event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        dismissPopup(layer);
        return;
      }
      const panel = interactionPanel(layer);
      const controls = panel ? interactionFocusableElements(panel) : [];
      if (!layer.modal && ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!controls.length) return;
        const currentIndex = controls.indexOf(document.activeElement as HTMLElement);
        const nextIndex = event.key === "Home"
          ? 0
          : event.key === "End"
            ? controls.length - 1
            : (Math.max(0, currentIndex) + (event.key === "ArrowDown" ? 1 : -1) + controls.length) % controls.length;
        controls[nextIndex].focus({ preventScroll: true });
        return;
      }
      if (event.key !== "Tab") return;
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
      const layer = resolveActiveInteractionLayer(document);
      if (!layer) {
        if (event.target instanceof HTMLElement) lastOutsideFocus = event.target;
        return;
      }
      if (!eventTargetInside(layer.element, event.target)) focusLayer(layer);
    };

    const handleSelectionChange = () => {
      const layer = resolveActiveInteractionLayer(document);
      if (layer?.modal) clearSelectionOutside(layer.element);
    };

    const handleCapturedEvent = (event: Event) => {
      if (event.type === "keydown" && event instanceof KeyboardEvent) handleKeyDown(event);
      else blockOutsideLayer(event);
    };

    const observer = new MutationObserver(scheduleSync);
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "hidden", "style", "aria-hidden"],
    });
    const captureOptions: AddEventListenerOptions = {
      capture: true,
      passive: false,
    };
    for (const eventName of BLOCKED_OUTSIDE_EVENTS) {
      window.addEventListener(eventName, handleCapturedEvent, captureOptions);
    }
    document.addEventListener("focusin", handleFocusIn, true);
    document.addEventListener("selectionchange", handleSelectionChange);
    scheduleSync();
    return () => {
      observer.disconnect();
      if (syncFrame !== null) window.cancelAnimationFrame(syncFrame);
      for (const eventName of BLOCKED_OUTSIDE_EVENTS) {
        window.removeEventListener(eventName, handleCapturedEvent, true);
      }
      document.removeEventListener("focusin", handleFocusIn, true);
      document.removeEventListener("selectionchange", handleSelectionChange);
      restoreInertState();
      document.body.removeAttribute("data-portmate-modal-open");
      document.body.removeAttribute("data-portmate-interaction-layer-open");
    };
  }, []);
}

function resolveActiveInteractionLayer(root: Document): InteractionLayer | null {
  const modal = activeModalLayer(root);
  if (modal) return { element: modal, modal: true };
  const popups = [...root.querySelectorAll<HTMLElement>(POPUP_LAYER_SELECTOR)]
    .filter((layer) => modalLayerVisible(layer));
  if (!popups.length) return null;
  return { element: popups.at(-1)!, modal: false };
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

function interactionPanel(layer: InteractionLayer): HTMLElement | null {
  if (!layer.modal) return layer.element;
  return layer.element.matches(MODAL_PANEL_SELECTOR)
    ? layer.element
    : layer.element.querySelector<HTMLElement>(MODAL_PANEL_SELECTOR);
}

function interactionFocusableElements(panel: HTMLElement): HTMLElement[] {
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
