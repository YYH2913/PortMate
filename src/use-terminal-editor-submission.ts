import { useEffect, useRef, useState } from "react";

/** One acknowledged editor submission per view/connection, independent of xterm. */
export function useTerminalEditorSubmission(connectionKey: string) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const currentKey = useRef(connectionKey);
  currentKey.current = connectionKey;
  const request = useRef<AbortController | null>(null);

  useEffect(() => {
    setBusy(false);
    setError("");
    return () => { request.current?.abort(); request.current = null; };
  }, [connectionKey]);

  async function submit(send: (signal: AbortSignal) => void | Promise<void>, onSuccess: () => void) {
    if (request.current) return;
    const controller = new AbortController();
    request.current = controller;
    setBusy(true);
    setError("");
    const current = () => request.current === controller && currentKey.current === connectionKey && !controller.signal.aborted;
    try {
      await send(controller.signal);
      if (current()) onSuccess();
    } catch (cause) {
      if (current()) setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (current()) {
        request.current = null;
        setBusy(false);
      }
    }
  }

  return { busy, error, submit, isBusy: () => request.current !== null, clearError: () => setError("") };
}
