import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import { latencySummary } from "./latency-summary.mjs";

/** Full live capture plus sustained RX/TX; clearing the terminal must not be needed. */
export async function checkTerminalByteLoad(page) {
  const pane = page.locator('[data-pane-id="pane-a"] .terminal-canvas');
  await page.evaluate(() => {
    let sequence = 0;
    window.__byteLoadSend = (direction, size = 16_384) => {
      const id = `byte-load-${sequence++}`;
      const bytes = Array(size).fill(direction === "inbound" ? 13 : 120);
      const event = { id, sessionId: "session-a", paneId: "session-a:main", ts: new Date().toISOString(), direction,
        stream: direction === "inbound" ? "stdout" : "control", text: direction === "inbound" ? "\r".repeat(size) : "x".repeat(size), bytesRef: null, annotations: { origin: "interactive" } };
      window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: size, truncated: false });
    };
    for (let index = 0; index < 64; index++) window.__byteLoadSend(index % 2 ? "outbound" : "inbound");
  });
  await page.waitForTimeout(250);
  const results = [];
  const modes = process.env.PORTMATE_UI_TEXT_RXTX_ONLY === "1" ? [["text", "文本"]] : [["text", "文本"], ["hex", "Hex"], ["split", "对照"]];
  for (const [mode, label] of modes) {
    const opening = Date.now();
    await pane.getByRole("button", { name: label, exact: true }).click();
    if (mode !== "text") await pane.locator(".terminal-byte-scroll").waitFor();
    const openMilliseconds = Date.now() - opening;
    let profiler;
    if (mode === "hex" && process.env.PORTMATE_TERMINAL_CPU_PROFILE) {
      profiler = await page.context().newCDPSession(page);
      await profiler.send("Profiler.enable");
      await profiler.send("Profiler.start");
    }
    if (mode === "hex") await pane.locator(".terminal-byte-scroll").focus();
    else await pane.locator(".xterm-helper-textarea").focus();
    if (mode === "text") {
      // This clears only the terminal screen, never RX/TX records, matching the
      // reported symptom. Full capture remains in memory during the check.
      await page.keyboard.press("Control+l");
    }
    await page.evaluate(({ interval, size }) => {
      const started = performance.now();
      window.__byteLoadGaps = [];
      window.__byteLoadInputFrames = [];
      window.__byteLoadPackets = 0;
      window.__byteStatsUpdates = 0;
      window.__byteAddedRows = 0;
      const rowContainer = document.querySelector('[data-pane-id="pane-a"] .terminal-byte-content');
      const rows = new MutationObserver(records => {
        for (const record of records) for (const node of record.addedNodes) {
          if (node instanceof Element && node.classList.contains("terminal-byte-row")) window.__byteAddedRows++;
        }
      });
      if (rowContainer) rows.observe(rowContainer, { childList: true });
      const stats = new MutationObserver(() => window.__byteStatsUpdates++);
      stats.observe(document.querySelector('[data-pane-id="pane-a"] .terminal-byte-summary'), { attributes: true, childList: true, subtree: true, characterData: true });
      let last = performance.now();
      const tick = () => { const now = performance.now(); window.__byteLoadGaps.push(now - last); last = now; window.__byteLoadRaf = requestAnimationFrame(tick); };
      window.__byteLoadRaf = requestAnimationFrame(tick);
      const key = event => {
        if (event.key !== "p" && event.key !== "ArrowDown") return;
        const at = performance.now();
        requestAnimationFrame(() => window.__byteLoadInputFrames.push(performance.now() - at));
      };
      document.addEventListener("keydown", key, true);
      window.__byteLoadTimer = setInterval(() => {
        window.__byteLoadSend("inbound", size); window.__byteLoadSend("outbound", size);
        window.__byteLoadPackets += 2;
      }, interval);
      window.__byteLoadStop = () => {
        clearInterval(window.__byteLoadTimer); cancelAnimationFrame(window.__byteLoadRaf);
        stats.disconnect();
        rows.disconnect();
        document.removeEventListener("keydown", key, true);
        return { gaps: window.__byteLoadGaps, inputFrames: window.__byteLoadInputFrames, packets: window.__byteLoadPackets, statsUpdates: window.__byteStatsUpdates, addedRows: window.__byteAddedRows, elapsed: performance.now() - started };
      };
    }, { interval: process.env.PORTMATE_UI_TEXT_RXTX_ONLY === "1" ? 8 : 33, size: process.env.PORTMATE_UI_TEXT_RXTX_ONLY === "1" ? 32_768 : 16_384 });
    let result;
    try {
      for (let key = 0; key < 15; key++) {
        await page.keyboard.press(mode === "hex" ? "ArrowDown" : "p");
        await page.waitForTimeout(100);
      }
    } finally {
      result = await page.evaluate(() => window.__byteLoadStop());
      if (profiler) {
        const { profile } = await profiler.send("Profiler.stop");
        writeFileSync(process.env.PORTMATE_TERMINAL_CPU_PROFILE, JSON.stringify(profile));
        await profiler.detach();
      }
    }
    const frameLatency = latencySummary(result.gaps);
    const inputFrameLatency = latencySummary(result.inputFrames);
    const rowCount = mode === "text" ? 0 : Number(await pane.locator(".terminal-byte-scroll").getAttribute("aria-rowcount"));
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("HeapProfiler.collectGarbage");
    const heap = await cdp.send("Runtime.getHeapUsage");
    await cdp.detach();
    results.push({ mode, openMilliseconds, frameP95Milliseconds: frameLatency.p95, frameP99Milliseconds: frameLatency.p99, maxFrameGapMilliseconds: frameLatency.max, inputFrameP95Milliseconds: inputFrameLatency.p95, inputFrameP99Milliseconds: inputFrameLatency.p99, frameLatency, inputFrameLatency, keys: result.inputFrames.length, packets: result.packets, logicalRows: rowCount, renderedRows: await pane.locator(".terminal-byte-row").count(), addedRows: result.addedRows, retainedHeapMiB: Math.round(heap.usedSize / 1024 / 1024), byteArrayBuffersMiB: Math.round((heap.backingStorageSize ?? 0) / 1024 / 1024), statsUpdates: result.statsUpdates, elapsed: Math.round(result.elapsed) });
  }
  console.log(JSON.stringify({ fullRxTxCapture: results }));
  for (const result of results) {
    assert(result.openMilliseconds < 750, `${result.mode} opened slowly with a full byte cache`);
    assert(result.frameP95Milliseconds < 75, `${result.mode} UI stalled with full RX/TX: ${JSON.stringify(result)}`);
    assert(result.frameP99Milliseconds < (result.mode === "text" ? 50 : 100), `${result.mode} P99 frame interval exceeded its budget: ${JSON.stringify(result)}`);
    assert(result.maxFrameGapMilliseconds < 250, `${result.mode} froze while capture was full: ${JSON.stringify(result)}`);
    assert.equal(result.keys, 15, "input did not remain live during byte capture");
    assert(result.inputFrameP95Milliseconds < 75, "keyboard handling waited behind the capture UI");
    assert(result.inputFrameP99Milliseconds < 100, "keyboard P99 frame feedback stalled");
    assert(result.packets >= 50, "RX/TX capture could not continue at the requested rate");
    assert(result.statsUpdates <= Math.ceil(result.elapsed / 100) + 1, "auxiliary RX/TX counters refreshed at transport rate");
    assert(result.renderedRows < 100, "byte inspector mounted the full retained capture");
    assert(result.addedRows <= result.renderedRows * 2, "capture updates recreated the visible row DOM instead of reusing slots");
  }
  if (process.env.PORTMATE_UI_TEXT_RXTX_ONLY !== "1") {
    await pane.getByRole("button", { name: "Hex", exact: true }).click();
    // Use a visible slot, not the overscan rows above the scroll viewport:
    // Playwright would scroll those into view and change the slot's contents.
    const row = pane.locator(".terminal-byte-row").nth(8);
    await row.waitFor();
    const oldId = await row.getAttribute("data-frame-id");
    await row.evaluate(element => { window.__reusedByteRow = element; });
    await page.evaluate(() => { window.__byteLoadSend("inbound"); window.__byteLoadSend("outbound"); });
    await page.waitForFunction(id => {
      const rows = document.querySelectorAll('[data-pane-id="pane-a"] .terminal-byte-row');
      return rows[8]?.getAttribute("data-frame-id") !== id;
    }, oldId);
    assert(await row.evaluate(element => window.__reusedByteRow === element), "capture refresh remounted a viewport slot");
    const selectedFrame = await row.getAttribute("data-frame-id");
    const selectedFrameOffset = await row.getAttribute("data-frame-byte-offset");
    const offset = Number(await row.getAttribute("data-row-offset"));
    const hexCell = row.locator('[data-byte-cell="3"][data-byte-column="hex"]');
    assert((await hexCell.getAttribute("aria-label")).includes(String(offset + 3)), "reused row kept an old accessible byte offset");
    await hexCell.click();
    const selectedRow = pane.locator(`[data-frame-id="${selectedFrame}"][data-frame-byte-offset="${selectedFrameOffset}"]`);
    const paired = selectedRow.locator('[data-byte-cell="3"]');
    await page.waitForFunction(({ frameId, frameOffset }) => {
      const row = [...document.querySelectorAll('[data-pane-id="pane-a"] .terminal-byte-row')].find(row => row.dataset.frameId === frameId && row.dataset.frameByteOffset === frameOffset);
      const cells = [...(row?.querySelectorAll('[data-byte-cell="3"]') ?? [])];
      return cells.length === 2 && cells.every(cell => cell.classList.contains("selected") && cell.getAttribute("aria-pressed") === "true");
    }, { frameId: selectedFrame, frameOffset: selectedFrameOffset });
    assert.equal(await paired.count(), 2, "reused row selected bytes from a previous capture frame");
    await selectedRow.locator('[data-byte-cell="4"][data-byte-column="ascii"]').hover();
    assert(await selectedRow.locator('[data-byte-cell="4"]').evaluateAll(cells => cells.length === 2 && cells.every(cell => cell.classList.contains("linked"))), "reused row lost Hex/ASCII hover correspondence");
  }
  // Clearing raw records remains explicit and immediate; it must not clear the
  // terminal's text or let a deferred statistics update resurrect old data.
  await pane.getByRole("button", { name: "文本", exact: true }).click();
  await page.evaluate(() => {
    const text = "\r\nBYTE-CAPTURE-KEEPS-TERMINAL\r\n";
    const bytes = Array.from(new TextEncoder().encode(text));
    window.__emitTauriEvent("portmate-terminal-live", { event: { id: "byte-clear-sentinel", sessionId: "session-a", paneId: "session-a:main", ts: new Date().toISOString(), direction: "inbound", stream: "stdout", text, bytesRef: null, annotations: {} }, bytes, originalLength: bytes.length, truncated: false });
  });
  await page.waitForTimeout(150);
  const beforeClear = await exportTerminal(page);
  assert(beforeClear.includes("BYTE-CAPTURE-KEEPS-TERMINAL"));
  await pane.locator('.terminal-byte-tool[aria-label="清空实时字节"]').click();
  await page.waitForTimeout(150);
  assert.equal((await pane.locator(".terminal-byte-summary").innerText()).trim(), "RX 0 B\nTX 0 B");
  assert(await pane.locator(".terminal-host .xterm-screen").count() === 1, "clearing byte records replaced the terminal");
  assert.equal(await exportTerminal(page), beforeClear, "clearing RX/TX records changed terminal text/history");
}

function exportTerminal(page) {
  return page.evaluate(() => new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(Error("terminal export timed out")), 2000);
    window.dispatchEvent(new CustomEvent("portmate-terminal-text-export", { detail: {
      sessionId: "session-a", viewId: "view-a", source: "buffer", respond: response => {
        clearTimeout(timeout);
        if (response.ok) resolve(response.payload.text); else reject(Error(response.error));
      },
    } }));
  }));
}
