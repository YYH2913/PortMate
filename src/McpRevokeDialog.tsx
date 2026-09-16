import { t, tr, useLocale, localizeDiagnostic } from "./i18n";
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
  useLocale();
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
        <header><div><strong id="mcp-revoke-title">{t("confirm-grant-revocation")}</strong><span>{t("second-confirmation-irreversible")}</span></div><button type="button" aria-label={t("cancel-revocation")} disabled={busy} onClick={onCancel}><X size={17} /></button></header>
        <p id="mcp-revoke-description">{tr("revoke-client-confirmation", [<strong>{target.name || target.clientId}</strong>])}</p>
        <code className="mcp-revoke-client-id">{target.clientId}</code>
        <label className="mcp-revoke-input"><span>{t("enter-client-id")}</span><input ref={input} value={value} autoComplete="off" spellCheck={false} disabled={busy} onChange={event => setValue(event.target.value)} /></label>
        {httpBound ? <div className="mcp-revoke-warning">{t("this-identity-is-bound-to-http-bridge-revoking-it")}</div> : null}
        {error ? <div className="utility-error" role="alert">{localizeDiagnostic(error)}</div> : null}
        <footer><button type="button" disabled={busy} onClick={onCancel}>{t("cancel")}</button><button type="submit" className="danger" disabled={busy || value !== target.clientId}>{busy ? t("revoking") : t("confirm-revocation")}</button></footer>
      </form>
    </div>
  );
}
