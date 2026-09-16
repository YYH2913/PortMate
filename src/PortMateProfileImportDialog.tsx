import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { FileUp, Upload, X } from "lucide-react";
import { KeyedRequestGate } from "./keyed-request-gate";
import {
  parsePortMateProfileTransfer,
  profileTransferWarningLabel,
} from "./portmate-profile-transfer";
import type { ProfileTransferWarning } from "./portmate-profile-transfer";
import type { SessionProfile } from "./types";
import type { SessionConfigImportSaveResult } from "./SessionConfigImportDialog";

const MAX_PROFILE_TRANSFER_BYTES = 8 * 1024 * 1024;

export default function PortMateProfileImportDialog({
  onImport,
  onClose,
  headerAddon,
  operationGate,
  onDraftDirtyChange,
}: {
  onImport: (profiles: SessionProfile[]) => Promise<SessionConfigImportSaveResult>;
  onClose: () => void;
  headerAddon?: (busy: boolean) => ReactNode;
  operationGate: KeyedRequestGate<"operation">;
  onDraftDirtyChange: (dirty: boolean) => void;
}) {
  useLocale();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const fileGate = useRef(new KeyedRequestGate<"file">());
  const [source, setSource] = useState("");
  const [fileName, setFileName] = useState("");
  const [profiles, setProfiles] = useState<SessionProfile[]>([]);
  const [warnings, setWarnings] = useState<ProfileTransferWarning[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => () => fileGate.current.invalidateAll(), []);

  function updateSource(value: string, name = "") {
    setSource(value);
    setFileName(name);
    setError("");
    try {
      const parsed = parsePortMateProfileTransfer(value);
      setProfiles(parsed.profiles);
      setWarnings(parsed.warnings);
    } catch (parseError) {
      setProfiles([]);
      setWarnings([]);
      if (value.trim()) setError(parseError instanceof Error ? parseError.message : String(parseError));
    }
    onDraftDirtyChange(Boolean(value || name));
  }

  async function readFile(file: File | null) {
    if (!file) return;
    if (file.size > MAX_PROFILE_TRANSFER_BYTES) {
      setError(t("profile-file-exceeds-the-8-mib-limit"));
      return;
    }
    const token = fileGate.current.replace("file");
    try {
      const text = await file.text();
      if (fileGate.current.isCurrent("file", token)) updateSource(text, file.name);
    } catch (readError) {
      if (fileGate.current.isCurrent("file", token)) setError(readError instanceof Error ? readError.message : String(readError));
    } finally {
      fileGate.current.finish("file", token);
    }
  }

  async function importProfiles() {
    if (!profiles.length || busy) return;
    const token = operationGate.begin("operation");
    if (token === null) return;
    setBusy(true);
    try {
      const result = await onImport(profiles);
      if (result.failures.length) {
        setError(result.failures.map((failure) => failure.message).join("；"));
        if (result.savedIds.length) onDraftDirtyChange(false);
        return;
      }
      onDraftDirtyChange(false);
      onClose();
    } catch (importError) {
      setError(importError instanceof Error ? importError.message : String(importError));
    } finally {
      if (operationGate.finish("operation", token)) setBusy(false);
    }
  }

  return (
    <div className="dialog-backdrop">
      <section className="wind-dialog session-import-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{t("import-portmate-profiles")}</strong>
          <button type="button" onClick={onClose} disabled={busy} aria-label={t("close")}><X size={20} /></button>
        </header>
        {headerAddon?.(busy)}
        <section className="session-import-content">
          <p className="session-identity-hint"><strong>{t("portable-configuration-without-plaintext-credentials")}</strong><span>{t("passwords-private-keys-and-proxy-secrets-are-excluded-from")}</span></p>
          <div className="session-import-file-row">
            <button type="button" onClick={() => fileInputRef.current?.click()} disabled={busy}><FileUp size={15} />{t("select-profile-file")}</button>
            <input ref={fileInputRef} type="file" accept=".json,.portmate.json" hidden onChange={(event) => { void readFile(event.currentTarget.files?.[0] ?? null); event.currentTarget.value = ""; }} />
            <span>{fileName || t("no-file-selected")}</span>
          </div>
          <textarea aria-label={t("ui-portmate-profile-json")} value={source} onChange={(event) => updateSource(event.target.value)} placeholder={t("or-paste-portmate-profile-json")} disabled={busy} />
          {error ? <div className="utility-error" role="alert">{localizeDiagnostic(error)}</div> : null}
          {warnings.length ? <div className="session-import-warnings">{warnings.map((warning) => <div key={JSON.stringify([warning.code, warning.profileName])}>{profileTransferWarningLabel(warning)}</div>)}</div> : null}
          {profiles.length ? <div className="session-import-preview">{t("importing-profiles", [profiles.length, profiles.map((profile) => profile.name).join("、")])}</div> : null}
        </section>
        <footer className="dialog-actions session-settings-actions">
          <button type="button" onClick={onClose} disabled={busy}>{t("cancel")}</button>
          <button type="button" className="session-connect-button" onClick={() => void importProfiles()} disabled={busy || !profiles.length}><Upload size={15} />{t("import-profiles")}</button>
        </footer>
      </section>
    </div>
  );
}
