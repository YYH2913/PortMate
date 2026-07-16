import { useMemo, useState } from "react";
import type { ComponentType, MouseEvent as ReactMouseEvent, ReactNode, SVGProps } from "react";
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

export function SessionListPanel({
  sessions,
  activeId,
  icons,
  onSelect,
  onOpenContextMenu,
}: {
  sessions: readonly SessionSummary[];
  activeId: string;
  icons: UtilityPanelIcons;
  onSelect: (id: string) => void;
  onOpenContextMenu: (event: ReactMouseEvent, id: string) => void;
}) {
  const [query, setQuery] = useState("");
  const visible = useMemo(() => filterWorkspaceSessions(sessions, query), [query, sessions]);
  const groups = useMemo(() => groupSessions(visible, (session) => session.profile.kind.toUpperCase()), [visible]);
  return (
    <>
      <PanelFilter label="筛选会话列表" value={query} icons={icons} onChange={setQuery} />
      <SessionTree
        groups={groups}
        activeId={activeId}
        colors={{}}
        icons={icons}
        emptyLabel={sessions.length ? "没有匹配的会话" : "没有可用的会话"}
        onSelect={onSelect}
        onOpenContextMenu={onOpenContextMenu}
      />
    </>
  );
}

export function CommandHistoryList({
  history,
  beforeList,
  icons,
  onPick,
}: {
  history: readonly string[];
  beforeList?: ReactNode;
  icons: UtilityPanelIcons;
  onPick: (value: string) => void;
}) {
  const [query, setQuery] = useState("");
  const visible = useMemo(() => filterCommandHistory(history, query), [history, query]);
  return (
    <>
      <PanelFilter label="筛选历史命令" value={query} icons={icons} onChange={setQuery} />
      <div className="right-tools-list">
        {beforeList}
        {visible.length ? (
          <div className="history-list">
            {visible.map((item, index) => (
              <button key={`${index}-${item}`} type="button" onClick={() => onPick(item)}>
                <span>{displayCommand(item)}</span>
              </button>
            ))}
          </div>
        ) : <div className="empty-pane top">{history.length ? "没有匹配的历史命令" : "没有可用的历史命令"}</div>}
      </div>
    </>
  );
}

export function filterWorkspaceSessions(
  sessions: readonly SessionSummary[],
  query: string,
): SessionSummary[] {
  const needle = normalizeFilter(query);
  if (!needle) return [...sessions];
  return sessions.filter((session) => sessionSearchText(session).includes(needle));
}

export function filterCommandHistory(history: readonly string[], query: string): string[] {
  const needle = normalizeFilter(query);
  if (!needle) return [...history];
  return history.filter((command) => normalizeFilter(displayCommand(command)).includes(needle));
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
          {items.map((session) => (
            <button
              key={session.profile.id}
              type="button"
              className={session.profile.id === activeId ? "tree-session active" : "tree-session"}
              onClick={() => onSelect(session.profile.id)}
              onContextMenu={(event) => onOpenContextMenu(event, session.profile.id)}
            >
              <span className="cyan-dot" style={colors[session.profile.id] ? { background: colors[session.profile.id] } : undefined} />
              <span>{session.profile.name}</span>
            </button>
          ))}
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

function sessionSearchText(session: SessionSummary): string {
  const connection = session.profile.connection;
  let endpoint: string;
  switch (connection.kind) {
    case "ssh":
    case "tmux":
      endpoint = `${connection.username} ${connection.endpoint.host} ${connection.endpoint.port}`;
      break;
    case "tcp":
    case "telnet":
      endpoint = `${connection.host} ${connection.port}`;
      break;
    case "serial":
      endpoint = `${connection.port} ${connection.baudRate}`;
      break;
    case "shell":
      endpoint = `${connection.program} ${connection.args.join(" ")} ${connection.cwd ?? ""}`;
      break;
  }
  return normalizeFilter([
    session.profile.id,
    session.profile.name,
    session.profile.group,
    session.profile.kind,
    session.runtime.status,
    ...session.profile.tags,
    endpoint,
  ].join(" "));
}

function displayCommand(command: string): string {
  return command.replace(/\s+/g, " ").trim() || command;
}

function normalizeFilter(value: string): string {
  return value.trim().toLowerCase();
}
