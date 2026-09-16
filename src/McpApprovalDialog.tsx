import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { Check, Clock3, ShieldAlert, X } from "lucide-react";
import type { McpApprovalRequest } from "./types";

const actionLabels: Record<string, string> = {
  send_text: "send-terminal-text",
  send_bytes: "send-raw-bytes",
  send_key: "send-terminal-key",
  serial_send_break: "send-serial-break",
  run_command: "run-terminal-command",
  run_local_command: "run-local-terminal-command",
  run_custom_script: "run-custom-script",
  attach_tmux: "attach-tmux",
  start_transfer: "start-file-transfer",
  cancel_transfer: "cancel-file-transfer",
  retry_transfer: "retry-file-transfer",
  create_tunnel: "create-specified-forward-or-proxy",
  stop_tunnel: "stop-specified-forward-or-proxy",
  tunnel_request: "send-request-through-tunnel",
  udp_request: "send-udp-datagram-through-tunnel",
};

export default function McpApprovalDialog({
  request,
  sessionName,
  queueCount,
  onDecision,
  onExpired,
}: {
  request: McpApprovalRequest;
  sessionName: string;
  queueCount: number;
  onDecision: (approvalId: string, approved: boolean) => Promise<void>;
  onExpired: (approvalId: string) => void;
}) {
  useLocale();
  const dialogRef = useRef<HTMLElement>(null);
  const rejectRef = useRef<HTMLButtonElement>(null);
  const expiredRef = useRef(false);
  const decisionRequestRef = useRef<string | null>(null);
  const [now, setNow] = useState(Date.now());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const createdAt = Date.parse(request.createdAt);
  const expiresAt = Date.parse(request.expiresAt);
  const remainingMs = Math.max(0, expiresAt - now);
  const totalMs = Math.max(1, expiresAt - createdAt);
  const remainingSeconds = Math.ceil(remainingMs / 1000);

  useEffect(() => {
    expiredRef.current = false;
    decisionRequestRef.current = null;
    setBusy(false);
    setError("");
    setNow(Date.now());
    window.requestAnimationFrame(() => rejectRef.current?.focus({ preventScroll: true }));
    const timer = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, [request.id]);

  useEffect(() => {
    if (remainingMs > 0 || expiredRef.current || busy || decisionRequestRef.current === request.id) return;
    expiredRef.current = true;
    onExpired(request.id);
  }, [busy, onExpired, remainingMs, request.id]);

  async function decide(approved: boolean) {
    if (decisionRequestRef.current !== null || remainingMs <= 0) return;
    decisionRequestRef.current = request.id;
    setBusy(true);
    setError("");
    try {
      await onDecision(request.id, approved);
    } catch (nextError) {
      if (decisionRequestRef.current !== request.id) return;
      decisionRequestRef.current = null;
      setBusy(false);
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    }
  }

  function trapFocus(event: React.KeyboardEvent<HTMLElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      void decide(false);
      return;
    }
    if (event.key !== "Tab") return;
    const controls = [...(dialogRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    if (!controls.length) {
      event.preventDefault();
      return;
    }
    const currentIndex = controls.indexOf(document.activeElement as HTMLButtonElement);
    const nextIndex = event.shiftKey
      ? (currentIndex <= 0 ? controls.length - 1 : currentIndex - 1)
      : (currentIndex < 0 || currentIndex === controls.length - 1 ? 0 : currentIndex + 1);
    event.preventDefault();
    controls[nextIndex].focus();
  }

  return (
    <div className="mcp-approval-backdrop">
      <section
        ref={dialogRef}
        className="mcp-approval-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="mcp-approval-title"
        aria-describedby="mcp-approval-status"
        onKeyDown={trapFocus}
      >
        <header>
          <span className="mcp-approval-icon"><ShieldAlert size={19} /></span>
          <div>
            <strong id="mcp-approval-title">{t("mcp-write-approval")}</strong>
            <span>{queueCount > 1 ? t("pending", [queueCount]) : t("confirmation-required")}</span>
          </div>
        </header>
        <dl>
          <div><dt>{t("ui-client")}</dt><dd><code>{request.clientId}</code></dd></div>
          <div><dt>{t("action")}</dt><dd>{t(actionLabels[request.action] ?? request.action)}</dd></div>
          <div><dt>{request.sessionId === "portmate-host" ? t("execution-host") : t("session")}</dt><dd><span>{request.sessionId === "portmate-host" ? t("portmate-host") : sessionName}</span><code>{request.sessionId}</code></dd></div>
          {request.target ? <div><dt>{t("target")}</dt><dd><span>{request.target.label}</span><code>{request.target.id}</code></dd></div> : null}
          <div><dt>{t("ui-scope-2")}</dt><dd><code>{request.scope}</code></dd></div>
        </dl>
        <div className="mcp-approval-timer" id="mcp-approval-status" role="status">
          <span style={{ width: `${Math.min(100, Math.max(0, remainingMs / totalMs * 100))}%` }} />
          <div><Clock3 size={13} /><span>{t("automatically-rejected-in-seconds", [remainingSeconds])}</span></div>
        </div>
        {error ? <div className="mcp-approval-error">{localizeDiagnostic(error)}</div> : null}
        <footer>
          <button ref={rejectRef} type="button" className="reject" disabled={busy} onClick={() => void decide(false)}><X size={15} />{t("reject")}</button>
          <button type="button" className="approve" disabled={busy} onClick={() => void decide(true)}><Check size={15} />{t("allow-once")}</button>
        </footer>
      </section>
    </div>
  );
}
