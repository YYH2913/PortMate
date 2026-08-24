import { describe, expect, it } from "vitest";
import { createSysmonLiveStore } from "./sysmon-live-state";
import type { SysmonSnapshot } from "./types";

describe("Sysmon live state", () => {
  it("deduplicates concurrent refreshes and shares the resulting snapshot", async () => {
    const requests: DeferredRequest[] = [];
    const store = createSysmonLiveStore((command, args) => new Promise((resolve, reject) => {
      requests.push({ command, args, resolve, reject });
    }));
    const updates: number[] = [];
    store.subscribe("router", () => updates.push(updates.length + 1));

    const first = store.refresh("router");
    const second = store.refresh("router");
    expect(requests).toHaveLength(1);
    expect(store.getState("router").busy).toBe(true);
    requests[0].resolve(snapshot("router", "2026-08-24T10:00:00.000Z", 12));

    expect(await first).toMatchObject({ sessionId: "router", cpuPercent: 12 });
    expect(await second).toMatchObject({ sessionId: "router", cpuPercent: 12 });
    expect(store.getState("router")).toMatchObject({ busy: false, error: "" });
    expect(store.getState("router").history).toHaveLength(1);
    expect(updates.length).toBeGreaterThanOrEqual(2);
  });

  it("keeps one polling timer until the final consumer releases it", async () => {
    const requests: DeferredRequest[] = [];
    const timers = new Map<number, () => void>();
    const cleared: number[] = [];
    let timerId = 0;
    const store = createSysmonLiveStore(
      (command, args) => new Promise((resolve, reject) => requests.push({ command, args, resolve, reject })),
      {
        setInterval: (callback) => {
          const id = ++timerId;
          timers.set(id, callback);
          return id;
        },
        clearInterval: (timer) => {
          cleared.push(timer as number);
          timers.delete(timer as number);
        },
      },
    );

    const stopSidebar = store.startPolling("router");
    const stopApplet = store.startPolling("router");
    expect(requests).toHaveLength(1);
    expect(timers.size).toBe(1);
    stopApplet();
    expect(cleared).toEqual([]);
    requests[0].resolve(snapshot("router", "2026-08-24T10:00:00.000Z", 10));
    await new Promise((resolve) => setTimeout(resolve, 0));

    timers.values().next().value?.();
    expect(requests).toHaveLength(2);
    stopSidebar();
    expect(cleared).toEqual([1]);
  });

  it("deduplicates history loads and rejects samples from another session", async () => {
    const requests: DeferredRequest[] = [];
    const store = createSysmonLiveStore((command, args) => new Promise((resolve, reject) => {
      requests.push({ command, args, resolve, reject });
    }));

    const first = store.loadHistory("router");
    const second = store.loadHistory("router");
    expect(requests).toHaveLength(1);
    requests[0].resolve([
      snapshot("other", "2026-08-24T10:00:01.000Z", 99),
      { ...snapshot("router", "2026-08-24T10:00:02.000Z", 80), cpuPercent: "invalid" },
      snapshot("router", "2026-08-24T10:00:00.000Z", 20),
    ]);
    expect(await first).toHaveLength(1);
    expect(await second).toHaveLength(1);
    expect(store.getState("router").snapshot?.cpuPercent).toBe(20);

    const refresh = store.refresh("router");
    requests[1].resolve(snapshot("other", "2026-08-24T10:00:02.000Z", 80));
    expect(await refresh).toBeNull();
    expect(store.getState("router").snapshot?.cpuPercent).toBe(20);
    expect(store.getState("router").error).toMatch(/does not match/);
  });

  it("retries immediately when a restarted poller inherits a stale request", async () => {
    const requests: DeferredRequest[] = [];
    const store = createSysmonLiveStore(
      (command, args) => new Promise((resolve, reject) => requests.push({ command, args, resolve, reject })),
      { setInterval: () => 1, clearInterval: () => {} },
    );

    const stopOldRuntime = store.startPolling("router");
    stopOldRuntime();
    const stopNewRuntime = store.startPolling("router");
    expect(requests).toHaveLength(1);
    requests[0].resolve(snapshot("stale-router", "2026-08-24T10:00:00.000Z", 10));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(requests).toHaveLength(2);
    requests[1].resolve(snapshot("router", "2026-08-24T10:00:01.000Z", 20));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(store.getState("router").snapshot?.cpuPercent).toBe(20);
    stopNewRuntime();
  });

  it("preserves the last sample across a transient refresh failure", async () => {
    const outcomes: Array<SysmonSnapshot | Error> = [
      snapshot("router", "2026-08-24T10:00:00.000Z", 30),
      new Error("temporary collector failure"),
      snapshot("router", "2026-08-24T10:00:02.000Z", 40),
    ];
    const store = createSysmonLiveStore(async () => {
      const outcome = outcomes.shift();
      if (outcome instanceof Error) throw outcome;
      return outcome;
    });

    await store.refresh("router");
    await store.refresh("router");
    expect(store.getState("router").snapshot?.cpuPercent).toBe(30);
    expect(store.getState("router").error).toBe("temporary collector failure");
    await store.refresh("router");
    expect(store.getState("router").snapshot?.cpuPercent).toBe(40);
    expect(store.getState("router").error).toBe("");
  });
});

type DeferredRequest = {
  command: string;
  args: Record<string, unknown>;
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
};

function snapshot(sessionId: string, ts: string, cpuPercent: number): SysmonSnapshot {
  return {
    sessionId,
    ts,
    uptimeSeconds: 60,
    cpuPercent,
    memoryPercent: 25,
    rxKbps: 1,
    txKbps: 2,
    loadAverage: [0.1, 0.2, 0.3],
    memoryTotalBytes: 1024,
    memoryAvailableBytes: 768,
    processes: [],
    disks: [],
    networkInterfaces: [],
  };
}
