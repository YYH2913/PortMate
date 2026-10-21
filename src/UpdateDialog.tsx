import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { Download, ExternalLink, RefreshCw, X } from "lucide-react";
import packageJson from "../package.json";
import { invokeBackend, isBackendAvailable } from "./api";
import { t, useLocale } from "./i18n";
import { formatBytes } from "./display-formatters";
import { normalizeTerminalWebLink, openIsolatedWebLink } from "./terminal-web-link";
import ReleaseNotes from "./ReleaseNotes";

export type UpdateCheckResult = {
  currentVersion: string;
  version: string;
  releaseUrl: string;
  notes: string;
  publishedAt: string | null;
  available: boolean;
  candidateId: string | null;
  manualReason: "unsupported-package" | "unsigned-release" | null;
};
type UpdateProgress = { phase: "downloading" | "verifying" | "preparing" | "installing"; downloaded: number; total: number | null };
type Phase = "checking" | "ready" | "error" | UpdateProgress["phase"];

function updateError(error: unknown): string {
  if (error && typeof error === "object" && "code" in error) {
    const { code, detail } = error as { code: string; detail?: string };
    const key = ({ busy: "update-busy", expired: "update-expired", unavailable: "update-desktop-only", "invalid-release": "update-invalid-release", "check-failed": "update-check-failed", "download-failed": "update-download-failed", "prepare-failed": "update-prepare-failed", "install-failed": "update-install-failed" } as Record<string, string>)[code];
    if (key) return `${t(key)}${detail ? `\n${detail}` : ""}`;
  }
  return `${t("update-check-failed")}\n${String(error)}`;
}

export default function UpdateDialog({ onClose }: { onClose: () => void }) {
  useLocale();
  const [result, setResult] = useState<UpdateCheckResult | null>(null);
  const [phase, setPhase] = useState<Phase>("checking");
  const [error, setError] = useState("");
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const pending = useRef(false);
  const mounted = useRef(true);
  const closeRef = useRef<HTMLButtonElement>(null);
  const busy = phase === "checking" || ["downloading", "verifying", "preparing", "installing"].includes(phase);

  async function check() {
    if (pending.current) return;
    if (!isBackendAvailable()) { setError(t("update-desktop-only")); setPhase("error"); return; }
    pending.current = true;
    setPhase("checking"); setError(""); setResult(null); setProgress(null);
    try {
      const next = await invokeBackend<UpdateCheckResult>("check_for_updates", {});
      if (mounted.current) { setResult(next); setPhase("ready"); }
    } catch (reason) {
      if (mounted.current) { setError(updateError(reason)); setPhase("error"); }
    } finally { pending.current = false; }
  }

  useEffect(() => {
    mounted.current = true;
    closeRef.current?.focus({ preventScroll: true });
    void check();
    return () => { mounted.current = false; };
  }, []);

  async function install() {
    if (pending.current || !result?.candidateId || !result.available) return;
    if (!window.confirm(t("update-confirm", [result.currentVersion, result.version]))) return;
    pending.current = true; setError(""); setPhase("downloading");
    const onProgress = new Channel<UpdateProgress>();
    onProgress.onmessage = (next) => {
      if (mounted.current && pending.current) { setProgress(next); setPhase(next.phase); }
    };
    try {
      await invokeBackend<void>("install_update", { candidateId: result.candidateId, onProgress });
      // The native command requests restart on success. Keep the dialog locked
      // until the old process exits instead of offering a duplicate install.
    } catch (reason) {
      if (mounted.current) { setError(updateError(reason)); setResult(null); setPhase("error"); }
      pending.current = false;
    }
  }

  function openRelease() {
    const link = normalizeTerminalWebLink(result?.releaseUrl ?? "https://github.com/YYH2913/PortMate/releases");
    if (link) openIsolatedWebLink(link);
  }

  return (
    <div className="dialog-backdrop update-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget && !busy) onClose();
    }} onKeyDown={(event) => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (!busy) onClose(); }
      if (event.key === "Tab") {
        const controls = [...event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), a[href]:not([aria-disabled="true"]), summary')]
          .filter((control) => {
            const collapsed = control.closest("details:not([open])");
            return control.getClientRects().length > 0 && (!collapsed || collapsed.querySelector("summary") === control);
          });
        if (!controls.length) { event.preventDefault(); return; }
        const index = controls.indexOf(document.activeElement as HTMLElement);
        event.preventDefault();
        const next = index < 0 ? (event.shiftKey ? controls.length - 1 : 0)
          : (index + (event.shiftKey ? controls.length - 1 : 1)) % controls.length;
        controls[next]?.focus();
      }
    }}>
      <section className="wind-dialog update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-title" aria-busy={busy}>
        <header className="dialog-title">
          <span className="app-icon" aria-hidden="true" /><strong id="update-title">{t("check-for-updates")}</strong>
          <button type="button" ref={closeRef} title={t("close")} aria-label={t("close")} disabled={busy} onClick={onClose}><X size={20} /></button>
        </header>
        <div className="update-content">
          <p className="update-current">PortMate <code>v{result?.currentVersion ?? packageJson.version}</code></p>
          <div role="status" aria-live="polite">
            {phase === "checking" && <p>{t("update-checking")}</p>}
            {result && phase === "ready" && <p className="update-status">{result.available ? t("update-available", [result.version]) : t("update-up-to-date")}</p>}
            {phase === "downloading" && <>
              <p>{t("update-downloading")}</p>
              <progress aria-label={t("update-download-progress")} max={progress?.total || undefined} value={progress?.total ? progress.downloaded : undefined} />
              <p>{progress ? `${formatBytes(progress.downloaded)}${progress.total ? ` / ${formatBytes(progress.total)}` : ""}` : ""}</p>
            </>}
            {phase === "verifying" && <p>{t("update-verifying")}</p>}
            {phase === "preparing" && <p>{t("update-preparing")}</p>}
            {phase === "installing" && <p>{t("update-installing")}</p>}
          </div>
          {result?.manualReason && <p className="update-manual">{t(result.manualReason === "unsupported-package" ? "update-manual-package" : "update-manual-unsigned")}</p>}
          {result?.available && result.candidateId && <p>{t("update-exit-note")}</p>}
          {result?.available && result.notes && <details className="update-notes" open>
            <summary>{t("update-release-notes")}</summary>
            <ReleaseNotes notes={result.notes} releaseUrl={result.releaseUrl} disabled={busy} />
          </details>}
          {error && <p className="update-error" role="alert">{error}</p>}
        </div>
        <footer className="update-actions">
          <button type="button" disabled={busy} onClick={openRelease}><ExternalLink size={15} />{t("releases")}</button>
          <span />
          {result?.available && result.candidateId ? <button type="button" className="primary" disabled={busy} onClick={() => void install()}><Download size={15} />{t("update-install")}</button>
            : <button type="button" disabled={busy} onClick={() => void check()}><RefreshCw size={15} />{t("check-for-updates")}</button>}
          <button type="button" disabled={busy} onClick={onClose}>{t("close")}</button>
        </footer>
      </section>
    </div>
  );
}
