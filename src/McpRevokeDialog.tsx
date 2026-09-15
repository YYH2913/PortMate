import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import type { McpGrant } from "./types";

export default function McpRevokeDialog({ target, httpBound, busy, error, onCancel, onConfirm }: {
  target: Pick<McpGrant, "clientId" | "name">;
  httpBound: boolean;
  busy: boolean;
  error: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const [value, setValue] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const frame = requestAnimationFrame(() => input.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, []);
  return (
    <div className="dialog-backdrop mcp-revoke-backdrop">
      <form className="mcp-revoke-dialog" role="alertdialog" aria-modal="true" aria-labelledby="mcp-revoke-title"
        aria-describedby="mcp-revoke-description" aria-busy={busy}
        onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (!busy) onCancel(); } }}
        onSubmit={event => { event.preventDefault(); if (!busy && value === target.clientId) onConfirm(); }}>
        <header><div><strong id="mcp-revoke-title">确认撤销授权</strong><span>二次确认 · 不可撤销</span></div><button type="button" aria-label="取消撤销" disabled={busy} onClick={onCancel}><X size={17} /></button></header>
        <p id="mcp-revoke-description">将撤销 <strong>{target.name || target.clientId}</strong> 的权限。输入完整 Client ID 后确认；取消不会执行任何操作。</p>
        <code className="mcp-revoke-client-id">{target.clientId}</code>
        <label className="mcp-revoke-input"><span>输入 Client ID</span><input ref={input} value={value} autoComplete="off" spellCheck={false} disabled={busy} onChange={event => setValue(event.target.value)} /></label>
        {httpBound ? <div className="mcp-revoke-warning">此身份绑定了 HTTP Bridge：撤销后将停止托管服务、清除旧 Token。不会自动切换到其他授权。</div> : null}
        {error ? <div className="utility-error" role="alert">{error}</div> : null}
        <footer><button type="button" disabled={busy} onClick={onCancel}>取消</button><button type="submit" className="danger" disabled={busy || value !== target.clientId}>{busy ? "正在撤销…" : "确认撤销"}</button></footer>
      </form>
    </div>
  );
}
