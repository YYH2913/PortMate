import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import { latencySummary } from "./latency-summary.mjs";

/** Sustained production-bundle RX/TX, burst recovery and >1,000 real key events. */
export async function checkTerminalTailLatency(page) {
  const durationSeconds = Number(process.env.PORTMATE_UI_TAIL_SECONDS ?? 60);
  assert(Number.isInteger(durationSeconds) && durationSeconds >= 60 && durationSeconds <= 600, "tail-latency duration must be 60–600 seconds");
  const pageErrors = [];
  page.on("pageerror", error => pageErrors.push(error.message));
  const pane = page.locator('[data-pane-id="pane-a"] .terminal-canvas');
  await pane.getByRole("button", { name: "文本", exact: true }).click();
  await pane.locator(".xterm-helper-textarea").focus();
  await page.evaluate(() => {
    let sequence = 0;
    const filler = "\r".repeat(32_768 - 96);
    window.__tailSend = (direction, burst = false) => {
      const id = sequence++;
      const text = direction === "inbound"
        ? `${filler}\r\x1b[32mTAIL-ROW-${id}\x1b[0m\r\n`
        : "x".repeat(32_768);
      const bytes = Array.from(new TextEncoder().encode(text));
      window.__emitTauriEvent("portmate-terminal-live", {
        event: { id: `tail-${id}`, sessionId: "session-a", paneId: "session-a:main", ts: new Date().toISOString(), direction,
          stream: direction === "inbound" ? "stdout" : "control", text, bytesRef: null, annotations: { origin: "interactive" } },
        bytes, originalLength: bytes.length, truncated: false,
      });
      if (window.__tailState) {
        window.__tailState.packets++;
        window.__tailState.bytes += bytes.length;
        if (burst) window.__tailState.burstPackets++;
        if (direction === "inbound") {
          window.__tailState.lastInboundId = id;
          window.__tailState.retainedInboundIds.push(id);
          if (window.__tailState.retainedInboundIds.length > 128) window.__tailState.retainedInboundIds.shift();
        }
      }
    };
    for (let index = 0; index < 64; index++) window.__tailSend(index % 2 ? "outbound" : "inbound");
  });
  await page.waitForTimeout(300);
  // Keep the byte capture full; screen clearing must not reset performance.
  await page.keyboard.press("Control+l");
  await page.evaluate(() => {
    const started = performance.now();
    const state = window.__tailState = { started, frames: [], inputFrames: [], dispatch: [], timerLag: [], packets: 0, bytes: 0,
      burstPackets: 0, bursts: 0, sentKeys: 0, deliveredKeys: 0, pendingKeyTimes: [], lastInboundId: -1, retainedInboundIds: [] };
    let lastFrame = started;
    const frame = () => {
      const now = performance.now();
      state.frames.push({ at: now - started, ms: now - lastFrame });
      lastFrame = now;
      state.raf = requestAnimationFrame(frame);
    };
    state.raf = requestAnimationFrame(frame);
    let deadline = started + 20;
    state.lagTimer = setInterval(() => {
      const now = performance.now();
      state.timerLag.push({ at: now - started, ms: Math.max(0, now - deadline) });
      deadline = now + 20;
    }, 20);
    state.keyListener = event => {
      if (event.key !== "p") return;
      const now = performance.now();
      state.sentKeys++;
      state.pendingKeyTimes.push(now);
      requestAnimationFrame(() => state.inputFrames.push({ at: performance.now() - started, ms: performance.now() - now }));
    };
    document.addEventListener("keydown", state.keyListener, true);
    const nativeInvoke = window.__TAURI_INTERNALS__.invoke;
    state.nativeInvoke = nativeInvoke;
    window.__TAURI_INTERNALS__.invoke = function(command, args, ...rest) {
      if (command === "send_text" && args?.sessionId === "session-a") {
        for (const character of args.text ?? "") if (character === "p") {
          const now = performance.now(), start = state.pendingKeyTimes.shift();
          state.deliveredKeys++;
          if (start !== undefined) state.dispatch.push({ at: now - started, ms: now - start });
        }
      }
      return nativeInvoke.call(this, command, args, ...rest);
    };
    state.streamTimer = setInterval(() => { window.__tailSend("inbound"); window.__tailSend("outbound"); }, 8);
    state.burstTimer = setInterval(() => {
      state.bursts++;
      // A 512 KiB transport burst among steady traffic exercises queue recovery.
      for (let index = 0; index < 16; index++) window.__tailSend(index % 2 ? "outbound" : "inbound", true);
    }, 2500);
    window.__tailStopTraffic = () => { clearInterval(state.streamTimer); clearInterval(state.burstTimer); };
    window.__tailFinish = () => {
      window.__tailStopTraffic();
      clearInterval(state.lagTimer); cancelAnimationFrame(state.raf);
      document.removeEventListener("keydown", state.keyListener, true);
      window.__TAURI_INTERNALS__.invoke = nativeInvoke;
      return { ...state, elapsed: performance.now() - started, nativeInvoke: undefined, keyListener: undefined };
    };
  });
  const cdp = await page.context().newCDPSession(page);
  const heapSamples = [];
  let result;
  let outputDrainMilliseconds = 0;
  let retainedOutputVerified = false;
  try {
    const started = Date.now();
    let nextHeapAt = 0, nextProgressAt = 15_000;
    while (Date.now() - started < durationSeconds * 1000) {
      await page.keyboard.press("p");
      await page.waitForTimeout(45);
      const elapsed = Date.now() - started;
      if (elapsed >= nextHeapAt) {
        const heap = await cdp.send("Runtime.getHeapUsage");
        heapSamples.push({ elapsed, heapBytes: heap.usedSize, backingBytes: heap.backingStorageSize ?? 0 });
        nextHeapAt += 10_000;
      }
      if (elapsed >= nextProgressAt) {
        console.log(`Tail-latency stress: ${Math.round(elapsed / 1000)}/${durationSeconds}s`);
        nextProgressAt += 15_000;
      }
    }
    await page.evaluate(() => window.__tailStopTraffic());
    const drainStarted = Date.now();
    await page.waitForFunction(() => window.__tailState.deliveredKeys === window.__tailState.sentKeys, undefined, { timeout: 2000 });
    await page.waitForFunction(() => window.__tailState.inputFrames.length === window.__tailState.sentKeys);
    await page.waitForFunction(async () => {
      const text = await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(Error("terminal export timed out")), 1000);
        window.dispatchEvent(new CustomEvent("portmate-terminal-text-export", { detail: {
          sessionId: "session-a", viewId: "view-a", source: "buffer", respond: response => {
            clearTimeout(timeout);
            if (response.ok) resolve(response.payload.text); else reject(Error(response.error));
          },
        } }));
      });
      return window.__tailState.retainedInboundIds.every(id => text.includes(`TAIL-ROW-${id}\n`));
    }, undefined, { timeout: 2000 });
    outputDrainMilliseconds = Date.now() - drainStarted;
    retainedOutputVerified = true;
    await page.waitForTimeout(100);
  } finally {
    result = await page.evaluate(() => window.__tailFinish());
    await cdp.detach();
  }
  const summarize = samples => latencySummary(samples.map(sample => sample.ms));
  const windowSummary = (samples, start, end) => summarize(samples.filter(sample => sample.at >= start && sample.at < end));
  const third = result.elapsed / 3;
  const summary = {
    durationSeconds: Math.round(result.elapsed / 1000),
    frames: summarize(result.frames), inputNextFrame: summarize(result.inputFrames), inputDispatch: summarize(result.dispatch),
    eventLoopLag: summarize(result.timerLag),
    frameTimeBeyond50Milliseconds: Math.round(result.frames.reduce((total, sample) => total + Math.max(0, sample.ms - 50), 0)),
    firstThird: { frames: windowSummary(result.frames, 0, third), input: windowSummary(result.inputFrames, 0, third) },
    lastThird: { frames: windowSummary(result.frames, third * 2, result.elapsed), input: windowSummary(result.inputFrames, third * 2, result.elapsed) },
    sentKeys: result.sentKeys, deliveredKeys: result.deliveredKeys, pendingKeys: result.pendingKeyTimes.length,
    packets: result.packets, totalMiB: Math.round(result.bytes / 1024 / 1024), bursts: result.bursts,
    outputDrainMilliseconds, retainedOutputVerified,
    heapSamples: heapSamples.map(sample => ({ seconds: Math.round(sample.elapsed / 1000), heapMiB: Math.round(sample.heapBytes / 1024 / 1024), backingMiB: Math.round(sample.backingBytes / 1024 / 1024) })),
    sampleScope: "Chromium production frontend with mocked native IPC; dispatch/next-animation-frame, not device echo or display presentation",
  };
  console.log(JSON.stringify({ tailLatency: summary }));
  if (process.env.PORTMATE_UI_TAIL_REPORT) writeFileSync(process.env.PORTMATE_UI_TAIL_REPORT, `${JSON.stringify(summary, null, 2)}\n`);
  assert.equal(pageErrors.length, 0, `browser errors under sustained RX/TX: ${JSON.stringify(pageErrors)}`);
  assert(summary.sentKeys >= 1000, `too few real key samples to report a useful P99: ${summary.sentKeys}`);
  assert.equal(summary.deliveredKeys, summary.sentKeys, "input was lost under sustained traffic");
  assert.equal(summary.pendingKeys, 0, "input remained queued after traffic stopped");
  assert.equal(summary.inputNextFrame.samples, summary.sentKeys, "some keys never received an animation-frame opportunity");
  assert(summary.frames.samples >= 2000, "too few frame samples for tail-latency validation");
  assert(summary.frames.p99 <= 50, `frame P99 exceeded 50 ms: ${JSON.stringify(summary.frames)}`);
  assert(summary.frames.max < 250, `UI froze for 250 ms or longer: ${JSON.stringify(summary.frames)}`);
  assert(summary.inputNextFrame.p99 <= 50, `input next-frame P99 exceeded 50 ms: ${JSON.stringify(summary.inputNextFrame)}`);
  assert(summary.inputDispatch.p99 <= 50, `input dispatch P99 exceeded 50 ms: ${JSON.stringify(summary.inputDispatch)}`);
  assert(summary.eventLoopLag.p99 <= 50, `event-loop P99 exceeded 50 ms: ${JSON.stringify(summary.eventLoopLag)}`);
  assert(summary.lastThird.frames.p99 <= Math.max(25, summary.firstThird.frames.p99 * 1.5), "tail latency degraded with capture age");
  assert(summary.outputDrainMilliseconds < 2000 && summary.retainedOutputVerified, "output did not recover completely after the load stopped");
  assert(Math.max(...heapSamples.map(sample => sample.heapBytes)) < 96 * 1024 * 1024, "sustained capture retained excessive JS heap");
}
