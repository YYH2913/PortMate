import assert from "node:assert/strict";

const sessionId = "session-a";
const viewId = "view-a";

function event(id, text, ts = "2026-09-05T09:00:00.000000Z") {
  return { id, sessionId, paneId: `${sessionId}:main`, ts, direction: "inbound", stream: "stdout", text, bytesRef: null, annotations: {} };
}

async function emit(page, value) {
  await page.evaluate((event) => {
    const bytes = [...new TextEncoder().encode(event.text)];
    window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
  }, value);
  await page.waitForTimeout(60);
}

async function readBuffer(page) {
  return page.evaluate(async ({ sessionId, viewId }) => {
    const { requestTerminalTextExport } = await import("/src/terminal-export-event.ts");
    return (await requestTerminalTextExport({ sessionId, viewId, source: "buffer" })).text;
  }, { sessionId, viewId });
}

async function pollHistory(page, events) {
  await page.evaluate(({ sessionId, events }) => {
    window.__terminalCompatLogs[sessionId] = events;
    window.__terminalCompatTailId = null;
  }, { sessionId, events });
  await page.waitForFunction((id) => window.__terminalCompatTailId === id, events.at(-1).id);
  await page.waitForTimeout(120);
}

export async function checkTerminalStreamRegressions(page, screenshotPrefix) {
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.locator('[data-pane-id="pane-a"] [data-view-id="view-a"] [role="tab"]').click();
  await page.waitForTimeout(120);
  await emit(page, event("replay-clear", "\x1b[?1049l\x1b[2J\x1b[3J\x1b[H"));
  const history = Array.from({ length: 6 }, (_, index) => event(
    `replay-history-${index}`, `OLD-HISTORY-${index}\r\n`, `2026-09-05T09:00:0${index}.000000Z`,
  ));
  for (const entry of history) await emit(page, entry);
  await pollHistory(page, history);

  await page.evaluate(({ sessionId }) => {
    for (let index = 0; index < 4_105; index += 1) {
      const event = {
        id: `replay-burst-${index}`, sessionId, paneId: `${sessionId}:main`,
        ts: "2026-09-05T09:01:00.000000Z", direction: "inbound", stream: "stdout",
        text: "x\b", bytesRef: null, annotations: {},
      };
      window.__emitTauriEvent("portmate-terminal-live", { event, bytes: [120, 8], originalLength: 2, truncated: false });
    }
  }, { sessionId });
  const prompt = event("replay-live-prompt", "LIVE-PROMPT> ", "2026-09-05T09:02:00.000000Z");
  await emit(page, prompt);
  const before = await readBuffer(page);
  await pollHistory(page, [...history.slice(1), prompt]);
  const after = await readBuffer(page);
  assert.equal(after, before, "a shifted polling window replayed old output after the live dedupe cache rolled over");

  await page.locator('[data-pane-id="pane-a"] [data-view-id="view-b"] [role="tab"]').click();
  await page.setViewportSize({ width: 1200, height: 820 });
  const hiddenOutput = event("replay-hidden-output", "\r\nHIDDEN-OUTPUT> ", "2026-09-05T07:03:00.000000Z");
  await emit(page, hiddenOutput);
  await page.locator('[data-pane-id="pane-a"] [data-view-id="view-a"] [role="tab"]').click();
  await page.waitForTimeout(150);
  const restored = await readBuffer(page);
  assert.equal((restored.match(/OLD-HISTORY-/g) ?? []).length, 6, restored);
  assert.equal((restored.match(/LIVE-PROMPT>/g) ?? []).length, 1, restored);
  assert.equal((restored.match(/HIDDEN-OUTPUT>/g) ?? []).length, 1, restored);
  await pollHistory(page, [event("late-unseen-history", "\x1b[HSTALE-PROMPT", "2026-09-05T09:00:00.000000Z")]);
  assert.equal(await readBuffer(page), restored, "late history moved the restored live cursor");
  const fallback = event("poll-fallback", "POLL-FALLBACK\r\n", "2026-09-05T09:04:00.000000Z");
  await pollHistory(page, [fallback]);
  assert.match(await readBuffer(page), /POLL-FALLBACK/);

  await emit(page, event("timestamp-reset", "\x1b[2J\x1b[3J\x1b[H"));
  for (let index = 0; index < 4; index += 1) {
    await emit(page, event(`timestamp-old-${index}`, `OLD-ROW-${index}\r\n`, `2026-09-05T05:00:0${index}.000000Z`));
  }
  const redrawTime = "2026-09-05T17:54:39.123456Z";
  await emit(page, event("timestamp-redraw", "\x1b[HNEW-ROW-0\r\nNEW-ROW-1\r\nNEW-ROW-2\r\nNEW-ROW-3\r\nCURRENT> ", redrawTime));
  const redrawn = await readBuffer(page);
  await page.screenshot({ path: `${screenshotPrefix}-stream-regressions.png` });

  const newRows = redrawn.split("\n").filter((line) => /NEW-ROW-|CURRENT>/.test(line));
  assert.equal(newRows.length, 5, redrawn);
  assert.ok(newRows.every((line) => line.includes(redrawTime)), `redrawn rows retained stale timestamps:\n${redrawn}`);
  await emit(page, event("timestamp-prompt-typing", "status", "2026-09-05T17:54:40.000000Z"));
  assert.ok((await readBuffer(page)).split("\n").find((line) => line.includes("CURRENT> status"))?.includes(redrawTime));

  const partialTime = "2026-09-05T17:55:00.654321Z";
  await emit(page, event("timestamp-partial-prefix", "\x1b[", partialTime));
  await emit(page, event("timestamp-partial", "2;1H\x1b[2KCHANGED-ROW", partialTime));
  const partial = (await readBuffer(page)).split("\n");
  assert.ok(partial.find((line) => line.includes("CHANGED-ROW"))?.includes(partialTime), partial.join("\n"));
  assert.ok(partial.filter((line) => /NEW-ROW-|CURRENT>/.test(line)).every((line) => line.includes(redrawTime)), partial.join("\n"));

  await emit(page, event("timestamp-cursor-only", "\x1b[20;1H", "2026-09-05T17:56:00.000000Z"));
  const cursorOnlyLabels = await page.locator('[data-pane-id="pane-a"] .terminal-timestamp-gutter time').evaluateAll((labels) => (
    labels.map((label) => label.getAttribute("datetime"))
  ));
  assert.ok(!cursorOnlyLabels.includes("2026-09-05T17:56:00.000000Z"), JSON.stringify(cursorOnlyLabels));
  const clearTime = "2026-09-05T17:57:00.000000Z";
  await emit(page, event("timestamp-clear-screen", "\x1b[2J\x1b[HFRESH-PROMPT> ", clearTime));
  const cleared = await readBuffer(page);
  assert.ok(cleared.includes(clearTime) && cleared.includes("FRESH-PROMPT> ") && !cleared.includes("NEW-ROW"), cleared);
  return { shiftedHistoryStable: true, restoredReplayStable: true, pollingFallback: true, redrawTimestamps: true, partialRedraw: true, cursorOnly: true, clearScreen: true };
}
