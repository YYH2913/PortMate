import { useCallback, useEffect, useRef, useState } from "react";

export type ClipboardFeedbackStatus = "copied" | "error";

export type ClipboardFeedback = {
  id: string;
  status: ClipboardFeedbackStatus;
};

export function useClipboardFeedback() {
  const [feedback, setFeedback] = useState<ClipboardFeedback | null>(null);
  const [busy, setBusy] = useState(false);
  const request = useRef(0);
  const inFlight = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => () => {
    request.current += 1;
    inFlight.current = false;
    if (timer.current !== undefined) {
      globalThis.clearTimeout(timer.current);
      timer.current = undefined;
    }
  }, []);

  const copy = useCallback(async (id: string, text: string) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    const revision = ++request.current;
    if (timer.current !== undefined) {
      globalThis.clearTimeout(timer.current);
      timer.current = undefined;
    }
    setFeedback(null);

    let status: ClipboardFeedbackStatus = "copied";
    try {
      const clipboard = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
      if (!clipboard?.writeText) {
        status = "error";
      } else {
        // Call writeText synchronously within the initiating click path to preserve user activation.
        await clipboard.writeText(text);
      }
    } catch {
      status = "error";
    } finally {
      inFlight.current = false;
      if (request.current === revision) setBusy(false);
    }

    if (request.current !== revision) return;

    setFeedback({ id, status });
    timer.current = globalThis.setTimeout(() => {
      if (request.current === revision) setFeedback(null);
    }, status === "error" ? 4500 : 1800);
  }, []);

  return { feedback, busy, copy };
}
