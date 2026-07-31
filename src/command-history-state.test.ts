import { describe, expect, it } from "vitest";
import {
  commandHistoryCommands,
  commandHistoryEntriesEqual,
  commandHistorySnapshot,
  MAX_COMMAND_HISTORY_COMMAND_CHARACTERS,
  MAX_COMMAND_HISTORY_STORAGE_BYTES,
  normalizeCommandHistoryCommand,
  normalizeCommandHistory,
  normalizeCommandHistoryPolicy,
  queuePendingCommandHistory,
  recordCommandHistory,
} from "./command-history-state";

describe("command history state", () => {
  it("normalizes bounded history preferences", () => {
    expect(normalizeCommandHistoryPolicy("250", "90")).toEqual({ limit: 250, retentionDays: 90 });
    expect(normalizeCommandHistoryPolicy("0", "-1")).toEqual({ limit: 1, retentionDays: 0 });
    expect(normalizeCommandHistoryPolicy("999999", "999999")).toEqual({ limit: 10_000, retentionDays: 3_650 });
    expect(normalizeCommandHistoryPolicy("bad", null)).toEqual({ limit: 10_000, retentionDays: 30 });
  });

  it("migrates legacy strings, preserves order, and rejects unsafe entries", () => {
    const now = Date.UTC(2026, 6, 16);
    const entries = normalizeCommandHistory([
      "git status",
      "npm test",
      "git status",
      "",
      "bad\0command",
      "x".repeat(MAX_COMMAND_HISTORY_COMMAND_CHARACTERS + 1),
      42,
    ], { limit: 10, retentionDays: 30 }, now);
    expect(commandHistoryCommands(entries)).toEqual(["git status", "npm test"]);
    expect(entries[0].recordedAt).toBe(now);
    expect(entries[1].recordedAt).toBe(now - 1);
  });

  it("expires old entries, clamps future timestamps, and supports no expiry", () => {
    const now = Date.UTC(2026, 6, 16);
    const value = {
      version: 2,
      entries: [
        { command: "future", recordedAt: now + 60_000 },
        { command: "recent", recordedAt: now - 2 * 86_400_000 },
        { command: "old", recordedAt: now - 40 * 86_400_000 },
        { command: "invalid", recordedAt: "never" },
      ],
    };
    expect(normalizeCommandHistory(value, { limit: 10, retentionDays: 30 }, now)).toEqual([
      { command: "future", recordedAt: now },
      { command: "recent", recordedAt: now - 2 * 86_400_000 },
    ]);
    expect(commandHistoryCommands(normalizeCommandHistory(value, { limit: 10, retentionDays: 0 }, now))).toEqual([
      "future",
      "recent",
      "old",
    ]);
  });

  it("moves repeated commands to the front and enforces the configured limit", () => {
    const policy = { limit: 2, retentionDays: 30 };
    const first = recordCommandHistory([], "git status", policy, 100);
    const second = recordCommandHistory(first, "npm test", policy, 200);
    const repeated = recordCommandHistory(second, "git status", policy, 300);
    expect(repeated).toEqual([
      { command: "git status", recordedAt: 300 },
      { command: "npm test", recordedAt: 200 },
    ]);
    expect(recordCommandHistory(repeated, "   ", policy, 400)).toEqual(repeated);
  });

  it("keeps pending backend writes valid, unique, newest, and bounded", () => {
    const policy = { limit: 2, retentionDays: 30 };
    expect(normalizeCommandHistoryCommand("  git status  ")).toBe("  git status  ");
    expect(normalizeCommandHistoryCommand("   ")).toBeNull();
    expect(normalizeCommandHistoryCommand("bad\0command")).toBeNull();
    expect(normalizeCommandHistoryCommand("x".repeat(MAX_COMMAND_HISTORY_COMMAND_CHARACTERS + 1))).toBeNull();

    let pending = queuePendingCommandHistory([], "a", policy, 1_000);
    pending = queuePendingCommandHistory(pending, "b", policy, 1_001);
    pending = queuePendingCommandHistory(pending, "a", policy, 1_002);
    expect(pending).toEqual(["b", "a"]);
    expect(queuePendingCommandHistory(pending, "   ", policy, 1_003)).toEqual(pending);
    expect(queuePendingCommandHistory(pending, "c", policy, 1_004)).toEqual(["a", "c"]);
  });

  it("bounds pending backend writes by the persisted UTF-8 payload budget", () => {
    const command = "\"\\\n界".repeat(2_000);
    let pending: string[] = [];
    for (let index = 0; index < 200; index += 1) {
      pending = queuePendingCommandHistory(
        pending,
        `${index}-${command}`,
        { limit: 10_000, retentionDays: 0 },
        10_000 + index,
      );
    }
    const entries = [...pending].reverse().map((item, index) => ({ command: item, recordedAt: 20_000 - index }));
    const bytes = new TextEncoder().encode(JSON.stringify({ version: 2, entries })).byteLength;
    expect(bytes).toBeLessThanOrEqual(MAX_COMMAND_HISTORY_STORAGE_BYTES);
    expect(pending.length).toBeLessThan(200);
  });

  it("bounds the serialized UTF-8 payload and returns defensive snapshots", () => {
    const command = "\"\\\n界".repeat(2_000);
    const source = Array.from({ length: 200 }, (_, index) => ({ command: `${index}-${command}`, recordedAt: 1_000 - index }));
    const entries = normalizeCommandHistory({ version: 2, entries: source }, { limit: 10_000, retentionDays: 0 }, 1_000);
    const bytes = new TextEncoder().encode(JSON.stringify(commandHistorySnapshot(entries))).byteLength;
    expect(bytes).toBeLessThanOrEqual(MAX_COMMAND_HISTORY_STORAGE_BYTES);
    expect(entries.length).toBeLessThan(source.length);
    const snapshot = commandHistorySnapshot(entries);
    expect(commandHistoryEntriesEqual(entries, snapshot.entries)).toBe(true);
    snapshot.entries[0].command = "changed";
    expect(entries[0].command).not.toBe("changed");
  });
});
