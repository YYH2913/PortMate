import { t, useLocale } from "./i18n";
import { useCallback, useMemo, useState, useSyncExternalStore } from "react";
import type { ComponentType, MouseEvent as ReactMouseEvent, SVGProps } from "react";
import { Copy, Plus, Trash2 } from "lucide-react";
import { requestAppConfirm } from "./app-dialog";
import type { CommandHistoryEntry } from "./command-history-state";
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

export function CommandHistoryList({
  entries,
  sessions,
  activeId,
  icons,
  onPick,
  onClear,
}: {
  entries: readonly CommandHistoryEntry[];
  sessions: readonly SessionSummary[];
  activeId: string;
  icons: UtilityPanelIcons;
  onPick: (entry: CommandHistoryEntry) => void;
  onClear?: () => void;
}) {
  useLocale();
  const [query, setQuery] = useUtilityPanelFilter("history");
  const [scope, setScope] = useState<"session" | "all">("session");
  const sessionLabels = useMemo(() => new Map(
    sessions.map((session) => [session.profile.id, session.profile.name]),
  ), [sessions]);
  const scoped = useMemo(() => (
    scope === "session" && activeId
      ? entries.filter((entry) => entry.sessionId === activeId)
      : entries
  ), [activeId, entries, scope]);
  const visible = useMemo(
    () => filterCommandHistory(scoped, query, sessionLabels),
    [query, scoped, sessionLabels],
  );
  const activeSessionName = sessionLabels.get(activeId) ?? t("current-session");

  async function handleClearHistory() {
    if (!onClear || !entries.length) return;
    const confirmed = await requestAppConfirm({
      title: t("command-history"),
      message: t("confirm-clear-command-history"),
      confirmLabel: t("delete"),
      danger: true,
    });
    if (confirmed) onClear();
  }

  return (
    <>
      <PanelFilter label={t("filter-command-history")} value={query} icons={icons} onChange={setQuery} />
      <div className="history-toolbar-row">
        <div className="history-scope-switch" role="group" aria-label={t("command-history-scope")}>
          <button type="button" className={scope === "session" ? "active" : ""} aria-pressed={scope === "session"} onClick={() => setScope("session")}>{t("current-session")}</button>
          <button type="button" className={scope === "all" ? "active" : ""} aria-pressed={scope === "all"} onClick={() => setScope("all")}>{t("all")}</button>
        </div>
        {onClear ? (
          <button
            type="button"
            className="history-clear-button"
            title={t("clear-command-history")}
            aria-label={t("clear-command-history")}
            disabled={!entries.length}
            onClick={() => void handleClearHistory()}
          >
            <Trash2 size={13} />
          </button>
        ) : null}
      </div>
      <div className="right-tools-list">
        {visible.length ? (
          <div className="history-list">
            {visible.map((entry, index) => (
              <div key={`${entry.sessionId ?? "legacy"}-${entry.recordedAt}-${index}-${entry.command}`} className="history-entry-row">
                <button
                  type="button"
                  className="history-entry-pick"
                  title={entry.command}
                  onClick={() => onPick(entry)}
                >
                  <span>{displayCommand(entry.command)}</span>
                  <small>
                    <span>{commandHistorySessionLabel(entry, sessionLabels)}</span>
                    <time dateTime={new Date(entry.recordedAt).toISOString()}>{formatCommandHistoryTime(entry.recordedAt)}</time>
                  </small>
                </button>
                <button
                  type="button"
                  className="history-copy-button"
                  title={t("copy-command")}
                  aria-label={t("copy-command")}
                  onClick={(event) => {
                    event.stopPropagation();
                    void navigator.clipboard?.writeText(entry.command).catch(() => {});
                  }}
                >
                  <Copy size={12} />
                </button>
              </div>
            ))}
          </div>
        ) : <div className="empty-pane top">{
          scoped.length
            ? t("no-matching-commands-in-history")
            : scope === "session" && activeId
              ? t("no-command-history-for", [activeSessionName])
              : t("no-command-history-available")
        }</div>}
      </div>
    </>
  );
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
  onChange,
}: {
  label: string;
  value: string;
  icons: UtilityPanelIcons;
  onChange: (value: string) => void;
}) {
  useLocale();
  const { Search, X } = icons;
  return (
    <div className="panel-filter">
      <Search size={13} aria-hidden="true" />
      <input aria-label={label} value={value} placeholder={t("filter")} onChange={(event) => onChange(event.target.value)} />
      {value ? (
        <button type="button" title={t("clear-filter")} aria-label={t("clear-2", [label])} onClick={() => onChange("")}>
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

function formatCommandHistoryTime(value: number): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? t("unknown-time") : date.toLocaleString();
}

function normalizeFilter(value: string): string {
  return value.trim().toLowerCase();
}
