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

  // Interactive shell regression 1: plain text arriving after a clear/blank row must
  // replace the stale clear timestamp and immediately render in the gutter.
  const blankClearTime = "2026-09-05T17:58:00.000000Z";
  const postClearPromptTime = "2026-09-05T17:58:05.123456Z";
  await emit(page, event("timestamp-clear-only", "\x1b[2J\x1b[3J\x1b[H", blankClearTime));
  await emit(page, event("timestamp-post-clear-plain", "user@host:~$ ", postClearPromptTime));
  await page.locator(`[data-pane-id="pane-a"] .terminal-timestamp-gutter time[datetime="${postClearPromptTime}"]`).first().waitFor({ timeout: 2000 });
  const postClearBuffer = await readBuffer(page);
  assert.ok(postClearBuffer.includes(`[${postClearPromptTime}] user@host:~$`), postClearBuffer);

  // Interactive shell regression 2: multi-line output + prompt ending with a late CSI (\x1b[K)
  // must timestamp every newly written pre-CSI row immediately.
  const lateCsiTime = "2026-09-05T17:58:10.234567Z";
  await emit(page, event("timestamp-late-csi", "\r\nLATE-CSI-1\r\nLATE-CSI-2\r\nuser@host:~$ \x1b[K", lateCsiTime));
  await page.locator(`[data-pane-id="pane-a"] .terminal-timestamp-gutter time[datetime="${lateCsiTime}"]`).first().waitFor({ timeout: 2000 });
  const lateCsiBuffer = await readBuffer(page);
  for (const label of ["LATE-CSI-1", "LATE-CSI-2"]) {
    assert.ok(lateCsiBuffer.split("\n").find((line) => line.includes(label))?.includes(lateCsiTime), lateCsiBuffer);
  }
  assert.ok(lateCsiBuffer.split("\n").find((line) => line.includes(`[${postClearPromptTime}] user@host:~$`)), lateCsiBuffer);

  // Interactive shell regression 3: leading blank lines (\r\n\r\nOUTPUT) must timestamp
  // the first non-empty output line rather than stopping on the blank line.
  const afterBlankTime = "2026-09-05T17:58:15.345678Z";
  await emit(page, event("timestamp-after-blank", "\r\n\r\nAFTER-BLANK-LINE\r\n", afterBlankTime));
  await page.locator(`[data-pane-id="pane-a"] .terminal-timestamp-gutter time[datetime="${afterBlankTime}"]`).first().waitFor({ timeout: 2000 });
  const afterBlankBuffer = await readBuffer(page);
  assert.ok(afterBlankBuffer.split("\n").find((line) => line.includes("AFTER-BLANK-LINE"))?.includes(afterBlankTime), afterBlankBuffer);

  // Interactive shell regression 4: carriage-return in-place overwrite (\r without \x1b[K)
  // must update the line's timestamp while ordinary prompt typing still keeps the original stamp.
  const progressStartTime = "2026-09-05T17:58:20.000000Z";
  const progressUpdateTime = "2026-09-05T17:58:25.456789Z";
  await emit(page, event("timestamp-progress-start", "Progress: 10%", progressStartTime));
  await emit(page, event("timestamp-progress-update", "\rProgress: 50%", progressUpdateTime));
  await page.locator(`[data-pane-id="pane-a"] .terminal-timestamp-gutter time[datetime="${progressUpdateTime}"]`).first().waitFor({ timeout: 2000 });
  const progressBuffer = await readBuffer(page);
  assert.ok(progressBuffer.split("\n").find((line) => line.includes("Progress: 50%"))?.includes(progressUpdateTime), progressBuffer);
  assert.ok(progressBuffer.split("\n").find((line) => line.includes("AFTER-BLANK-LINE"))?.includes(afterBlankTime), progressBuffer);

  // Interactive shell regression 5: CSI packet scrolling past the bottom of a full viewport
  // must timestamp every newly scrolled row immediately.
  const fillTime = "2026-09-05T17:58:30.000000Z";
  const fillPayload = "\r\n" + Array.from({ length: 45 }, (_, index) => `FILL-ROW-${index}`).join("\r\n");
  await emit(page, event("timestamp-fill-viewport", fillPayload, fillTime));
  const csiScrollTime = "2026-09-05T17:58:35.567890Z";
  await emit(page, event("timestamp-csi-scroll", "\x1b[K\r\nSCROLLED-CSI-1\r\nSCROLLED-CSI-2\r\n", csiScrollTime));
  await page.locator(`[data-pane-id="pane-a"] .terminal-timestamp-gutter time[datetime="${csiScrollTime}"]`).first().waitFor({ timeout: 2000 });
  const csiScrollBuffer = await readBuffer(page);
  for (const label of ["SCROLLED-CSI-1", "SCROLLED-CSI-2"]) {
    assert.ok(csiScrollBuffer.split("\n").find((line) => line.includes(label))?.includes(csiScrollTime), csiScrollBuffer);
  }
  assert.ok(csiScrollBuffer.split("\n").find((line) => line.includes("FILL-ROW-44"))?.includes(fillTime), csiScrollBuffer);

  // Interactive shell regression 6: split UTF-8 raw bytes with empty decoded preview on frame 1
  // must timestamp the completed character on arrival of the final byte.
  const utf8LeadTime = "2026-09-05T17:58:40.000000Z";
  const utf8TrailTime = "2026-09-05T17:58:42.678901Z";
  await page.evaluate(({ sessionId, utf8LeadTime, utf8TrailTime }) => {
    window.__emitTauriEvent("portmate-terminal-live", {
      event: { id: "utf8-lead", sessionId, paneId: `${sessionId}:main`, ts: utf8LeadTime, direction: "inbound", stream: "stdout", text: "", bytesRef: null, annotations: {} },
      bytes: [0xe4, 0xb8], originalLength: 2, truncated: false,
    });
    window.__emitTauriEvent("portmate-terminal-live", {
      event: { id: "utf8-trail", sessionId, paneId: `${sessionId}:main`, ts: utf8TrailTime, direction: "inbound", stream: "stdout", text: "", bytesRef: null, annotations: {} },
      bytes: [0xad, 0x0d, 0x0a], originalLength: 3, truncated: false,
    });
  }, { sessionId, utf8LeadTime, utf8TrailTime });
  await page.locator(`[data-pane-id="pane-a"] .terminal-timestamp-gutter time[datetime="${utf8TrailTime}"]`).first().waitFor({ timeout: 2000 });
  const utf8Buffer = await readBuffer(page);
  assert.ok(utf8Buffer.split("\n").find((line) => line.includes("中"))?.includes(utf8TrailTime), utf8Buffer);

  // A line feed into an existing row moves the cursor without rewriting content.
  const originalRowTime = "2026-09-05T17:59:10.000000Z";
  await emit(page, event("timestamp-navigation-reset", "\x1b[2J\x1b[3J\x1b[H"));
  await emit(page, event("timestamp-navigation-first", "NAV-ROW-1\r\n", "2026-09-05T17:59:00.000000Z"));
  await emit(page, event("timestamp-navigation-second", "NAV-ROW-2", originalRowTime));
  await emit(page, event("timestamp-navigation-home", "\x1b[H", "2026-09-05T17:59:11.000000Z"));
  await emit(page, event("timestamp-navigation-lf", "\r\n", "2026-09-05T17:59:12.000000Z"));
  const navigationBuffer = await readBuffer(page);
  assert.ok(navigationBuffer.split("\n").find(line => line.includes("NAV-ROW-2"))?.includes(originalRowTime), navigationBuffer);

  return {
    shiftedHistoryStable: true,
    restoredReplayStable: true,
    pollingFallback: true,
    redrawTimestamps: true,
    partialRedraw: true,
    cursorOnly: true,
    clearScreen: true,
    interactiveTimestamps: true,
  };
}
