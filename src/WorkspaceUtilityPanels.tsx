import { formatUiNumber, t, useLocale } from "./i18n";
import { memo, useCallback, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { ComponentType, FocusEvent as ReactFocusEvent, KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent, RefObject, SVGProps } from "react";
import { Check, ChevronDown, ChevronUp, CircleAlert, Copy, CornerDownLeft, FileInput, Plus, SquareTerminal, Trash2 } from "lucide-react";
import { requestAppConfirm } from "./app-dialog";
import type { CommandHistoryEntry } from "./command-history-state";
import { hasActiveInteractionLayer } from "./modal-interaction-boundary";
import { useClipboardFeedback } from "./use-clipboard-feedback";
import { filterWorkspaceSessions } from "./session-search-state";
import { sessionRuntimeHealthDescription } from "./session-runtime-state";
import type { SessionSummary } from "./types";
import ShellElevationIndicator from "./ShellElevationIndicator";
import { shellElevationRequested } from "./shell-elevation";
import { groupExplorerSessions, normalizeSessionExplorerGrouping, readSessionExplorerGrouping, sessionExplorerTypeLabel, subscribeSessionExplorerGrouping, writeSessionExplorerGrouping } from "./session-explorer-grouping";
import type { SessionExplorerGroup } from "./session-explorer-grouping";

type UtilityIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: string | number }>;
type UtilityPanelIcons = {
  Folder: UtilityIcon;
  Search: UtilityIcon;
  X: UtilityIcon;
};

type UtilityPanelFilter = "explorer" | "history";

const EXPLORER_COLLAPSED_GROUPS_KEY = "portmate.explorerCollapsedGroups.v1";

const utilityPanelFilterCache: Record<UtilityPanelFilter, string> = {
  explorer: "",
  history: "",
};

function useUtilityPanelFilter(panel: UtilityPanelFilter) {
  const [query, setQueryState] = useState(() => utilityPanelFilterCache[panel]);
  const setQuery = useCallback((value: string) => {
    utilityPanelFilterCache[panel] = value;
    setQueryState(value);
  }, [panel]);
  return [query, setQuery] as const;
}

function readCollapsedExplorerGroups(): Set<string> {
  try {
    const raw = window.localStorage.getItem(EXPLORER_COLLAPSED_GROUPS_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? new Set(parsed.filter((item): item is string => typeof item === "string"))
      : new Set();
  } catch {
    return new Set();
  }
}

function writeCollapsedExplorerGroups(groups: ReadonlySet<string>) {
  try {
    window.localStorage.setItem(EXPLORER_COLLAPSED_GROUPS_KEY, JSON.stringify([...groups]));
  } catch { /* In-memory collapse state remains usable. */ }
}

export function SessionExplorerPanel({
  sessions,
  activeId,
  colors,
  icons,
  onSelect,
  onConnect,
  onNewSession,
  onImportSessions,
  onOpenContextMenu,
}: {
  sessions: readonly SessionSummary[];
  activeId: string;
  colors: Readonly<Record<string, string>>;
  icons: UtilityPanelIcons;
  onSelect: (id: string) => void;
  onConnect?: (id: string) => void;
  onNewSession?: () => void;
  onImportSessions?: () => void;
  onOpenContextMenu: (event: ReactMouseEvent, id: string, orderedIds?: readonly string[]) => void;
}) {
  useLocale();
  const [query, setQuery] = useUtilityPanelFilter("explorer");
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => readCollapsedExplorerGroups());
  const grouping = useSyncExternalStore(subscribeSessionExplorerGrouping, readSessionExplorerGrouping, () => "profile" as const);
  const visible = useMemo(() => filterWorkspaceSessions(sessions, query), [query, sessions]);
  const groups = useMemo(() => groupExplorerSessions(visible, grouping), [grouping, visible]);
  const filterActive = query.trim().length > 0;
  const orderedIds = useMemo(
    () => groups.flatMap(group => !filterActive && collapsedGroups.has(group.key)
      ? [] : group.sessions.map(session => session.profile.id)),
    [groups, filterActive, collapsedGroups],
  );

  const toggleGroup = useCallback((groupKey: string) => {
    setCollapsedGroups((current) => {
      const next = new Set(current);
      if (next.has(groupKey)) next.delete(groupKey);
      else next.add(groupKey);
      writeCollapsedExplorerGroups(next);
      return next;
    });
  }, []);

  return (
    <>
      <div className="explorer-controls">
        <div className="explorer-filter-row">
          <PanelFilter label={t("filter-explorer-sessions")} value={query} icons={icons} onChange={setQuery} />
          {onNewSession ? (
            <button
              type="button"
              className="explorer-new-button"
              title={t("new-session")}
              aria-label={t("new-session")}
              onClick={onNewSession}
            >
              <Plus size={14} />
            </button>
          ) : null}
        </div>
        <label className="explorer-grouping">
          <span>{t("explorer-grouping-label")}</span>
          <select
            aria-label={t("explorer-grouping")}
            value={grouping}
            onChange={(event) => writeSessionExplorerGrouping(normalizeSessionExplorerGrouping(event.target.value))}
          >
            <option value="profile">{t("explorer-grouping-profile")}</option>
            <option value="type">{t("explorer-grouping-type")}</option>
          </select>
        </label>
      </div>
      <SessionTree
        groups={groups}
        orderedIds={orderedIds}
        collapsedGroups={filterActive ? null : collapsedGroups}
        onToggleGroup={toggleGroup}
        activeId={activeId}
        colors={colors}
        icons={icons}
        hasAnySessions={sessions.length > 0}
        emptyLabel={sessions.length ? t("no-matching-sessions") : t("no-sessions-available-2")}
        onSelect={onSelect}
        onConnect={onConnect}
        onNewSession={onNewSession}
        onImportSessions={onImportSessions}
        onOpenContextMenu={onOpenContextMenu}
      />
    </>
  );
}

type CommandHistoryScope = "session" | "all";

export function getCommandHistoryScope(
  entries: readonly CommandHistoryEntry[],
  activeId: string,
  preferredScope: CommandHistoryScope,
) {
  const sessionEntries = activeId ? entries.filter((entry) => entry.sessionId === activeId) : [];
  const scope = activeId ? preferredScope : "all";

  return {
    scope,
    entries: scope === "session" ? sessionEntries : entries,
    sessionCount: sessionEntries.length,
    allCount: entries.length,
  };
}

type HistoryTimeFormats = {
  compact: Intl.DateTimeFormat;
  full: Intl.DateTimeFormat;
};

export function formatCommandHistoryTimestamp(value: number, formats: HistoryTimeFormats) {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return null;

  return {
    dateTime: date.toISOString(),
    compact: formats.compact.format(date),
    full: formats.full.format(date),
  };
}

type CommandHistoryListProps = {
  entries: readonly CommandHistoryEntry[];
  sessions: readonly SessionSummary[];
  activeId: string;
  icons: UtilityPanelIcons;
  onPick: (entry: CommandHistoryEntry) => void;
  onClear?: () => void;
  canRetryClear?: boolean;
  onInsertToSender?: (entry: CommandHistoryEntry) => void | Promise<void>;
  senderBusy?: boolean;
};

export function CommandHistoryList({
  entries,
  sessions,
  activeId,
  icons,
  onPick,
  onClear,
  canRetryClear = false,
  onInsertToSender,
  senderBusy = false,
}: CommandHistoryListProps) {
  const { locale } = useLocale();
  const controlsId = useId();
  const hintId = `${controlsId}-hint`;
  const [query, setQuery] = useUtilityPanelFilter("history");
  const [preferredScope, setPreferredScope] = useState<CommandHistoryScope>("session");
  const [focusedRowKey, setFocusedRowKey] = useState<string | null>(null);
  const [expandedRowKeys, setExpandedRowKeys] = useState<ReadonlySet<string>>(() => new Set());
  const clearPending = useRef(false);
  const filterInputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const listOwnedFocusRef = useRef(false);
  const { feedback, busy: copyBusy, copy } = useClipboardFeedback();

  const sessionLabels = useMemo(() => new Map(
    sessions.map((session) => [session.profile.id, session.profile.name]),
  ), [sessions]);
  const scoped = useMemo(
    () => getCommandHistoryScope(entries, activeId, preferredScope),
    [entries, activeId, preferredScope],
  );
  const timeFormats = useMemo<HistoryTimeFormats>(() => {
    const language = locale === "zh" ? "zh-CN" : locale;
    return {
      compact: new Intl.DateTimeFormat(language, {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      }),
      full: new Intl.DateTimeFormat(language, { dateStyle: "medium", timeStyle: "long" }),
    };
  }, [locale]);

  const rows = useMemo(() => {
    const seenKeys = new Map<string, number>();
    return filterCommandHistory(scoped.entries, query, sessionLabels).map((entry) => {
      // Normalized history is unique by session and command; keep focus stable on timestamp updates.
      const baseKey = JSON.stringify([entry.sessionId, entry.command]);
      const occurrence = seenKeys.get(baseKey) ?? 0;
      seenKeys.set(baseKey, occurrence + 1);
      return {
        key: occurrence === 0 ? baseKey : `${baseKey}#${occurrence}`,
        entry,
        sessionLabel: commandHistorySessionLabel(entry, sessionLabels),
        timestamp: formatCommandHistoryTimestamp(entry.recordedAt, timeFormats),
      };
    });
  }, [scoped.entries, query, sessionLabels, timeFormats]);
  const focusedRowStillPresent = focusedRowKey !== null && rows.some(({ key }) => key === focusedRowKey);
  const tabRowKey = focusedRowStillPresent ? focusedRowKey : rows[0]?.key;
  const activeSessionName = sessionLabels.get(activeId) ?? t("current-session");
  const copyMessage = feedback?.status === "copied" ? t("copied")
    : feedback?.status === "error" ? t("clipboard-copy-failed") : "";

  const toggleExpandedRow = useCallback((rowKey: string) => {
    setExpandedRowKeys((current) => {
      const next = new Set(current);
      if (next.has(rowKey)) next.delete(rowKey);
      else next.add(rowKey);
      return next;
    });
  }, []);

  useEffect(() => {
    if (entries.length === 0) {
      setExpandedRowKeys((current) => current.size ? new Set() : current);
    }
  }, [entries.length]);

  useEffect(() => {
    if (!listOwnedFocusRef.current || focusedRowStillPresent) return;
    if (typeof document === "undefined") return;
    if (hasActiveInteractionLayer()) {
      listOwnedFocusRef.current = false;
      return;
    }
    const activeEl = document.activeElement;
    // Restore focus only if the previously focused row was detached (activeElement fell back to body/null).
    if (activeEl && activeEl !== document.body && !listRef.current?.contains(activeEl)) {
      listOwnedFocusRef.current = false;
      return;
    }
    const replacement = listRef.current?.querySelector<HTMLButtonElement>('.history-entry-pick[tabindex="0"]');
    if (replacement) {
      replacement.focus({ preventScroll: true });
      return;
    }
    listOwnedFocusRef.current = false;
    filterInputRef.current?.focus({ preventScroll: true });
  }, [focusedRowStillPresent, rows.length, tabRowKey]);

  const handleListFocus = useCallback(() => {
    listOwnedFocusRef.current = true;
  }, []);

  const handleListBlur = useCallback((event: ReactFocusEvent<HTMLUListElement>) => {
    if (event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget)) {
      return;
    }
    if (event.relatedTarget !== null) {
      listOwnedFocusRef.current = false;
      return;
    }
    // Clicking non-focusable chrome emits relatedTarget === null while the target stays connected;
    // row eviction detaches the target from the DOM.
    const blurredTarget = event.target;
    queueMicrotask(() => {
      if (blurredTarget instanceof Node && blurredTarget.isConnected) {
        listOwnedFocusRef.current = false;
      }
    });
  }, []);

  let emptyMessage = t("no-command-history-available");
  if (scoped.entries.length) {
    emptyMessage = t("no-matching-commands-in-history");
  } else if (scoped.scope === "session") {
    emptyMessage = t("no-command-history-for", [activeSessionName]);
  }

  const canClear = scoped.allCount > 0 || canRetryClear;
  async function handleClearHistory() {
    if (!onClear || !canClear || clearPending.current) return;
    clearPending.current = true;
    try {
      const confirmed = await requestAppConfirm({
        title: t("clear-command-history"),
        message: t("confirm-clear-command-history"),
        confirmLabel: t("clear-command-history"),
        danger: true,
      });
      if (confirmed) {
        setExpandedRowKeys(new Set());
        setFocusedRowKey(null);
        onClear();
      }
    } finally {
      clearPending.current = false;
    }
  }

  return (
    <div className="history-panel-layout">
      <div className="history-controls">
        <PanelFilter label={t("filter-command-history")} value={query} icons={icons} inputRef={filterInputRef} onChange={setQuery} />
        <div className="history-toolbar-row">
          <div className="history-scope-switch" role="group" aria-label={t("command-history-scope")}>
            <button
              type="button"
              aria-label={t("current-session")}
              aria-describedby={`${controlsId}-session-count`}
              aria-pressed={scoped.scope === "session"}
              disabled={!activeId}
              onClick={() => setPreferredScope("session")}
            >
              <span className="history-scope-label">{t("current-session")}</span>
              <span className="history-scope-count" id={`${controlsId}-session-count`}>
                {formatUiNumber(scoped.sessionCount)}
              </span>
            </button>
            <button
              type="button"
              aria-label={t("all")}
              aria-describedby={`${controlsId}-all-count`}
              aria-pressed={scoped.scope === "all"}
              onClick={() => setPreferredScope("all")}
            >
              <span className="history-scope-label">{t("all")}</span>
              <span className="history-scope-count" id={`${controlsId}-all-count`}>
                {formatUiNumber(scoped.allCount)}
              </span>
            </button>
          </div>
          {onClear ? (
            <button
              type="button"
              className="history-action history-clear-button"
              title={t("clear-command-history")}
              aria-label={t("clear-command-history")}
              aria-disabled={!canClear}
              onClick={() => void handleClearHistory()}
            >
              <Trash2 size={14} aria-hidden="true" />
            </button>
          ) : null}
        </div>
        <div className="history-help">
          <p id={hintId}>{t("history-pick-hint")}</p>
          <p className="history-status" role="status" data-copy-state={feedback?.status}>
            {copyMessage || (query.trim()
              ? t("history-filter-results", [formatUiNumber(rows.length), formatUiNumber(scoped.entries.length)])
              : "")}
          </p>
        </div>
      </div>
      <div className="history-scroll">
        {rows.length ? (
          <ul
            ref={listRef}
            className="history-list"
            role="list"
            onFocus={handleListFocus}
            onBlur={handleListBlur}
            onKeyDown={handleHistoryNavigation}
          >
            {rows.map((row) => (
              <CommandHistoryRow
                key={row.key}
                rowKey={row.key}
                entry={row.entry}
                sessionLabel={row.sessionLabel}
                timestamp={row.timestamp}
                hintId={hintId}
                tabIndex={row.key === tabRowKey ? 0 : -1}
                locale={locale}
                copyState={feedback?.id === row.key ? feedback.status : undefined}
                copyBusy={copyBusy}
                expanded={expandedRowKeys.has(row.key)}
                senderBusy={senderBusy}
                onFocusRow={setFocusedRowKey}
                onToggleExpand={toggleExpandedRow}
                onPick={onPick}
                onCopy={copy}
                onInsertToSender={onInsertToSender}
              />
            ))}
          </ul>
        ) : <p className="history-empty">{emptyMessage}</p>}
      </div>
    </div>
  );
}

type CommandHistoryRowProps = Pick<CommandHistoryListProps, "onPick" | "onInsertToSender"> & {
  rowKey: string;
  entry: CommandHistoryEntry;
  sessionLabel: string;
  timestamp: ReturnType<typeof formatCommandHistoryTimestamp>;
  hintId: string;
  tabIndex: 0 | -1;
  locale: ReturnType<typeof useLocale>["locale"];
  copyState?: "copied" | "error";
  copyBusy: boolean;
  expanded: boolean;
  senderBusy: boolean;
  onFocusRow: (key: string) => void;
  onToggleExpand: (key: string) => void;
  onCopy: (key: string, text: string) => Promise<void>;
};

const CommandHistoryRow = memo(function CommandHistoryRow({
  rowKey,
  entry,
  sessionLabel,
  timestamp,
  hintId,
  tabIndex,
  locale,
  copyState,
  copyBusy,
  expanded,
  senderBusy,
  onFocusRow,
  onToggleExpand,
  onPick,
  onCopy,
  onInsertToSender,
}: CommandHistoryRowProps) {
  const metaId = useId();
  const codeId = useId();
  const copyLabel = t("copy-command", [], locale);
  const copyTitle = copyState === "copied" ? t("copied", [], locale)
    : copyState === "error" ? t("clipboard-copy-failed", [], locale) : copyLabel;
  const insertLabel = t("history-load-in-send-panel", [], locale);
  const isMultiline = /[\r\n]/.test(entry.command);
  const multilineLabel = isMultiline ? t("history-multiline-command", [], locale) : "";
  const canExpand = isMultiline || entry.command.length > 36;
  const expandLabel = t(expanded ? "history-collapse-command" : "history-expand-command", [], locale);
  const CopyIcon = copyState === "copied" ? Check : copyState === "error" ? CircleAlert : Copy;

  return (
    <li
      className="history-entry-row"
      data-expanded={expanded ? "true" : undefined}
      onFocus={() => onFocusRow(rowKey)}
    >
      <button
        type="button"
        className="history-entry-pick"
        tabIndex={tabIndex}
        title={entry.command}
        aria-describedby={`${hintId} ${metaId}`}
        aria-keyshortcuts="ArrowUp ArrowDown Home End"
        onClick={() => onPick(entry)}
      >
        <SquareTerminal size={14} aria-hidden="true" />
        <code id={codeId} dir="ltr">{entry.command}</code>
      </button>
      <div className="history-entry-actions">
        {onInsertToSender ? (
          <button
            type="button"
            className="history-action history-insert-button"
            tabIndex={tabIndex}
            title={insertLabel}
            aria-label={insertLabel}
            disabled={senderBusy}
            onClick={() => void onInsertToSender(entry)}
          >
            <FileInput size={14} aria-hidden="true" />
          </button>
        ) : null}
        <button
          type="button"
          className="history-action history-copy-button"
          tabIndex={tabIndex}
          title={copyTitle}
          aria-label={copyLabel}
          aria-busy={copyBusy || undefined}
          data-copy-state={copyState}
          onClick={() => void onCopy(rowKey, entry.command)}
        >
          <CopyIcon size={14} aria-hidden="true" />
        </button>
      </div>
      <div id={metaId} className="history-entry-meta">
        <span className="history-entry-origin">
          {multilineLabel ? (
            <span className="history-multiline" role="img" title={multilineLabel} aria-label={multilineLabel}>
              <CornerDownLeft size={12} aria-hidden="true" />
            </span>
          ) : null}
          <bdi title={sessionLabel}>{sessionLabel}</bdi>
        </span>
        <span className="history-entry-meta-end">
          {canExpand ? (
            <button
              type="button"
              className="history-expand-button"
              tabIndex={tabIndex}
              aria-expanded={expanded}
              aria-controls={codeId}
              aria-label={expandLabel}
              title={expandLabel}
              onClick={() => onToggleExpand(rowKey)}
            >
              {expanded ? <ChevronUp size={12} aria-hidden="true" /> : <ChevronDown size={12} aria-hidden="true" />}
            </button>
          ) : null}
          {timestamp ? (
            <time dir="auto" dateTime={timestamp.dateTime} title={timestamp.full}>
              <span aria-hidden="true">{timestamp.compact}</span>
              <span className="sr-only">{timestamp.full}</span>
            </time>
          ) : <span>{t("unknown-time", [], locale)}</span>}
        </span>
      </div>
    </li>
  );
});

function handleHistoryNavigation(event: ReactKeyboardEvent<HTMLUListElement>) {
  if (event.defaultPrevented || event.nativeEvent.isComposing
    || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  if (!(event.target instanceof Element)) return;

  const list = event.currentTarget;
  const row = event.target.closest(".history-entry-row");
  if (!row || row.parentElement !== list) return;

  let nextRow: Element | null;
  switch (event.key) {
    case "ArrowUp": nextRow = row.previousElementSibling; break;
    case "ArrowDown": nextRow = row.nextElementSibling; break;
    case "Home": nextRow = list.firstElementChild; break;
    case "End": nextRow = list.lastElementChild; break;
    default: return;
  }

  event.preventDefault();
  event.stopPropagation();
  nextRow?.querySelector<HTMLButtonElement>(".history-entry-pick")?.focus();
}

export function filterCommandHistory(
  history: readonly CommandHistoryEntry[],
  query: string,
  sessionLabels: ReadonlyMap<string, string> = new Map(),
): CommandHistoryEntry[] {
  const needle = normalizeFilter(query);
  if (!needle) return [...history];
  return history.filter((entry) => normalizeFilter([
    displayCommand(entry.command),
    commandHistorySessionLabel(entry, sessionLabels),
  ].join(" ")).includes(needle));
}

function PanelFilter({
  label,
  value,
  icons,
  inputRef,
  onChange,
}: {
  label: string;
  value: string;
  icons: UtilityPanelIcons;
  inputRef?: RefObject<HTMLInputElement | null>;
  onChange: (value: string) => void;
}) {
  useLocale();
  const { Search, X } = icons;
  const fallbackRef = useRef<HTMLInputElement>(null);
  const resolvedRef = inputRef ?? fallbackRef;
  return (
    <div className="panel-filter">
      <Search size={13} aria-hidden="true" />
      <input
        ref={resolvedRef}
        aria-label={label}
        value={value}
        placeholder={t("filter")}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && value && !event.defaultPrevented && !event.nativeEvent.isComposing) {
            event.preventDefault();
            event.stopPropagation();
            onChange("");
          }
        }}
      />
      {value ? (
        <button
          type="button"
          title={t("clear-filter")}
          aria-label={t("clear-2", [label])}
          onClick={() => {
            resolvedRef.current?.focus({ preventScroll: true });
            onChange("");
          }}
        >
          <X size={13} />
        </button>
      ) : null}
    </div>
  );
}

function SessionTree({
  groups,
  orderedIds,
  collapsedGroups,
  onToggleGroup,
  activeId,
  colors,
  icons,
  hasAnySessions,
  emptyLabel,
  onSelect,
  onConnect,
  onNewSession,
  onImportSessions,
  onOpenContextMenu,
}: {
  groups: readonly SessionExplorerGroup[];
  orderedIds: readonly string[];
  collapsedGroups: ReadonlySet<string> | null;
  onToggleGroup: (groupKey: string) => void;
  activeId: string;
  colors: Readonly<Record<string, string>>;
  icons: UtilityPanelIcons;
  hasAnySessions: boolean;
  emptyLabel: string;
  onSelect: (id: string) => void;
  onConnect?: (id: string) => void;
  onNewSession?: () => void;
  onImportSessions?: () => void;
  onOpenContextMenu: (event: ReactMouseEvent, id: string, orderedIds?: readonly string[]) => void;
}) {
  useLocale();
  const { Folder } = icons;
  if (!groups.length) {
    return (
      <div className="empty-pane top">
        <span>{emptyLabel}</span>
        {!hasAnySessions && (onNewSession || onImportSessions) ? (
          <div className="empty-pane-actions">
            {onNewSession ? <button type="button" onClick={onNewSession}>{t("new-session")}</button> : null}
            {onImportSessions ? <button type="button" onClick={onImportSessions}>{t("import-sessions")}</button> : null}
          </div>
        ) : null}
      </div>
    );
  }
  return (
    <div className="tree-list">
      {groups.map((group) => {
        const label = group.type
          ? sessionExplorerTypeLabel(group.type, t("explorer-serial-type"))
          : group.name || t("explorer-ungrouped");
        const collapsed = Boolean(collapsedGroups?.has(group.key));
        return (
          <div key={group.key} data-session-group={group.key}>
            <button
              type="button"
              className={collapsed ? "tree-folder collapsed" : "tree-folder"}
              disabled={collapsedGroups === null}
              aria-expanded={!collapsed}
              aria-label={collapsedGroups === null ? label : t(collapsed ? "expand-group" : "collapse-group", [label])}
              title={label}
              onClick={() => onToggleGroup(group.key)}
            >
              <Folder size={14} aria-hidden="true" />
              <span>{label}</span>
            </button>
            {!collapsed ? group.sessions.map((session) => {
              const health = sessionRuntimeHealthDescription(session.runtime);
              const customColor = colors[session.profile.id];
              return (
                <button
                  key={session.profile.id}
                  type="button"
                  className={session.profile.id === activeId ? "tree-session active" : "tree-session"}
                  title={`${session.profile.name}\n${health}`}
                  aria-label={session.profile.name}
                  aria-description={health}
                  onClick={() => onSelect(session.profile.id)}
                  onDoubleClick={() => {
                    if (onConnect && ["disconnected", "error", "blocked"].includes(session.runtime.status)) {
                      onConnect(session.profile.id);
                    }
                  }}
                  onContextMenu={(event) => onOpenContextMenu(event, session.profile.id, orderedIds)}
                >
                  <span className={`session-status-dot status-${session.runtime.status}`} aria-hidden="true" />
                  {customColor ? <span className="tree-session-color" style={{ background: customColor }} aria-hidden="true" /> : null}
                  {shellElevationRequested(session.profile) ? <ShellElevationIndicator profile={session.profile} /> : null}
                  <span>{session.profile.name}</span>
                </button>
              );
            }) : null}
          </div>
        );
      })}
    </div>
  );
}

function displayCommand(command: string): string {
  return command.replace(/\s+/g, " ").trim() || command;
}

function commandHistorySessionLabel(
  entry: CommandHistoryEntry,
  sessionLabels: ReadonlyMap<string, string>,
): string {
  if (!entry.sessionId) return t("no-associated-session");
  return sessionLabels.get(entry.sessionId) ?? t("deleted-session", [entry.sessionId]);
}

function normalizeFilter(value: string): string {
  return value.trim().replace(/\s+/g, " ").toLowerCase();
}
