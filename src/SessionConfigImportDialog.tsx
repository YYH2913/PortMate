import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import { AlertTriangle, FileUp, LoaderCircle, Upload, X } from "lucide-react";
import { KeyedRequestGate } from "./keyed-request-gate";

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
  headerAddon,
  operationGate,
  onDraftDirtyChange,
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
  headerAddon?: (busy: boolean) => ReactNode;
  operationGate: KeyedRequestGate<"operation">;
  onDraftDirtyChange: (dirty: boolean) => void;
  onImport: (candidates: C[]) => Promise<SessionConfigImportSaveResult>;
  onClose: () => void;
}) {
  useLocale();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const fileReadGate = useRef(new KeyedRequestGate<"file">());
  const [source, setSource] = useState("");
  const [sourceName, setSourceName] = useState("");
  const [sourceError, setSourceError] = useState("");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const [busy, setBusy] = useState(false);
  const fileReadActive = useRef(false);
  const [fileReadBusy, setFileReadBusy] = useState(false);
  const [resultMessage, setResultMessage] = useState<{ text: string; error: boolean } | null>(null);
  const parsed = useMemo(() => parse(source, sourceName), [parse, source, sourceName]);
  const activeError = sourceError || parsed.error;
  const selectedCandidates = parsed.candidates.filter((candidate) => selectedIds.has(candidate.id));
  const locked = busy || fileReadBusy;

  useEffect(() => {
    setSelectedIds(new Set(parsed.candidates.map((candidate) => candidate.id)));
    setResultMessage(null);
  }, [source, sourceName]);

  useEffect(() => () => fileReadGate.current.invalidateAll(), []);

  function updateSource(value: string, name = "") {
    if (value.length > maxSourceChars) {
      setSourceError(t("configuration-exceeds-the-character-limit", [maxSourceChars.toLocaleString()]));
      return;
    }
    setSource(value);
    setSourceName(name);
    setSourceError("");
    onDraftDirtyChange(Boolean(value || name));
  }

  async function readConfigFile(file: File | null) {
    if (!file) return;
    const token = fileReadGate.current.replace("file");
    fileReadActive.current = true;
    setFileReadBusy(true);
    setResultMessage(null);
    if (file.size > maxSourceChars) {
      setSourceError(t("file-exceeds-the-byte-limit", [maxSourceChars.toLocaleString()]));
      if (fileReadGate.current.finish("file", token)) {
        fileReadActive.current = false;
        setFileReadBusy(false);
      }
      return;
    }
    try {
      const text = await file.text();
      if (fileReadGate.current.isCurrent("file", token)) updateSource(text, file.name);
    } catch (error) {
      if (fileReadGate.current.isCurrent("file", token)) setSourceError(errorMessage(error));
    } finally {
      if (fileReadGate.current.finish("file", token)) {
        fileReadActive.current = false;
        setFileReadBusy(false);
      }
    }
  }

  function cancelFileRead() {
    fileReadGate.current.invalidate("file");
    fileReadActive.current = false;
    setFileReadBusy(false);
  }

  function toggleCandidate(id: string, selected: boolean) {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (selected) next.add(id);
      else next.delete(id);
      return next;
    });
    setResultMessage(null);
    onDraftDirtyChange(true);
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (fileReadActive.current || activeError || !selectedCandidates.length) return;
    const token = operationGate.begin("operation");
    if (token === null) return;
    setBusy(true);
    setResultMessage(null);
    try {
      const result = await onImport(selectedCandidates);
      if (!operationGate.isCurrent("operation", token)) return;
      const failedIds = new Set(result.failures.map((failure) => failure.id));
      setSelectedIds(failedIds);
      onDraftDirtyChange(failedIds.size > 0);
      const failures = result.failures.map((failure) => failure.message).filter(Boolean);
      setResultMessage({
        text: failures.length
          ? t("imported-sessions", [result.savedIds.length, failures.join("；")])
          : t("imported-sessions-2", [result.savedIds.length]),
        error: failures.length > 0,
      });
    } catch (error) {
      if (operationGate.isCurrent("operation", token)) {
        setResultMessage({ text: errorMessage(error), error: true });
      }
    } finally {
      if (operationGate.finish("operation", token)) setBusy(false);
    }
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => {
      if (!locked && event.target === event.currentTarget) onClose();
    }}>
      <form className="wind-dialog session-config-import-dialog" role="dialog" aria-modal="true" aria-labelledby="session-config-import-title" onSubmit={(event) => void submit(event)}>
        <header className="dialog-title">
          <FileUp size={17} />
          <strong id="session-config-import-title">{title}</strong>
          {headerAddon?.(locked)}
          <button type="button" title={t("close")} aria-label={t("close-2", [title])} onClick={onClose} disabled={locked}><X size={18} /></button>
        </header>
        <section className="session-config-import-content">
          <div className="session-import-source-header">
            <span>{sourceName || sourceLabel}</span>
            <button type="button" className="session-import-file-button" onClick={() => fileInputRef.current?.click()} disabled={busy}>
              <Upload size={14} />{t("select-file")}</button>
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
            onChange={(event) => {
              cancelFileRead();
              updateSource(event.target.value);
            }}
            disabled={locked}
          />
          <div className="session-import-summary" aria-live="polite">
            <span>{t("sessions", [parsed.candidates.length])}</span>
            {parsed.warnings.length ? <span className="warning"><AlertTriangle size={14} />{parsed.warnings.length}{t("unimported-items")}</span> : null}
          </div>
          <div className="session-import-list" role="list" aria-label={t("preview-2", [title])}>
            {parsed.candidates.map((candidate) => {
              const name = candidateName(candidate);
              const target = candidateTarget(candidate);
              const details = candidateDetails?.(candidate);
              return (
                <label key={candidate.id} className="session-import-row" role="listitem">
                  <input
                    type="checkbox"
                    aria-label={t("import-2", [name])}
                    checked={selectedIds.has(candidate.id)}
                    disabled={locked}
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
              <summary><AlertTriangle size={14} />{t("view-unimported-items")}</summary>
              <ul>{parsed.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>
            </details>
          ) : null}
          {activeError || resultMessage ? <div className={activeError || resultMessage?.error ? "dialog-note error" : "dialog-note"}>{localizeDiagnostic(activeError) || resultMessage?.text}</div> : <div className="dialog-note" />}
        </section>
        <footer className="dialog-footer session-import-footer">
          <span>{selectedCandidates.length ? t("selected", [selectedCandidates.length]) : ""}</span>
          <div className="dialog-actions inline">
            <button type="button" onClick={onClose} disabled={locked}>{t("cancel")}</button>
            <button type="submit" className="primary" disabled={locked || Boolean(activeError) || !selectedCandidates.length}>
              {busy ? <LoaderCircle size={15} className="spin" /> : <FileUp size={15} />}{t("import")}</button>
          </div>
        </footer>
      </form>
    </div>
  );
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
