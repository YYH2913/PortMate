import { useCallback, useEffect, useSyncExternalStore } from "react";
import { invokeBackend } from "./api";
import { mergeSysmonHistory, normalizeSysmonHistory } from "./sysmon-history";
import type { SysmonSnapshot } from "./types";

const DEFAULT_SYSMON_HISTORY_LIMIT = 120;
export const SYSMON_POLL_INTERVAL_MS = 10_000;

export type SysmonLiveState = {
  snapshot: SysmonSnapshot | null;
  history: SysmonSnapshot[];
  busy: boolean;
  historyBusy: boolean;
  error: string;
  historyError: string;
};

type SysmonBackendRequest = (
  command: "refresh_sysmon" | "list_sysmon_history",
  args: Record<string, unknown>,
) => Promise<unknown>;

type SysmonScheduler = {
  setInterval: (callback: () => void, milliseconds: number) => unknown;
  clearInterval: (timer: unknown) => void;
};

export type SysmonLiveStore = ReturnType<typeof createSysmonLiveStore>;

const emptySysmonLiveState: SysmonLiveState = {
  snapshot: null,
  history: [],
  busy: false,
  historyBusy: false,
  error: "",
  historyError: "",
};

export function createSysmonLiveStore(
  request: SysmonBackendRequest,
  scheduler: SysmonScheduler = {
    setInterval: (callback, milliseconds) => globalThis.setInterval(callback, milliseconds),
    clearInterval: (timer) => globalThis.clearInterval(timer as ReturnType<typeof setInterval>),
  },
) {
  const states = new Map<string, SysmonLiveState>();
  const listeners = new Map<string, Set<() => void>>();
  const refreshes = new Map<string, Promise<SysmonSnapshot | null>>();
  const historyLoads = new Map<string, Promise<SysmonSnapshot[]>>();
  const pollers = new Map<string, { references: number; timer: unknown }>();

  function getState(sessionId: string): SysmonLiveState {
    let state = states.get(sessionId);
    if (!state) {
      state = { ...emptySysmonLiveState, history: [] };
      states.set(sessionId, state);
    }
    return state;
  }

  function subscribe(sessionId: string, listener: () => void) {
    let sessionListeners = listeners.get(sessionId);
    if (!sessionListeners) {
      sessionListeners = new Set();
      listeners.set(sessionId, sessionListeners);
    }
    sessionListeners.add(listener);
    return () => {
      sessionListeners?.delete(listener);
      if (!sessionListeners?.size) listeners.delete(sessionId);
    };
  }

  function updateState(sessionId: string, update: (state: SysmonLiveState) => SysmonLiveState) {
    const current = getState(sessionId);
    const next = update(current);
    if (next === current) return;
    states.set(sessionId, next);
    for (const listener of listeners.get(sessionId) ?? []) listener();
  }

  function refresh(sessionId: string): Promise<SysmonSnapshot | null> {
    if (!sessionId) return Promise.resolve(null);
    const active = refreshes.get(sessionId);
    if (active) return active;
    updateState(sessionId, (state) => ({ ...state, busy: true, error: "" }));
    const operation = request("refresh_sysmon", { sessionId })
      .then((value) => requireSysmonSnapshot(value, sessionId))
      .then((snapshot) => {
        updateState(sessionId, (state) => ({
          ...state,
          snapshot,
          history: mergeSysmonHistory(state.history, snapshot, DEFAULT_SYSMON_HISTORY_LIMIT),
          error: "",
        }));
        return snapshot;
      })
      .catch((error) => {
        updateState(sessionId, (state) => ({ ...state, error: formatSysmonError(error) }));
        return null;
      })
      .finally(() => {
        refreshes.delete(sessionId);
        updateState(sessionId, (state) => state.busy ? { ...state, busy: false } : state);
      });
    refreshes.set(sessionId, operation);
    return operation;
  }

  function loadHistory(
    sessionId: string,
    limit = DEFAULT_SYSMON_HISTORY_LIMIT,
  ): Promise<SysmonSnapshot[]> {
    if (!sessionId) return Promise.resolve([]);
    const active = historyLoads.get(sessionId);
    if (active) return active;
    const boundedLimit = Math.max(1, Math.min(DEFAULT_SYSMON_HISTORY_LIMIT, Math.trunc(limit) || 1));
    updateState(sessionId, (state) => ({ ...state, historyBusy: true, historyError: "" }));
    const operation = request("list_sysmon_history", { sessionId, limit: boundedLimit })
      .then((value) => {
        if (!Array.isArray(value)) throw new Error("Sysmon history response is invalid");
        const loaded = value.flatMap((candidate) => {
          if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)
            || (candidate as { sessionId?: unknown }).sessionId !== sessionId) return [];
          try {
            return [requireSysmonSnapshot(candidate, sessionId)];
          } catch {
            return [];
          }
        });
        let history: SysmonSnapshot[] = [];
        updateState(sessionId, (state) => {
          history = normalizeSysmonHistory(
            [...state.history, ...loaded],
            sessionId,
            boundedLimit,
          );
          const latest = history.at(-1) ?? null;
          return {
            ...state,
            snapshot: latestSnapshot(state.snapshot, latest),
            history,
            historyError: "",
          };
        });
        return history;
      })
      .catch((error) => {
        updateState(sessionId, (state) => ({ ...state, historyError: formatSysmonError(error) }));
        return [];
      })
      .finally(() => {
        historyLoads.delete(sessionId);
        updateState(sessionId, (state) => state.historyBusy ? { ...state, historyBusy: false } : state);
      });
    historyLoads.set(sessionId, operation);
    return operation;
  }

  function startPolling(sessionId: string, intervalMs = SYSMON_POLL_INTERVAL_MS) {
    if (!sessionId) return () => {};
    const current = pollers.get(sessionId);
    if (current) {
      current.references += 1;
    } else {
      void refresh(sessionId).then((snapshot) => {
        if (!snapshot && pollers.has(sessionId)) void refresh(sessionId);
      });
      pollers.set(sessionId, {
        references: 1,
        timer: scheduler.setInterval(() => void refresh(sessionId), intervalMs),
      });
    }
    let stopped = false;
    return () => {
      if (stopped) return;
      stopped = true;
      const active = pollers.get(sessionId);
      if (!active) return;
      active.references -= 1;
      if (active.references > 0) return;
      scheduler.clearInterval(active.timer);
      pollers.delete(sessionId);
    };
  }

  return { getState, subscribe, refresh, loadHistory, startPolling };
}

const sysmonLiveStore = createSysmonLiveStore((command, args) => invokeBackend(command, args));

export function useSysmonLiveState(sessionId: string | null | undefined): SysmonLiveState {
  const subscribe = useCallback(
    (listener: () => void) => sessionId ? sysmonLiveStore.subscribe(sessionId, listener) : () => {},
    [sessionId],
  );
  const getSnapshot = useCallback(
    () => sessionId ? sysmonLiveStore.getState(sessionId) : emptySysmonLiveState,
    [sessionId],
  );
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export function useSysmonLivePolling(
  sessionId: string | null | undefined,
  enabled: boolean,
) {
  useEffect(() => {
    if (!enabled || !sessionId) return;
    return sysmonLiveStore.startPolling(sessionId);
  }, [enabled, sessionId]);
}

export function refreshSysmonLive(sessionId: string) {
  return sysmonLiveStore.refresh(sessionId);
}

export function loadSysmonLiveHistory(sessionId: string, limit = DEFAULT_SYSMON_HISTORY_LIMIT) {
  return sysmonLiveStore.loadHistory(sessionId, limit);
}

function requireSysmonSnapshot(value: unknown, sessionId: string): SysmonSnapshot {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Sysmon snapshot response does not match the requested session");
  }
  const snapshot = value as Partial<SysmonSnapshot>;
  const numeric = [
    snapshot.uptimeSeconds,
    snapshot.cpuPercent,
    snapshot.memoryPercent,
    snapshot.rxKbps,
    snapshot.txKbps,
    snapshot.memoryTotalBytes,
    snapshot.memoryAvailableBytes,
  ];
  if (snapshot.sessionId !== sessionId
    || typeof snapshot.ts !== "string"
    || !Number.isFinite(Date.parse(snapshot.ts))
    || numeric.some((entry) => typeof entry !== "number" || !Number.isFinite(entry))
    || !Array.isArray(snapshot.loadAverage)
    || snapshot.loadAverage.length !== 3
    || snapshot.loadAverage.some((entry) => !Number.isFinite(entry))
    || !Array.isArray(snapshot.processes)
    || !Array.isArray(snapshot.disks)
    || !Array.isArray(snapshot.networkInterfaces)) {
    throw new Error("Sysmon snapshot response does not match the requested session");
  }
  return snapshot as SysmonSnapshot;
}

function latestSnapshot(
  current: SysmonSnapshot | null,
  candidate: SysmonSnapshot | null,
): SysmonSnapshot | null {
  if (!current) return candidate;
  if (!candidate) return current;
  return Date.parse(candidate.ts) >= Date.parse(current.ts) ? candidate : current;
}

function formatSysmonError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
