import { useEffect, useLayoutEffect, useRef, useState } from "react";

const useIsomorphicLayoutEffect = typeof document !== "undefined" ? useLayoutEffect : useEffect;

export type ViewportPoint = {
  left: number;
  top: number;
};

export function isMacPlatform(): boolean {
  return typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
}

export function computeContextMenuPosition(
  x: number,
  y: number,
  width: number,
  height: number,
  viewportWidth: number,
  viewportHeight: number,
  margin = 8,
): ViewportPoint {
  const safeWidth = Math.max(0, width);
  const safeHeight = Math.max(0, height);
  const maxLeft = Math.max(margin, viewportWidth - safeWidth - margin);
  const maxTop = Math.max(margin, viewportHeight - safeHeight - margin);

  let left = x;
  if (x + safeWidth > viewportWidth - margin && x - safeWidth >= margin) {
    left = x - safeWidth;
  }
  left = Math.max(margin, Math.min(left, maxLeft));

  let top = y;
  if (y + safeHeight > viewportHeight - margin && y - safeHeight >= margin) {
    top = y - safeHeight;
  }
  top = Math.max(margin, Math.min(top, maxTop));

  return { left: Math.round(left), top: Math.round(top) };
}

export function computeSubmenuPosition(
  triggerRect: Pick<DOMRect, "left" | "right" | "top" | "bottom">,
  panelWidth: number,
  panelHeight: number,
  viewportWidth: number,
  viewportHeight: number,
  rtl: boolean,
  margin = 8,
): ViewportPoint {
  // Touch the trigger so the pointer can cross without leaving the submenu.
  const maxLeft = Math.max(margin, viewportWidth - panelWidth - margin);
  const maxTop = Math.max(margin, viewportHeight - panelHeight - margin);

  const preferredLeft = rtl
    ? triggerRect.left - panelWidth
    : triggerRect.right;
  const flippedLeft = rtl
    ? triggerRect.right
    : triggerRect.left - panelWidth;

  const fitsPreferred = preferredLeft >= margin && preferredLeft + panelWidth <= viewportWidth - margin;
  const fitsFlipped = flippedLeft >= margin && flippedLeft + panelWidth <= viewportWidth - margin;

  const rawLeft = fitsPreferred ? preferredLeft : fitsFlipped ? flippedLeft : preferredLeft;
  const left = Math.max(margin, Math.min(rawLeft, maxLeft));
  const top = Math.max(margin, Math.min(triggerRect.top - 5, maxTop));

  return { left: Math.round(left), top: Math.round(top) };
}

export function useMeasuredMenuPosition(
  x: number,
  y: number,
  fallbackWidth: number,
  fallbackHeight: number,
) {
  const ref = useRef<HTMLDivElement>(null);
  const viewportWidth = typeof window === "undefined" ? 1440 : window.innerWidth;
  const viewportHeight = typeof window === "undefined" ? 900 : window.innerHeight;
  const [position, setPosition] = useState<ViewportPoint>(() => (
    computeContextMenuPosition(x, y, fallbackWidth, fallbackHeight, viewportWidth, viewportHeight)
  ));

  useIsomorphicLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const update = () => {
      const next = computeContextMenuPosition(x, y, element.offsetWidth || fallbackWidth,
        element.offsetHeight || fallbackHeight, window.innerWidth, window.innerHeight);
      setPosition((current) => (current.left === next.left && current.top === next.top ? current : next));
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    window.addEventListener("resize", update);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [x, y, fallbackWidth, fallbackHeight]);

  return { ref, style: position };
}
