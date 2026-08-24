import { useMemo, useState } from "react";
import type { ComponentType, MouseEvent as ReactMouseEvent, SVGProps } from "react";
import type { CommandHistoryEntry } from "./command-history-state";
import { filterWorkspaceSessions } from "./session-search-state";
import { sessionRuntimeHealthDescription } from "./session-runtime-state";
import type { SessionSummary } from "./types";

type UtilityIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: string | number }>;
type UtilityPanelIcons = {
  Folder: UtilityIcon;
  Search: UtilityIcon;
  X: UtilityIcon;
};

export function SessionExplorerPanel({
  sessions,
  activeId,
  colors,
  icons,
  onSelect,
  onOpenContextMenu,
}: {
  sessions: readonly SessionSummary[];
  activeId: string;
  colors: Readonly<Record<string, string>>;
  icons: UtilityPanelIcons;
  onSelect: (id: string) => void;
  onOpenContextMenu: (event: ReactMouseEvent, id: string) => void;
}) {
  const [query, setQuery] = useState("");
  const visible = useMemo(() => filterWorkspaceSessions(sessions, query), [query, sessions]);
  const groups = useMemo(() => groupSessions(visible, (session) => session.profile.group || "Sessions"), [visible]);
  return (
    <>
      <PanelFilter label="筛选资源管理器会话" value={query} icons={icons} onChange={setQuery} />
      <SessionTree
        groups={groups}
        activeId={activeId}
        colors={colors}
        icons={icons}
        emptyLabel={sessions.length ? "没有匹配的会话" : "没有可用的会话"}
        onSelect={onSelect}
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
}: {
  entries: readonly CommandHistoryEntry[];
  sessions: readonly SessionSummary[];
  activeId: string;
  icons: UtilityPanelIcons;
  onPick: (entry: CommandHistoryEntry) => void;
}) {
  const [query, setQuery] = useState("");
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
  const activeSessionName = sessionLabels.get(activeId) ?? "当前会话";
  return (
    <>
      <PanelFilter label="筛选历史命令" value={query} icons={icons} onChange={setQuery} />
      <div className="history-scope-switch" role="group" aria-label="历史命令范围">
        <button type="button" className={scope === "session" ? "active" : ""} aria-pressed={scope === "session"} onClick={() => setScope("session")}>当前会话</button>
        <button type="button" className={scope === "all" ? "active" : ""} aria-pressed={scope === "all"} onClick={() => setScope("all")}>全部</button>
      </div>
      <div className="right-tools-list">
        {visible.length ? (
          <div className="history-list">
            {visible.map((entry, index) => (
              <button
                key={`${entry.sessionId ?? "legacy"}-${entry.recordedAt}-${index}-${entry.command}`}
                type="button"
                title={entry.command}
                onClick={() => onPick(entry)}
              >
                <span>{displayCommand(entry.command)}</span>
                <small>
                  <span>{commandHistorySessionLabel(entry, sessionLabels)}</span>
                  <time dateTime={new Date(entry.recordedAt).toISOString()}>{formatCommandHistoryTime(entry.recordedAt)}</time>
                </small>
              </button>
            ))}
          </div>
        ) : <div className="empty-pane top">{
          scoped.length
            ? "没有匹配的历史命令"
            : scope === "session" && activeId
              ? `${activeSessionName} 尚无历史命令`
              : "没有可用的历史命令"
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
  const { Search, X } = icons;
  return (
    <div className="panel-filter">
      <Search size={13} aria-hidden="true" />
      <input aria-label={label} value={value} placeholder="筛选" onChange={(event) => onChange(event.target.value)} />
      {value ? (
        <button type="button" title="清除筛选" aria-label={`清除${label}`} onClick={() => onChange("")}>
          <X size={13} />
        </button>
      ) : null}
    </div>
  );
}

function SessionTree({
  groups,
  activeId,
  colors,
  icons,
  emptyLabel,
  onSelect,
  onOpenContextMenu,
}: {
  groups: Readonly<Record<string, SessionSummary[]>>;
  activeId: string;
  colors: Readonly<Record<string, string>>;
  icons: UtilityPanelIcons;
  emptyLabel: string;
  onSelect: (id: string) => void;
  onOpenContextMenu: (event: ReactMouseEvent, id: string) => void;
}) {
  const { Folder } = icons;
  if (!Object.keys(groups).length) return <div className="empty-pane top">{emptyLabel}</div>;
  return (
    <div className="tree-list">
      {Object.entries(groups).map(([group, items]) => (
        <div key={group}>
          <div className="tree-folder">
            <Folder size={14} />
            <span>{group}</span>
          </div>
          {items.map((session) => {
            const health = sessionRuntimeHealthDescription(session.runtime);
            return (
              <button
                key={session.profile.id}
                type="button"
                className={session.profile.id === activeId ? "tree-session active" : "tree-session"}
                title={`${session.profile.name}\n${health}`}
                aria-label={session.profile.name}
                aria-description={health}
                onClick={() => onSelect(session.profile.id)}
                onContextMenu={(event) => onOpenContextMenu(event, session.profile.id)}
              >
                <span className="cyan-dot" style={colors[session.profile.id] ? { background: colors[session.profile.id] } : undefined} />
                <span>{session.profile.name}</span>
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}

function groupSessions(
  sessions: readonly SessionSummary[],
  keyFor: (session: SessionSummary) => string,
): Record<string, SessionSummary[]> {
  return sessions.reduce<Record<string, SessionSummary[]>>((groups, session) => {
    const key = keyFor(session);
    groups[key] ??= [];
    groups[key].push(session);
    return groups;
  }, {});
}

function displayCommand(command: string): string {
  return command.replace(/\s+/g, " ").trim() || command;
}

function commandHistorySessionLabel(
  entry: CommandHistoryEntry,
  sessionLabels: ReadonlyMap<string, string>,
): string {
  if (!entry.sessionId) return "未关联会话";
  return sessionLabels.get(entry.sessionId) ?? `已删除会话 · ${entry.sessionId}`;
}

function formatCommandHistoryTime(value: number): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "时间未知" : date.toLocaleString();
}

function normalizeFilter(value: string): string {
  return value.trim().toLowerCase();
}
