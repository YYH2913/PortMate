import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import { AlertTriangle, FileUp, LoaderCircle, Upload, X } from "lucide-react";
import {
  OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS,
  parseOpenSshConfig,
} from "./openssh-config-import";
import type { OpenSshImportCandidate } from "./openssh-config-import";

export type OpenSshConfigImportSaveResult = {
  savedIds: string[];
  failures: Array<{ id: string; message: string }>;
};

export default function OpenSshConfigImportDialog({
  onImport,
  onClose,
}: {
  onImport: (candidates: OpenSshImportCandidate[]) => Promise<OpenSshConfigImportSaveResult>;
  onClose: () => void;
}) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [source, setSource] = useState("");
  const [sourceName, setSourceName] = useState("");
  const [sourceError, setSourceError] = useState("");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const [busy, setBusy] = useState(false);
  const [resultMessage, setResultMessage] = useState<{ text: string; error: boolean } | null>(null);
  const parsed = useMemo(() => parseOpenSshConfig(source), [source]);
  const activeError = sourceError || parsed.error;
  const selectedCandidates = parsed.candidates.filter((candidate) => selectedIds.has(candidate.id));

  useEffect(() => {
    setSelectedIds(new Set(parsed.candidates.map((candidate) => candidate.id)));
    setResultMessage(null);
  }, [source]);

  function updateSource(value: string, name = "") {
    if (value.length > OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS) {
      setSourceError(`配置超过 ${OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS.toLocaleString()} 字符限制`);
      return;
    }
    setSource(value);
    setSourceName(name);
    setSourceError("");
  }

  async function readConfigFile(file: File | null) {
    if (!file) return;
    if (file.size > OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS) {
      setSourceError(`文件超过 ${OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS.toLocaleString()} 字节限制`);
      return;
    }
    try {
      updateSource(await file.text(), file.name);
    } catch (error) {
      setSourceError(errorMessage(error));
    }
  }

  function toggleCandidate(id: string, selected: boolean) {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (selected) next.add(id);
      else next.delete(id);
      return next;
    });
    setResultMessage(null);
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || activeError || !selectedCandidates.length) return;
    setBusy(true);
    setResultMessage(null);
    try {
      const result = await onImport(selectedCandidates);
      const failedIds = new Set(result.failures.map((failure) => failure.id));
      setSelectedIds(failedIds);
      const failures = result.failures.map((failure) => failure.message).filter(Boolean);
      setResultMessage({
        text: failures.length
          ? `已导入 ${result.savedIds.length} 个会话；${failures.join("；")}`
          : `已导入 ${result.savedIds.length} 个会话`,
        error: failures.length > 0,
      });
    } catch (error) {
      setResultMessage({ text: errorMessage(error), error: true });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => {
      if (!busy && event.target === event.currentTarget) onClose();
    }}>
      <form className="wind-dialog openssh-config-import-dialog" role="dialog" aria-modal="true" aria-labelledby="openssh-config-import-title" onSubmit={(event) => void submit(event)}>
        <header className="dialog-title">
          <FileUp size={17} />
          <strong id="openssh-config-import-title">导入 OpenSSH 会话</strong>
          <button type="button" title="关闭" aria-label="关闭 OpenSSH 会话导入" onClick={onClose} disabled={busy}><X size={18} /></button>
        </header>
        <section className="openssh-config-import-content">
          <div className="openssh-import-source-header">
            <span>{sourceName || "OpenSSH config"}</span>
            <button type="button" className="openssh-import-file-button" onClick={() => fileInputRef.current?.click()} disabled={busy}>
              <Upload size={14} />
              选择文件
            </button>
            <input
              ref={fileInputRef}
              className="openssh-import-file-input"
              type="file"
              onChange={(event) => {
                void readConfigFile(event.currentTarget.files?.[0] ?? null);
                event.currentTarget.value = "";
              }}
            />
          </div>
          <textarea
            aria-label="OpenSSH 配置内容"
            value={source}
            spellCheck={false}
            placeholder="Host name"
            onChange={(event) => updateSource(event.target.value)}
            disabled={busy}
          />
          <div className="openssh-import-summary" aria-live="polite">
            <span>{parsed.candidates.length} 个会话</span>
            {parsed.warnings.length ? <span className="warning"><AlertTriangle size={14} />{parsed.warnings.length} 个未导入项</span> : null}
          </div>
          <div className="openssh-import-list" role="list" aria-label="OpenSSH 导入预览">
            {parsed.candidates.map((candidate) => (
              <label key={candidate.id} className="openssh-import-row" role="listitem">
                <input
                  type="checkbox"
                  aria-label={`导入 ${candidate.hostAlias}`}
                  checked={selectedIds.has(candidate.id)}
                  disabled={busy}
                  onChange={(event) => toggleCandidate(candidate.id, event.target.checked)}
                />
                <span className="openssh-import-target">
                  <strong>{candidate.hostAlias}</strong>
                  <code title={formatEndpoint(candidate)}>{formatEndpoint(candidate)}</code>
                </span>
                <span className="openssh-import-details">
                  {candidate.identityFiles.length ? `${candidate.identityFiles.length} 个密钥` : ""}
                  {candidate.jumps.length ? `${candidate.jumps.length} 个跳板` : ""}
                  {candidate.warnings.length ? <span title={candidate.warnings.join("\n")}><AlertTriangle size={14} /></span> : null}
                </span>
              </label>
            ))}
            {source.trim() && !parsed.candidates.length && !activeError ? <div className="empty-pane top">没有可导入的字面 Host 条目</div> : null}
          </div>
          {parsed.warnings.length ? (
            <details className="openssh-import-warnings">
              <summary><AlertTriangle size={14} />查看未导入项</summary>
              <ul>{parsed.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>
            </details>
          ) : null}
          {activeError || resultMessage ? <div className={activeError || resultMessage?.error ? "dialog-note error" : "dialog-note"}>{activeError || resultMessage?.text}</div> : <div className="dialog-note" />}
        </section>
        <footer className="dialog-footer openssh-import-footer">
          <span>{selectedCandidates.length ? `已选择 ${selectedCandidates.length} 个` : ""}</span>
          <div className="dialog-actions inline">
            <button type="button" onClick={onClose} disabled={busy}>取消</button>
            <button type="submit" className="primary" disabled={busy || Boolean(activeError) || !selectedCandidates.length}>
              {busy ? <LoaderCircle size={15} className="spin" /> : <FileUp size={15} />}
              导入
            </button>
          </div>
        </footer>
      </form>
    </div>
  );
}

function formatEndpoint(candidate: OpenSshImportCandidate) {
  const host = candidate.host.includes(":") ? `[${candidate.host}]` : candidate.host;
  return candidate.username ? `${candidate.username}@${host}:${candidate.port}` : `${host}:${candidate.port}`;
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
