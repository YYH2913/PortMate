import { useEffect, useRef, useState } from "react";
import { Check, Clock3, ShieldAlert, X } from "lucide-react";
import type { McpApprovalRequest } from "./types";

const actionLabels: Record<string, string> = {
  send_text: "发送终端文本",
  send_bytes: "透传原始字节",
  send_key: "发送终端按键",
  serial_send_break: "发送串口 Break",
  run_command: "执行终端命令",
  run_custom_script: "运行自定义脚本",
  attach_tmux: "连接 Tmux",
  open_session: "打开会话",
  close_session: "断开会话",
  start_transfer: "启动文件传输",
  cancel_transfer: "取消文件传输",
  retry_transfer: "重试文件传输",
  create_tunnel: "创建指定转发或代理",
  stop_tunnel: "停止指定转发或代理",
  create_host_route: "创建 PortMate 主机路由",
  stop_host_route: "停止 PortMate 主机路由",
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
            <strong id="mcp-approval-title">MCP 写操作审批</strong>
            <span>{queueCount > 1 ? `待处理 ${queueCount} 项` : "需要本次确认"}</span>
          </div>
        </header>
        <dl>
          <div><dt>Client</dt><dd><code>{request.clientId}</code></dd></div>
          <div><dt>操作</dt><dd>{actionLabels[request.action] ?? request.action}</dd></div>
          <div><dt>会话</dt><dd><span>{sessionName}</span><code>{request.sessionId}</code></dd></div>
          {request.target ? <div><dt>目标</dt><dd><span>{request.target.label}</span><code>{request.target.id}</code></dd></div> : null}
          <div><dt>Scope</dt><dd><code>{request.scope}</code></dd></div>
        </dl>
        <div className="mcp-approval-timer" id="mcp-approval-status" role="status">
          <span style={{ width: `${Math.min(100, Math.max(0, remainingMs / totalMs * 100))}%` }} />
          <div><Clock3 size={13} /><span>{remainingSeconds} 秒后自动拒绝</span></div>
        </div>
        {error ? <div className="mcp-approval-error">{error}</div> : null}
        <footer>
          <button ref={rejectRef} type="button" className="reject" disabled={busy} onClick={() => void decide(false)}><X size={15} />拒绝</button>
          <button type="button" className="approve" disabled={busy} onClick={() => void decide(true)}><Check size={15} />本次允许</button>
        </footer>
      </section>
    </div>
  );
}
