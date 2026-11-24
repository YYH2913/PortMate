import { useEffect } from "react";
import { InteractionLayerIndex } from "./interaction-layer-index";

const MODAL_LAYER_SELECTOR = ".dialog-backdrop, .mcp-approval-backdrop, .screen-lock-overlay";
const POPUP_LAYER_SELECTOR = ".portmate-context-menu, .menu-popover";
const MODAL_PANEL_SELECTOR = '[role="dialog"], [role="alertdialog"], .wind-dialog, .mcp-approval-dialog';
const SELECTION_SCOPE_SELECTOR = `${MODAL_LAYER_SELECTOR}, ${POPUP_LAYER_SELECTOR}, .terminal-pane, .detached-pane-root, .serial-analyzer-root`;
const FOCUSABLE_SELECTOR = [
  "a[href]",
  "details > summary:first-of-type",
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
  "input",
  "compositionstart",
  "compositionupdate",
  "compositionend",
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
export const MODAL_LAYER_ACTIVATED_EVENT = "portmate-modal-layer-activated";

export type ModalLayerActivatedDetail = {
  layer: HTMLElement;
};

type InteractionLayer = {
  element: HTMLElement;
  modal: boolean;
};

const layerIndexes = new WeakMap<Document, InteractionLayerIndex>();
function layerIndex(root: Document): InteractionLayerIndex {
  let index = layerIndexes.get(root);
  if (!index) {
    index = new InteractionLayerIndex(root, `${MODAL_LAYER_SELECTOR}, ${POPUP_LAYER_SELECTOR}`);
    layerIndexes.set(root, index);
  }
  return index;
}

export function activeModalLayer(root: Document = document): HTMLElement | null {
  const layers = layerIndex(root).elements()
    .filter((layer) => layer.matches(MODAL_LAYER_SELECTOR) && modalLayerVisible(layer));
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
    let dismissedPopupLayer: HTMLElement | null = null;
    const lastFocusByLayer = new WeakMap<HTMLElement, HTMLElement>();
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
      const previous = lastFocusByLayer.get(layer.element);
      const restore = previous && panel.contains(previous) && !previous.matches(":disabled")
        && !previous.closest("[inert], [hidden]") && previous.getClientRects().length > 0;
      (restore ? previous : firstControl ?? panel).focus({ preventScroll: true });
    };

    const syncLayer = () => {
      syncFrame = null;
      const next = resolveActiveInteractionLayer(document);
      const nextLayer = next?.element ?? null;
      if (next?.modal) {
        const popup = visiblePopupLayers(document).at(-1) ?? null;
        if (popup && dismissedPopupLayer !== popup) {
          dismissedPopupLayer = popup;
          window.dispatchEvent(new Event(INTERACTION_LAYER_DISMISS_EVENT));
        }
      } else {
        dismissedPopupLayer = null;
      }
      document.body.toggleAttribute("data-portmate-modal-open", Boolean(next?.modal));
      document.body.toggleAttribute("data-portmate-interaction-layer-open", Boolean(next));
      for (const layer of layerIndex(document).elements()) {
        if (layer.matches(MODAL_LAYER_SELECTOR)) layer.dataset.modalLayer = layer === nextLayer ? "active" : "inactive";
        else layer.dataset.interactionLayer = layer === nextLayer ? "active" : "inactive";
      }
      if (next && nextLayer) {
        if (next.modal) isolateLayer(nextLayer);
        else restoreInertState();
        clearSelectionOutside(nextLayer);
        if (next.modal && currentLayer !== nextLayer) {
          window.dispatchEvent(new CustomEvent<ModalLayerActivatedDetail>(MODAL_LAYER_ACTIVATED_EVENT, {
            detail: { layer: nextLayer },
          }));
        }
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
      if (currentLayer && !currentLayer.isConnected) {
        // The previously isolated modal unmounted; clear stale inert markers
        // and stale modal-open flag immediately so underlying dialogs/menus do
        // not drop fast input before the frame-batched focus sync runs.
        restoreInertState();
        if (!activeModalLayer(document)) {
          document.body.removeAttribute("data-portmate-modal-open");
        }
      }
      if (syncFrame !== null) return;
      syncFrame = window.requestAnimationFrame(syncLayer);
    };

    const dismissPopup = (layer: InteractionLayer) => {
      if (!layer.modal) window.dispatchEvent(new Event(INTERACTION_LAYER_DISMISS_EVENT));
    };

    const blockOutsideLayer = (event: Event): boolean => {
      const layer = resolveActiveInteractionLayer(document);
      if (!layer || eventTargetInside(layer.element, event.target)) return false;
      const targetElement = event.target instanceof Element ? event.target : null;
      if (!layer.modal && layer.element.matches(".menu-popover") && targetElement?.closest(".menu-trigger, .menu-appearance-trigger")) {
        // Clicking or hovering another top-bar menu trigger should switch/toggle in one interaction.
        const escapeFromTrigger = event instanceof KeyboardEvent && event.type === "keydown" && event.key === "Escape";
        if (escapeFromTrigger || ["pointerdown", "pointerup", "pointermove", "mousedown", "mouseup", "click"].includes(event.type)) {
          return false;
        }
      }
      if (!layer.modal && event.type === "contextmenu" && targetElement?.closest(".tree-session, .workspace-pane-tab, .terminal-host, .file-row[data-file-path]")) {
        // Right-clicking another context-menu host switches directly to the new context menu in one click.
        dismissPopup(layer);
        return false;
      }
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
        if (event.target instanceof Element && event.target.closest(".context-submenu-panel")) {
          // Allow an open submenu panel to handle Escape by collapsing only itself.
          return;
        }
        event.preventDefault();
        event.stopImmediatePropagation();
        const menuTrigger = layer.element.matches(".menu-popover")
          ? layer.element.parentElement?.querySelector<HTMLButtonElement>(".menu-trigger, .menu-appearance-trigger") ?? null
          : null;
        dismissPopup(layer);
        if (menuTrigger) requestAnimationFrame(() => menuTrigger.focus({ preventScroll: true }));
        return;
      }
      const menuNavigation = !layer.modal && layer.element.getAttribute("role") !== "dialog"
        && ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key);
      if (!menuNavigation && event.key !== "Tab") return;
      const panel = interactionPanel(layer);
      const submenu = event.target instanceof Element ? event.target.closest(".context-submenu-panel") : null;
      const controls = panel ? interactionFocusableElements(panel).filter(element =>
        !menuNavigation || element.closest(".context-submenu-panel") === submenu) : [];
      if (menuNavigation) {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!controls.length) return;
        const currentIndex = controls.indexOf(document.activeElement as HTMLElement);
        const nextIndex = event.key === "Home"
          ? 0
          : event.key === "End"
            ? controls.length - 1
            : (Math.max(0, currentIndex) + (event.key === "ArrowDown" ? 1 : -1) + controls.length) % controls.length;
        controls[nextIndex].focus();
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
      controls[wrapsBackward ? controls.length - 1 : 0].focus();
    };

    const handleFocusIn = (event: FocusEvent) => {
      const layer = resolveActiveInteractionLayer(document);
      if (!layer) {
        if (event.target instanceof HTMLElement) lastOutsideFocus = event.target;
        return;
      }
      if (!layer.modal && layer.element.matches(".menu-popover")
        && event.target instanceof Element && event.target.closest(".menu-trigger, .menu-appearance-trigger")) {
        return;
      }
      if (!eventTargetInside(layer.element, event.target)) focusLayer(layer);
      else if (event.target instanceof HTMLElement) lastFocusByLayer.set(layer.element, event.target);
    };

    const handleSelectionChange = () => {
      const layer = resolveActiveInteractionLayer(document);
      if (layer) clearSelectionOutside(layer.element);
      else clearSelectionAcrossScopes();
    };

    const handleCapturedEvent = (event: Event) => {
      if (event.type === "keydown" && event instanceof KeyboardEvent) handleKeyDown(event);
      else blockOutsideLayer(event);
    };

    const unsubscribeLayers = layerIndex(document).subscribe(scheduleSync);
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
      unsubscribeLayers();
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
  const popups = visiblePopupLayers(root);
  if (!popups.length) return null;
  return { element: popups.at(-1)!, modal: false };
}

function visiblePopupLayers(root: Document): HTMLElement[] {
  return layerIndex(root).elements()
    .filter((layer) => layer.matches(POPUP_LAYER_SELECTOR) && modalLayerVisible(layer));
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
    if (element.getAttribute("tabindex") === "-1") return false;
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

function clearSelectionAcrossScopes() {
  const selection = document.getSelection();
  if (!selection?.rangeCount || !selection.anchorNode || !selection.focusNode) return;
  const anchorScope = selectionScope(selection.anchorNode);
  const focusScope = selectionScope(selection.focusNode);
  if (anchorScope !== focusScope && (anchorScope || focusScope)) selection.removeAllRanges();
}

function selectionScope(node: Node): Element | null {
  const element = node instanceof Element ? node : node.parentElement;
  return element?.closest(SELECTION_SCOPE_SELECTOR) ?? null;
}
