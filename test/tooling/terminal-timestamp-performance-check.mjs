import assert from "node:assert/strict";

export async function checkTerminalTimestampPerformance(page) {
  const sessionId = "session-a";
  const viewId = "view-a";
  await page.evaluate(() => {
    window.__terminalCompatLogs["session-a"] = [];
    window.__timestampBurst = (id, text, ts) => {
      const event = { id, sessionId: "session-a", paneId: "session-a:main", ts, direction: "inbound", stream: "stdout", text, bytesRef: null, annotations: {} };
      const bytes = [...new TextEncoder().encode(text)];
      window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
      return event;
    };
    window.__timestampBurst("burst-clear", "\x1b[?1049l\x1b[2J\x1b[3J\x1b[H", "2026-11-07T00:00:00.000000Z");
  });
  await page.waitForTimeout(60);
  const result = await page.evaluate(async () => {
    const frames = 2400;
    const started = performance.now();
    const gaps = [];
    let previous = started;
    const tick = () => { const now = performance.now(); gaps.push(now - previous); previous = now; raf = requestAnimationFrame(tick); };
    let raf = requestAnimationFrame(tick);
    const events = [];
    for (let i = 0; i < frames; i++) {
      // Differ by seconds, offset and sub-millisecond precision. A shared first
      // timestamp across a raw-byte batch must fail for every later row.
      const ts = new Date(Date.UTC(2026, 10, 7) + i * 1000).toISOString().replace(".000Z", ".123456Z");
      events.push(window.__timestampBurst(`burst-${i}`, `TIMED-ROW-${i}\r\n`, ts));
    }
    const { requestTerminalTextExport } = await import("/src/terminal-export-event.ts");
    let text = "";
    while (!text.includes(`TIMED-ROW-${frames - 1}`)) {
      if (performance.now() - started > 10_000) throw Error("canonical output stalled for 10 seconds");
      await new Promise(resolve => setTimeout(resolve, 50));
      text = (await requestTerminalTextExport({ sessionId: "session-a", viewId: "view-a", source: "buffer" })).text;
    }
    cancelAnimationFrame(raf);
    const elapsed = performance.now() - started;
    window.__terminalCompatLogs["session-a"] = events.slice(-600);
    return { text, elapsed, maxFrameGap: Math.max(...gaps), frames };
  });
  const lines = result.text.split("\n");
  for (let i = 0; i < result.frames; i++) {
    const ts = new Date(Date.UTC(2026, 10, 7) + i * 1000).toISOString().replace(".000Z", ".123456Z");
    const row = lines.find(line => line.endsWith(`TIMED-ROW-${i}`));
    assert(row?.includes(ts), `wrong timestamp on TIMED-ROW-${i}: ${row}`);
  }
  await page.waitForTimeout(2200);
  const afterPoll = await page.evaluate(async ({ sessionId, viewId }) => {
    const { requestTerminalTextExport } = await import("/src/terminal-export-event.ts");
    return (await requestTerminalTextExport({ sessionId, viewId, source: "buffer" })).text;
  }, { sessionId, viewId });
  assert.equal(afterPoll, result.text, "history polling changed timestamps or replayed live output");
  assert(result.elapsed < 3000, `canonical output drain took ${result.elapsed}ms`);
  assert(result.maxFrameGap < 500, `output blocked the UI for ${result.maxFrameGap}ms`);

  const clearStamp = "2026-11-07T01:00:00.654321Z";
  await page.evaluate(ts => window.__timestampBurst("clear-redraw", "\x1b[H\x1b[2JAFTER-CLEAR\r\n", ts), clearStamp);
  const host = page.locator('[data-pane-id="pane-a"] .terminal-host');
  await page.locator(`[data-pane-id="pane-a"] .terminal-timestamp-gutter time[datetime="${clearStamp}"]`).first().waitFor();
  const normalBeforeTui = await page.evaluate(async () => {
    const { requestTerminalTextExport } = await import("/src/terminal-export-event.ts");
    return (await requestTerminalTextExport({ sessionId: "session-a", viewId: "view-a", source: "buffer" })).text;
  });
  assert(normalBeforeTui.split("\n").find(line => line.endsWith("AFTER-CLEAR"))?.includes(clearStamp), "screen clearing retained an old timestamp");
  await page.evaluate(() => window.__timestampBurst("alt-enter", "\x1b[?1049hTUI-CONTENT", "2026-11-07T01:00:01.000000Z"));
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalBuffer === "alternate");
  await page.evaluate(() => window.__timestampBurst("alt-exit", "\x1b[?1049l", "2026-11-07T01:00:02.000000Z"));
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalBuffer === "normal");
  const normalAfterTui = await page.evaluate(async () => {
    const { requestTerminalTextExport } = await import("/src/terminal-export-event.ts");
    return (await requestTerminalTextExport({ sessionId: "session-a", viewId: "view-a", source: "buffer" })).text;
  });
  assert.equal(normalAfterTui, normalBeforeTui, "leaving a TUI changed normal-buffer timestamps");

  await host.locator(".xterm-helper-textarea").focus();
  await page.evaluate(() => {
    window.__invokeCalls = [];
    window.__perfStreamDone = false;
    let frame = 0;
    window.__perfStreamTimer = setInterval(() => {
      for (let i = 0; i < 20; i++, frame++) {
        window.__timestampBurst(`continuous-${frame}`, `CONTINUOUS-${frame}\r\n`, new Date(Date.UTC(2026, 10, 7, 2) + frame).toISOString());
      }
      if (frame >= 2000) { clearInterval(window.__perfStreamTimer); window.__perfStreamDone = true; }
    }, 16);
  });
  const inputStarted = Date.now();
  await page.keyboard.type("p");
  await page.waitForFunction(() => window.__invokeCalls.some(call => call.command === "send_text" && call.args.sessionId === "session-a" && call.args.text === "p"));
  const inputMilliseconds = Date.now() - inputStarted;
  assert.equal(await page.evaluate(() => window.__perfStreamDone), false, "input waited for ongoing output to finish");
  assert(inputMilliseconds < 500, `input stalled for ${inputMilliseconds}ms during output`);
  await page.waitForFunction(() => window.__perfStreamDone);
  console.log(JSON.stringify({ canonicalFrames: result.frames, milliseconds: Math.round(result.elapsed), maxFrameGap: Math.round(result.maxFrameGap), timestampsChecked: result.frames, pollingStable: true, clearAndTuiStable: true, inputDuringOutputMilliseconds: inputMilliseconds }));
}
