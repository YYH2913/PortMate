import { afterEach, describe, expect, it, vi } from "vitest";
import {
  SESSION_EXPLORER_GROUPING_KEY, groupExplorerSessions, normalizeSessionExplorerGrouping,
  readSessionExplorerGrouping, sessionExplorerTypeLabel, subscribeSessionExplorerGrouping,
  writeSessionExplorerGrouping,
} from "../../src/session-explorer-grouping";
import { filterWorkspaceSessions } from "../../src/session-search-state";
import type { SessionKind, SessionSummary } from "../../src/types";

function session(id: string, kind: SessionKind, group: string): SessionSummary {
  return {
    profile: { id, name: id, kind, group, tags: [], connection: { kind, endpoint: { host: "localhost", port: 22 }, username: "user", args: [] } },
    runtime: { status: "disconnected" },
  } as unknown as SessionSummary;
}

function storageWindow() {
  const entries = new Map<string, string>();
  const storage = {
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => { entries.set(key, value); },
  };
  const window = Object.assign(new EventTarget(), { localStorage: storage });
  vi.stubGlobal("window", window);
  return { window, storage, entries };
}
function storageEvent(window: EventTarget, key: string | null, storageArea?: object) {
  window.dispatchEvent(Object.assign(new Event("storage"), { key, storageArea }));
}

afterEach(() => {
  // Clear an optional transient choice without exporting production test hooks.
  const { window } = storageWindow();
  const unsubscribe = subscribeSessionExplorerGrouping(() => {});
  storageEvent(window, null);
  unsubscribe();
  vi.unstubAllGlobals();
});

describe("session explorer automatic type grouping", () => {
  it("groups every protocol by connection type in stable order while preserving row order and Profiles", () => {
    const sessions = [
      session("tmux", "tmux", "ops"), session("ssh2", "ssh", "lab"),
      session("tcp", "tcp", "ops"), session("serial", "serial", "lab"),
      session("ssh1", "ssh", "ops"), session("telnet", "telnet", "lab"),
      session("shell", "shell", ""),
    ];
    const before = JSON.stringify(sessions);
    const groups = groupExplorerSessions(sessions, "type");
    expect(groups.map(group => group.type)).toEqual(["ssh", "serial", "shell", "telnet", "tcp", "tmux"]);
    expect(groups[0].sessions.map(item => item.profile.id)).toEqual(["ssh2", "ssh1"]);
    expect(groups.flatMap(group => group.sessions)).toHaveLength(sessions.length);
    expect(groups[0].sessions[0]).toBe(sessions[1]);
    expect(JSON.stringify(sessions)).toBe(before);
    expect(groupExplorerSessions([...sessions].reverse(), "type").map(group => group.type)).toEqual(groups.map(group => group.type));
  });

  it("keeps custom groups intact and handles empty and object-prototype names safely", () => {
    const sessions = [
      session("a", "ssh", "__proto__"), session("b", "serial", "constructor"),
      session("c", "shell", "toString"), session("d", "tcp", "__proto__"),
      session("e", "telnet", ""), session("f", "ssh", "Sessions"),
    ];
    const groups = groupExplorerSessions(sessions, "profile");
    expect(groups.map(group => group.name)).toEqual(["__proto__", "constructor", "toString", "", "Sessions"]);
    expect(groups.map(group => group.type)).toEqual([null, null, null, null, null]);
    expect(groups[0].sessions.map(item => item.profile.id)).toEqual(["a", "d"]);
    expect(new Set(groups.map(group => group.key)).size).toBe(groups.length);
    expect(groupExplorerSessions(sessions, "type").find(group => group.type === "ssh")?.sessions).toEqual([sessions[0], sessions[5]]);
    expect(groupExplorerSessions(sessions, "profile")).toEqual(groups);
    expect(groupExplorerSessions([], "profile")).toEqual([]);
    expect(groupExplorerSessions([], "type")).toEqual([]);
  });

  it("uses the connection discriminator and removes empty groups after existing filtering/deduplication", () => {
    const ssh = session("a", "ssh", "Network");
    const serial = session("b", "serial", "Lab");
    serial.profile.name = "USB COM3";
    const latest = { ...serial, profile: { ...serial.profile, name: "USB COM4" } };
    const filtered = filterWorkspaceSessions([ssh, serial, latest], "USB");
    expect(groupExplorerSessions(filtered, "type").map(group => [group.type, group.sessions.length])).toEqual([["serial", 1]]);
    expect(groupExplorerSessions(filtered, "type")[0].sessions[0].profile.name).toBe("USB COM4");
    const staleKind = { ...ssh, profile: { ...ssh.profile, kind: "serial" as const } };
    expect(groupExplorerSessions([staleKind], "type")[0].type).toBe("ssh");
  });

  it("labels COM/serial explicitly and distinguishes SSH, Shell and Tmux", () => {
    expect(["ssh", "serial", "shell", "telnet", "tcp", "tmux"].map(kind => sessionExplorerTypeLabel(kind as SessionKind, "COM / 串口")))
      .toEqual(["SSH", "COM / 串口", "Shell", "Telnet", "TCP", "Tmux"]);
  });
});

describe("session explorer grouping preference", () => {
  it("defaults to existing profile groups, ignoring corrupt or unknown preferences", () => {
    for (const value of [null, undefined, "", "unknown", "TYPE", "\"type\"", "{}", {}, 1]) {
      expect(normalizeSessionExplorerGrouping(value)).toBe("profile");
    }
    const { entries } = storageWindow();
    expect(readSessionExplorerGrouping()).toBe("profile");
    entries.set(SESSION_EXPLORER_GROUPING_KEY, "type");
    expect(readSessionExplorerGrouping()).toBe("type");
    entries.set(SESSION_EXPLORER_GROUPING_KEY, "invalid");
    expect(readSessionExplorerGrouping()).toBe("profile");
    vi.unstubAllGlobals();
    expect(readSessionExplorerGrouping()).toBe("profile");
  });

  it("persists identifiers, not translations, and notifies local subscribers with removable listeners", () => {
    const { entries } = storageWindow();
    const notify = vi.fn();
    const unsubscribe = subscribeSessionExplorerGrouping(notify);
    writeSessionExplorerGrouping("type");
    expect(entries.get(SESSION_EXPLORER_GROUPING_KEY)).toBe("type");
    expect(readSessionExplorerGrouping()).toBe("type");
    writeSessionExplorerGrouping("profile");
    expect(readSessionExplorerGrouping()).toBe("profile");
    expect(notify).toHaveBeenCalledTimes(2);
    unsubscribe();
    writeSessionExplorerGrouping("type");
    expect(notify).toHaveBeenCalledTimes(2);
  });

  it("reflects other-window writes/removals and clears without reacting to unrelated storage", () => {
    const { window, entries, storage } = storageWindow();
    const notify = vi.fn();
    const unsubscribe = subscribeSessionExplorerGrouping(notify);
    entries.set(SESSION_EXPLORER_GROUPING_KEY, "type");
    storageEvent(window, "unrelated", storage);
    storageEvent(window, SESSION_EXPLORER_GROUPING_KEY, {});
    expect(notify).not.toHaveBeenCalled();
    storageEvent(window, SESSION_EXPLORER_GROUPING_KEY, storage);
    expect(notify).toHaveBeenCalledOnce();
    expect(readSessionExplorerGrouping()).toBe("type");
    entries.delete(SESSION_EXPLORER_GROUPING_KEY);
    storageEvent(window, SESSION_EXPLORER_GROUPING_KEY, storage);
    expect(readSessionExplorerGrouping()).toBe("profile");
    entries.set(SESSION_EXPLORER_GROUPING_KEY, "type");
    entries.clear(); storageEvent(window, null, storage);
    expect(readSessionExplorerGrouping()).toBe("profile");
    expect(notify).toHaveBeenCalledTimes(3);
    unsubscribe();
    storageEvent(window, SESSION_EXPLORER_GROUPING_KEY, storage);
    expect(notify).toHaveBeenCalledTimes(3);
  });

  it("keeps an in-memory choice when persistence is denied, then recovers when storage returns", () => {
    const { window, entries, storage } = storageWindow();
    const notify = vi.fn();
    const unsubscribe = subscribeSessionExplorerGrouping(notify);
    Object.defineProperty(window, "localStorage", { configurable: true, get() { throw Error("denied"); } });
    expect(readSessionExplorerGrouping()).toBe("profile");
    writeSessionExplorerGrouping("type");
    expect(readSessionExplorerGrouping()).toBe("type");
    expect(notify).toHaveBeenCalledOnce();
    Object.defineProperty(window, "localStorage", { configurable: true, value: storage });
    writeSessionExplorerGrouping("type");
    expect(entries.get(SESSION_EXPLORER_GROUPING_KEY)).toBe("type");
    storageEvent(window, null, storage);
    expect(readSessionExplorerGrouping()).toBe("type");
    unsubscribe();
  });
});
