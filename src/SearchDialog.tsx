import { t, useLocale } from "./i18n";
import { useEffect, useMemo, useState } from "react";
import { Search, X } from "lucide-react";
import { buildSessionSearchResults } from "./session-search-state";
import type { SessionEvent, SessionSummary } from "./types";

export type SearchDialogState = { mode: "sessions" | "logs"; query: string };

export default function SearchDialog({
  state,
  sessions,
  logs,
  onChange,
  onSelect,
  onClose,
}: {
  state: SearchDialogState;
  sessions: SessionSummary[];
  logs: Record<string, SessionEvent[]>;
  onChange: (state: SearchDialogState) => void;
  onSelect: (sessionId: string) => void;
  onClose: () => void;
}) {
  useLocale();
  const results = useMemo(
    () => buildSessionSearchResults(state.mode, state.query, sessions, logs),
    [logs, sessions, state.mode, state.query],
  );
  const [selectedIndex, setSelectedIndex] = useState(0);

  useEffect(() => setSelectedIndex(0), [state.mode, state.query]);
  useEffect(() => {
    setSelectedIndex((current) => Math.min(current, Math.max(0, results.length - 1)));
  }, [results.length]);
  useEffect(() => {
    document.getElementById(`search-result-${selectedIndex}`)?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  function selectResult(index: number) {
    const result = results[index];
    if (result) onSelect(result.sessionId);
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog search-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{state.mode === "sessions" ? t("session-search") : t("log-search")}</strong>
          <button onClick={onClose}><X size={20} /></button>
        </header>
        <div className="search-content">
          <div className="search-tabs" role="tablist" aria-label={t("search-scope")}>
            <button role="tab" aria-selected={state.mode === "sessions"} className={state.mode === "sessions" ? "active" : ""} onClick={() => onChange({ ...state, mode: "sessions" })}>{t("session")}</button>
            <button role="tab" aria-selected={state.mode === "logs"} className={state.mode === "logs" ? "active" : ""} onClick={() => onChange({ ...state, mode: "logs" })}>{t("logs")}</button>
          </div>
          <label className="search-query">
            <Search size={14} aria-hidden="true" />
            <input
              autoFocus
              role="combobox"
              aria-label={t("search-sessions-and-logs")}
              aria-controls="search-results"
              aria-expanded="true"
              aria-activedescendant={results.length ? `search-result-${selectedIndex}` : undefined}
              value={state.query}
              onChange={(event) => onChange({ ...state, query: event.target.value })}
              onKeyDown={(event) => {
                if (event.key === "ArrowDown") {
                  event.preventDefault();
                  setSelectedIndex((current) => results.length ? Math.min(results.length - 1, current + 1) : 0);
                } else if (event.key === "ArrowUp") {
                  event.preventDefault();
                  setSelectedIndex((current) => results.length ? Math.max(0, current - 1) : 0);
                } else if (event.key === "Enter") {
                  event.preventDefault();
                  selectResult(selectedIndex);
                } else if (event.key === "Escape") {
                  event.preventDefault();
                  onClose();
                }
              }}
              placeholder={t("name-tag-status-or-endpoint")}
            />
          </label>
          <div id="search-results" className="search-results" role="listbox" aria-label={t("search-results")}>
            {results.map((result, index) => (
              <button
                id={`search-result-${index}`}
                key={result.key}
                type="button"
                role="option"
                aria-selected={index === selectedIndex}
                className={index === selectedIndex ? "selected" : ""}
                onMouseEnter={() => setSelectedIndex(index)}
                onClick={() => selectResult(index)}
              >
                <strong>{result.title}</strong>
                <span>{result.detail}</span>
              </button>
            ))}
            {!results.length ? <div className="empty-pane top">{t("no-matching-results")}</div> : null}
          </div>
        </div>
      </section>
    </div>
  );
}
