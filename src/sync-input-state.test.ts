import { describe, expect, it } from "vitest";
import { defaultSyncInputSettings, formatSyncInput, normalizeSyncInputSettings, resolveSyncInputTargets, SyncInputDispatcher } from "./sync-input-state";

describe("sync input state", () => {
  it("normalizes protocols, delay, newline, and bounded affixes", () => {
    expect(normalizeSyncInputSettings({
      protocols: ["ssh", "ssh", "serial", "invalid"],
      newlineMode: "crlf",
      delayMs: 99999,
      prefix: "<",
      suffix: ">",
    })).toEqual({
      protocols: ["ssh", "serial"],
      newlineMode: "crlf",
      delayMs: 5000,
      prefix: "<",
      suffix: ">",
    });
  });

  it("always includes the source and filters connected broadcast targets", () => {
    const settings = { ...defaultSyncInputSettings, protocols: ["ssh" as const] };
    expect(resolveSyncInputTargets("serial", [
      { id: "serial", kind: "serial", connected: true },
      { id: "ssh", kind: "ssh", connected: true },
      { id: "ssh-duplicate", kind: "ssh", connected: false },
      { id: "tcp", kind: "tcp", connected: true },
      { id: "ssh", kind: "ssh", connected: true },
    ], settings)).toEqual(["serial", "ssh"]);
  });

  it("keeps a non-empty source even when it is absent from the candidate snapshot", () => {
    expect(resolveSyncInputTargets("source", [
      { id: "extra", kind: "ssh", connected: true },
    ], defaultSyncInputSettings)).toEqual(["source"]);
    expect(resolveSyncInputTargets("", [
      { id: "extra", kind: "ssh", connected: true },
    ], defaultSyncInputSettings)).toEqual([]);
  });

  it("bounds persisted prefix and suffix values", () => {
    const normalized = normalizeSyncInputSettings({
      prefix: "p".repeat(1100),
      suffix: "s".repeat(1100),
    });
    expect(normalized.prefix).toHaveLength(1024);
    expect(normalized.suffix).toHaveLength(1024);
  });

  it("normalizes mixed newlines and applies prefix and suffix", () => {
    expect(formatSyncInput("a\r\nb\rc\n", {
      ...defaultSyncInputSettings,
      newlineMode: "lf",
      prefix: "[",
      suffix: "]",
    })).toBe("[a\nb\nc\n]");
  });

  it("uses CRLF for a Telnet Enter in protocol mode", () => {
    expect(formatSyncInput("show\r", defaultSyncInputSettings, "telnet")).toBe("show\r\n");
    expect(formatSyncInput("show\r", defaultSyncInputSettings, "ssh")).toBe("show\r");
  });

  it("uses protocol-safe Telnet Enter when broadcasting is disabled", async () => {
    const calls: Array<[string, string]> = [];
    const dispatcher = new SyncInputDispatcher();
    const result = await dispatcher.enqueue({
      sourceId: "telnet",
      text: "show\r",
      broadcastEnabled: false,
      applyAffixes: false,
      settings: { ...defaultSyncInputSettings, newlineMode: "preserve", prefix: "ignored" },
      candidates: [{ id: "telnet", kind: "telnet", connected: true }],
    }, async (sessionId, text) => { calls.push([sessionId, text]); }, () => false);
    expect(calls).toEqual([["telnet", "show\r\n"]]);
    expect(result.succeeded).toEqual(["telnet"]);
  });

  it("serializes input batches in FIFO order", async () => {
    let releaseFirst: (() => void) | undefined;
    const firstBlocked = new Promise<void>((resolve) => { releaseFirst = resolve; });
    const calls: string[] = [];
    const dispatcher = new SyncInputDispatcher();
    const candidates = [{ id: "source", kind: "ssh" as const, connected: true }];
    const send = async (_sessionId: string, text: string) => {
      calls.push(text);
      if (text === "first") await firstBlocked;
    };
    const first = dispatcher.enqueue({ sourceId: "source", text: "first", broadcastEnabled: false, applyAffixes: false, settings: defaultSyncInputSettings, candidates }, send, () => false);
    const second = dispatcher.enqueue({ sourceId: "source", text: "second", broadcastEnabled: false, applyAffixes: false, settings: defaultSyncInputSettings, candidates }, send, () => false);
    await Promise.resolve();
    expect(calls).toEqual(["first"]);
    releaseFirst?.();
    await Promise.all([first, second]);
    expect(calls).toEqual(["first", "second"]);
  });

  it("reports failures and cancels delayed extra targets without dropping the source", async () => {
    let enabled = true;
    const waits: number[] = [];
    const dispatcher = new SyncInputDispatcher(async (milliseconds) => { waits.push(milliseconds); });
    const result = await dispatcher.enqueue({
      sourceId: "source",
      text: "run\r",
      broadcastEnabled: true,
      applyAffixes: false,
      settings: { ...defaultSyncInputSettings, delayMs: 25 },
      candidates: [
        { id: "source", kind: "ssh", connected: true },
        { id: "bad", kind: "ssh", connected: true },
        { id: "skipped", kind: "telnet", connected: true },
      ],
    }, async (sessionId) => {
      if (sessionId === "bad") {
        enabled = false;
        throw new Error("failed");
      }
    }, () => enabled);
    expect(result).toEqual({ succeeded: ["source"], failed: ["bad"], skipped: ["skipped"] });
    expect(waits).toEqual([25]);
  });

  it("does not revive a cancelled batch when broadcasting is re-enabled", async () => {
    let enterWait: (() => void) | undefined;
    const waitEntered = new Promise<void>((resolve) => { enterWait = resolve; });
    const dispatcher = new SyncInputDispatcher(() => {
      enterWait?.();
      return new Promise<void>(() => {});
    });
    const calls: string[] = [];
    const resultPromise = dispatcher.enqueue({
      sourceId: "source",
      text: "run",
      broadcastEnabled: true,
      applyAffixes: false,
      settings: { ...defaultSyncInputSettings, delayMs: 25 },
      candidates: [
        { id: "source", kind: "ssh", connected: true },
        { id: "first-extra", kind: "ssh", connected: true },
        { id: "second-extra", kind: "ssh", connected: true },
      ],
    }, async (sessionId) => { calls.push(sessionId); }, () => true);
    await waitEntered;
    dispatcher.cancelBroadcasts();
    const result = await resultPromise;
    expect(calls).toEqual(["source"]);
    expect(result.skipped).toEqual(["first-extra", "second-extra"]);
  });

  it("keeps interactive input streaming and frames each atomic input once", async () => {
    const dispatcher = new SyncInputDispatcher();
    const candidates = [{ id: "source", kind: "ssh" as const, connected: true }];
    const settings = { ...defaultSyncInputSettings, prefix: "[", suffix: "]" };
    const calls: string[] = [];
    const send = async (_sessionId: string, text: string) => { calls.push(text); };
    for (const text of ["a", "b", "c"]) {
      await dispatcher.enqueue({ sourceId: "source", text, broadcastEnabled: true, applyAffixes: false, settings, candidates }, send, () => true);
    }
    await dispatcher.enqueue({ sourceId: "source", text: "abc", broadcastEnabled: true, applyAffixes: true, settings, candidates }, send, () => true);
    expect(calls).toEqual(["a", "b", "c", "[abc]"]);
  });
});
