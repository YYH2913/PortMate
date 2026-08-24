import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { FileUp, Upload, X } from "lucide-react";
import { KeyedRequestGate } from "./keyed-request-gate";
import {
  parsePortMateProfileTransfer,
} from "./portmate-profile-transfer";
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
  const fileInputRef = useRef<HTMLInputElement>(null);
  const fileGate = useRef(new KeyedRequestGate<"file">());
  const [source, setSource] = useState("");
  const [fileName, setFileName] = useState("");
  const [profiles, setProfiles] = useState<SessionProfile[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
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
      setError("Profile 文件超过 8 MiB 限制");
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
          <strong>导入 PortMate Profile</strong>
          <button type="button" onClick={onClose} disabled={busy} aria-label="关闭"><X size={20} /></button>
        </header>
        {headerAddon?.(busy)}
        <section className="session-import-content">
          <p className="session-identity-hint"><strong>可迁移配置，不包含明文凭据</strong><span>密码、私钥和代理 Secret 不会写入导出文件；导入后可在连接弹窗中一键保存到本机 Stronghold。</span></p>
          <div className="session-import-file-row">
            <button type="button" onClick={() => fileInputRef.current?.click()} disabled={busy}><FileUp size={15} />选择 Profile 文件</button>
            <input ref={fileInputRef} type="file" accept=".json,.portmate.json" hidden onChange={(event) => { void readFile(event.currentTarget.files?.[0] ?? null); event.currentTarget.value = ""; }} />
            <span>{fileName || "未选择文件"}</span>
          </div>
          <textarea aria-label="PortMate Profile JSON" value={source} onChange={(event) => updateSource(event.target.value)} placeholder="也可以粘贴 PortMate Profile JSON" disabled={busy} />
          {error ? <div className="utility-error" role="alert">{error}</div> : null}
          {warnings.length ? <div className="session-import-warnings">{warnings.map((warning) => <div key={warning}>{warning}</div>)}</div> : null}
          {profiles.length ? <div className="session-import-preview">将导入 {profiles.length} 个 Profile：{profiles.map((profile) => profile.name).join("、")}</div> : null}
        </section>
        <footer className="dialog-actions session-settings-actions">
          <button type="button" onClick={onClose} disabled={busy}>取消</button>
          <button type="button" className="session-connect-button" onClick={() => void importProfiles()} disabled={busy || !profiles.length}><Upload size={15} />导入 Profile</button>
        </footer>
      </section>
    </div>
  );
}
