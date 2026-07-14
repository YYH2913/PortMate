import type { SysmonSnapshot } from "./types";

export type SysmonTrendMode = "usage" | "network";

export function normalizeSysmonHistory(
  snapshots: SysmonSnapshot[],
  sessionId: string,
  limit = 120,
) {
  const boundedLimit = Math.max(1, Math.trunc(limit) || 1);
  const byTimestamp = new Map<string, SysmonSnapshot>();
  for (const snapshot of snapshots) {
    if (snapshot.sessionId !== sessionId || !Number.isFinite(Date.parse(snapshot.ts))) continue;
    byTimestamp.set(snapshot.ts, snapshot);
  }
  return [...byTimestamp.values()]
    .sort((left, right) => Date.parse(left.ts) - Date.parse(right.ts))
    .slice(-boundedLimit);
}

export function mergeSysmonHistory(
  current: SysmonSnapshot[],
  snapshot: SysmonSnapshot,
  limit = 120,
) {
  return normalizeSysmonHistory([...current, snapshot], snapshot.sessionId, limit);
}

export function sysmonTrendMax(history: SysmonSnapshot[], mode: SysmonTrendMode) {
  if (mode === "usage") return 100;
  const maximum = history.reduce(
    (value, snapshot) => Math.max(
      value,
      finiteNonnegative(snapshot.rxKbps),
      finiteNonnegative(snapshot.txKbps),
    ),
    0,
  );
  if (maximum <= 1) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(maximum));
  const normalized = maximum / magnitude;
  const step = normalized <= 1 ? 1 : normalized <= 2 ? 2 : normalized <= 5 ? 5 : 10;
  return step * magnitude;
}

export function sysmonTrendValue(snapshot: SysmonSnapshot, mode: SysmonTrendMode, series: 0 | 1) {
  if (mode === "usage") {
    return Math.min(100, finiteNonnegative(series === 0 ? snapshot.cpuPercent : snapshot.memoryPercent));
  }
  return finiteNonnegative(series === 0 ? snapshot.rxKbps : snapshot.txKbps);
}

function finiteNonnegative(value: number) {
  return Number.isFinite(value) ? Math.max(0, value) : 0;
}
