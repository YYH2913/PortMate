import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import { AlertTriangle, FileUp, LoaderCircle, Upload, X } from "lucide-react";

export type SessionConfigImportCandidate = {
  id: string;
  warnings: string[];
};

export type SessionConfigImportResult<C extends SessionConfigImportCandidate> = {
  candidates: C[];
  warnings: string[];
  error: string | null;
};

export type SessionConfigImportSaveResult = {
  savedIds: string[];
  failures: Array<{ id: string; message: string }>;
};

export default function SessionConfigImportDialog<C extends SessionConfigImportCandidate>({
  title,
  sourceLabel,
  sourceAriaLabel,
  sourcePlaceholder,
  emptyMessage,
  maxSourceChars,
  parse,
  candidateName,
  candidateTarget,
  candidateDetails,
  onImport,
  onClose,
}: {
  title: string;
  sourceLabel: string;
  sourceAriaLabel: string;
  sourcePlaceholder: string;
  emptyMessage: string;
  maxSourceChars: number;
  parse: (source: string, sourceName: string) => SessionConfigImportResult<C>;
  candidateName: (candidate: C) => string;
  candidateTarget: (candidate: C) => string;
  candidateDetails?: (candidate: C) => ReactNode;
  onImport: (candidates: C[]) => Promise<SessionConfigImportSaveResult>;
  onClose: () => void;
}) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [source, setSource] = useState("");
  const [sourceName, setSourceName] = useState("");
  const [sourceError, setSourceError] = useState("");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const [busy, setBusy] = useState(false);
  const [resultMessage, setResultMessage] = useState<{ text: string; error: boolean } | null>(null);
  const parsed = useMemo(() => parse(source, sourceName), [parse, source, sourceName]);
  const activeError = sourceError || parsed.error;
  const selectedCandidates = parsed.candidates.filter((candidate) => selectedIds.has(candidate.id));

  useEffect(() => {
    setSelectedIds(new Set(parsed.candidates.map((candidate) => candidate.id)));
    setResultMessage(null);
  }, [source, sourceName]);

  function updateSource(value: string, name = "") {
    if (value.length > maxSourceChars) {
      setSourceError(`配置超过 ${maxSourceChars.toLocaleString()} 字符限制`);
      return;
    }
    setSource(value);
    setSourceName(name);
    setSourceError("");
  }

  async function readConfigFile(file: File | null) {
    if (!file) return;
    if (file.size > maxSourceChars) {
      setSourceError(`文件超过 ${maxSourceChars.toLocaleString()} 字节限制`);
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
      <form className="wind-dialog session-config-import-dialog" role="dialog" aria-modal="true" aria-labelledby="session-config-import-title" onSubmit={(event) => void submit(event)}>
        <header className="dialog-title">
          <FileUp size={17} />
          <strong id="session-config-import-title">{title}</strong>
          <button type="button" title="关闭" aria-label={`关闭${title}`} onClick={onClose} disabled={busy}><X size={18} /></button>
        </header>
        <section className="session-config-import-content">
          <div className="session-import-source-header">
            <span>{sourceName || sourceLabel}</span>
            <button type="button" className="session-import-file-button" onClick={() => fileInputRef.current?.click()} disabled={busy}>
              <Upload size={14} />
              选择文件
            </button>
            <input
              ref={fileInputRef}
              className="session-import-file-input"
              type="file"
              onChange={(event) => {
                void readConfigFile(event.currentTarget.files?.[0] ?? null);
                event.currentTarget.value = "";
              }}
            />
          </div>
          <textarea
            aria-label={sourceAriaLabel}
            value={source}
            spellCheck={false}
            placeholder={sourcePlaceholder}
            onChange={(event) => updateSource(event.target.value)}
            disabled={busy}
          />
          <div className="session-import-summary" aria-live="polite">
            <span>{parsed.candidates.length} 个会话</span>
            {parsed.warnings.length ? <span className="warning"><AlertTriangle size={14} />{parsed.warnings.length} 个未导入项</span> : null}
          </div>
          <div className="session-import-list" role="list" aria-label={`${title}预览`}>
            {parsed.candidates.map((candidate) => {
              const name = candidateName(candidate);
              const target = candidateTarget(candidate);
              const details = candidateDetails?.(candidate);
              return (
                <label key={candidate.id} className="session-import-row" role="listitem">
                  <input
                    type="checkbox"
                    aria-label={`导入 ${name}`}
                    checked={selectedIds.has(candidate.id)}
                    disabled={busy}
                    onChange={(event) => toggleCandidate(candidate.id, event.target.checked)}
                  />
                  <span className="session-import-target">
                    <strong>{name}</strong>
                    <code title={target}>{target}</code>
                  </span>
                  <span className="session-import-details">
                    {details}
                    {candidate.warnings.length ? <span title={candidate.warnings.join("\n")}><AlertTriangle size={14} /></span> : null}
                  </span>
                </label>
              );
            })}
            {source.trim() && !parsed.candidates.length && !activeError ? <div className="empty-pane top">{emptyMessage}</div> : null}
          </div>
          {parsed.warnings.length ? (
            <details className="session-import-warnings">
              <summary><AlertTriangle size={14} />查看未导入项</summary>
              <ul>{parsed.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>
            </details>
          ) : null}
          {activeError || resultMessage ? <div className={activeError || resultMessage?.error ? "dialog-note error" : "dialog-note"}>{activeError || resultMessage?.text}</div> : <div className="dialog-note" />}
        </section>
        <footer className="dialog-footer session-import-footer">
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

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
