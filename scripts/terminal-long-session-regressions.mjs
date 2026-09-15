import assert from "node:assert/strict";

/** Real TerminalCanvas path: distinct output timestamps, not one giant write. */
export async function checkTerminalLongSession(page) {
  const host = page.locator('[data-pane-id="pane-a"] .terminal-host');
  await page.evaluate(() => {
    window.__terminalCompatLogs["session-a"] = [];
    window.__longSessionEmit = (id, text, stamp) => window.__emitTauriEvent("portmate-session-event", {
      id, sessionId: "session-a", paneId: "session-a:main", ts: stamp,
      direction: "inbound", stream: "stdout", text, bytesRef: null, annotations: {},
    });
    window.__longSessionEmit("long-clear", "\x1b[?1049l\x1b[2J\x1b[3J\x1b[H", "2026-09-15T00:00:00.000000Z");
  });
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalBuffer === "normal");
  const results = [];
  for (const [start, count] of [[0, 1_000], [20_000, 1_000]]) {
    if (start === 20_000) {
      // Seed a long serialized view through the real restoration path. Besides
      // avoiding 20k browser timer ticks, this catches quadratic marker restore.
      const size = await host.getAttribute("data-terminal-size");
      await page.locator('[data-pane-id="pane-a"] [data-view-id="view-b"] [role="tab"]').click();
      await page.evaluate(async ({ size, count }) => {
        const { terminalStateCache, terminalStateCacheKey } = await import("/src/terminal-state-cache.ts");
        const [cols, rows] = size.split("x").map(Number);
        const timestamps = Array.from({ length: count }, (_, i) => ({ line: i, ts: new Date(Date.UTC(2026, 8, 15) + i + 1).toISOString() }));
        if (!terminalStateCache.save(terminalStateCacheKey("session-a", "view-a"), {
          serialized: timestamps.map((_, i) => `LONG-ROW-${i}\r\n`).join(""), cols, rows,
          seenEventIds: [], timestamps, replayTimestamp: timestamps.at(-1).ts,
        })) throw new Error("long-session cache rejected the fixture");
      }, { size, count: start });
      const restoredAt = Date.now();
      await page.locator('[data-pane-id="pane-a"] [data-view-id="view-a"] [role="tab"]').click();
      await page.waitForFunction(count => Number(document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalTimestampMarkerCount) >= count, start);
      console.log(`Long-session restore: ${Date.now() - restoredAt}ms (${start} timestamps)`);
    }
    const elapsed = await page.evaluate(async ({ start, count }) => {
      const started = performance.now();
      for (let i = start; i < start + count; i += 1) {
        const stamp = new Date(Date.UTC(2026, 8, 15) + i + 1).toISOString();
        window.__longSessionEmit(`long-${i}`, `LONG-ROW-${i}\r\n`, stamp);
      }
      // Wait for the production timestamp gutter to observe all distinct lines.
      await new Promise((resolve, reject) => {
        const check = () => {
          const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
          if (Number(host?.dataset.terminalTimestampMarkerCount) >= start + count) return resolve();
          if (performance.now() - started > 90_000) return reject(new Error("long-session writes failed to drain"));
          setTimeout(check, 10);
        };
        check();
      });
      return performance.now() - started;
    }, { start, count });
    results.push({ historyRows: start, appendedRows: count, milliseconds: Math.round(elapsed) });
    console.log(`Long-session batch: ${JSON.stringify(results.at(-1))}`);
  }
  const before = await host.getAttribute("data-terminal-base-y");
  await page.evaluate(() => window.__longSessionEmit("long-redraw", "\x1b[2K\rLONG-PROMPT> status", "2026-09-15T01:00:00.123456Z"));
  await page.waitForTimeout(150);
  const exported = await page.evaluate(async () => {
    const { requestTerminalTextExport } = await import("/src/terminal-export-event.ts");
    return (await requestTerminalTextExport({ sessionId: "session-a", viewId: "view-a", source: "buffer" })).text;
  });
  assert.equal((exported.match(/LONG-ROW-/g) ?? []).length, 21_000, "long interaction lost retained output");
  assert(exported.includes("LONG-ROW-0") && exported.includes("LONG-ROW-20999"));
  assert(exported.split("\n").find(line => line.includes("LONG-PROMPT> status"))?.includes("2026-09-15T01:00:00.123456Z"));
  assert.equal(await host.getAttribute("data-terminal-base-y"), before, "redrawing the prompt moved the retained history");
  const retainedTimestamps = Number(await host.getAttribute("data-terminal-timestamp-marker-count"));
  assert(retainedTimestamps >= 20_900, `unexpected loss of timestamp intervals: ${retainedTimestamps}, ${await host.evaluate(element => JSON.stringify(element.dataset))}`);
  const backlogStarted = Date.now();
  await page.evaluate(() => {
    for (let i = 0; i < 12_000; i += 1) window.__longSessionEmit(`backlog-${i}`, "", "2026-09-15T01:01:00.000000Z");
    for (let i = 11_000; i < 12_000; i += 1) window.__longSessionEmit(`backlog-${i}`, "DUPLICATE-BACKLOG", "2026-09-15T01:01:00.000000Z");
    window.__longSessionEmit("after-backlog", "\r\nAFTER-BACKLOG> ", "2026-09-15T01:02:00.000000Z");
  });
  await page.waitForFunction(baseY => Number(document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalBaseY) > Number(baseY), before);
  const backlogMilliseconds = Date.now() - backlogStarted;
  const drained = await page.evaluate(async () => {
    const { requestTerminalTextExport } = await import("/src/terminal-export-event.ts");
    return (await requestTerminalTextExport({ sessionId: "session-a", viewId: "view-a", source: "buffer" })).text;
  });
  assert(drained.includes("AFTER-BACKLOG> "), "synchronous backlog stalled subsequent terminal output");
  assert(!drained.includes("DUPLICATE-BACKLOG"), "pending duplicate IDs were evicted before parsing");
  await page.evaluate(() => { window.__invokeCalls = []; });
  await host.locator(".xterm-helper-textarea").focus();
  await page.keyboard.type("status");
  await page.waitForFunction(() => window.__invokeCalls.filter(call => call.command === "send_text" && call.args.sessionId === "session-a")
    .map(call => call.args.text ?? "").join("").includes("status"));
  // A deliberately generous end-to-end budget; complexity is tested separately
  // using marker-access counts rather than machine-dependent timing assertions.
  assert(results.at(-1).milliseconds < 10_000, "input processing stalled after a long interaction");
  return { batches: results, retainedLines: 21_000, redrawTimestamp: true, keyboardInput: true, synchronousBacklog: { events: 12_000, milliseconds: backlogMilliseconds } };
}
