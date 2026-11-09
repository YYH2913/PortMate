import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import { latencySummary } from "./latency-summary.mjs";

/** Sustained output in both visible panes, with real xterm keyboard handling. */
export async function checkTerminalFluidity(page) {
  await page.evaluate(() => {
    window.__fluidSend = (id, sessionId, text, ts, direction = "inbound") => {
      const event = { id, sessionId, paneId: `${sessionId}:main`, ts, direction, stream: direction === "inbound" ? "stdout" : "control", text, bytesRef: null, annotations: { origin: "interactive" } };
      const bytes = Array.from(new TextEncoder().encode(text));
      window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
    };
    for (const id of ["session-a", "session-b"]) {
      window.__fluidSend(`fluid-enter-${id}`, id, "\x1b[?1049h", "2026-11-07T03:00:00.000000Z");
    }
  });
  await page.waitForFunction(() => [...document.querySelectorAll(".terminal-host")].every(host => host.dataset.terminalBuffer === "alternate"));
  await page.locator('[data-pane-id="pane-a"] .xterm-helper-textarea').focus();
  await page.waitForTimeout(300);
  let profiler;
  if (process.env.PORTMATE_TERMINAL_CPU_PROFILE) {
    profiler = await page.context().newCDPSession(page);
    await profiler.send("Profiler.enable");
    await profiler.send("Profiler.start");
    await profiler.send("Tracing.start", { categories: "devtools.timeline,disabled-by-default-devtools.timeline", transferMode: "ReturnAsStream" });
  }
  await page.evaluate(() => {
    const nativeInvoke = window.__TAURI_INTERNALS__.invoke;
    const nativeBounds = Element.prototype.getBoundingClientRect;
    const key = event => { if (event.key === "x") window.__fluidLastKey = performance.now(); };
    document.addEventListener("keydown", key, true);
    window.__fluidInputs = [];
    window.__fluidLayouts = 0;
    window.__fluidFrames = 0;
    window.__fluidTxPackets = 0;
    window.__fluidGaps = [];
    window.__TAURI_INTERNALS__.invoke = function(command, args, ...rest) {
      if (command === "send_text" && args?.sessionId === "session-a" && args.text.includes("x")) {
        window.__fluidInputs.push(performance.now() - window.__fluidLastKey);
      }
      return nativeInvoke.call(this, command, args, ...rest);
    };
    Element.prototype.getBoundingClientRect = function(...args) {
      if (this.classList.contains("terminal-host") || this.classList.contains("xterm-screen")) window.__fluidLayouts++;
      return nativeBounds.apply(this, args);
    };
    let last = performance.now();
    const tick = () => {
      const now = performance.now();
      window.__fluidGaps.push(now - last);
      last = now;
      window.__fluidRaf = requestAnimationFrame(tick);
    };
    window.__fluidRaf = requestAnimationFrame(tick);
    const sizes = [...document.querySelectorAll(".terminal-host")].map(host => host.dataset.terminalSize.split("x").map(Number));
    window.__fluidTimer = setInterval(() => {
      const frame = window.__fluidFrames++;
      const ts = new Date(Date.UTC(2026, 10, 7, 3) + frame * 33 + 1).toISOString();
      for (const [index, id] of ["session-a", "session-b"].entries()) {
        const [cols, rows] = sizes[index];
        let text = "\x1b[H";
        for (let row = 0; row < rows; row++) {
          text += `\x1b[${row + 1};1H\x1b[${31 + row % 6}m${`${frame} CPU ${row} ${"abcdefghij".repeat(20)}`.slice(0, cols - 1)}\x1b[0m`;
        }
        window.__fluidSend(`fluid-${id}-${frame}`, id, text, ts);
      }
    }, 33);
    const txPayload = "x".repeat(32_768);
    window.__fluidTxTimer = setInterval(() => {
      const sequence = window.__fluidTxPackets++;
      window.__fluidSend(`fluid-tx-${sequence}`, "session-a", txPayload, new Date(Date.UTC(2026, 10, 7, 3) + sequence).toISOString(), "outbound");
    }, 8);
    window.__fluidStop = () => {
      clearInterval(window.__fluidTimer);
      clearInterval(window.__fluidTxTimer);
      cancelAnimationFrame(window.__fluidRaf);
      document.removeEventListener("keydown", key, true);
      window.__TAURI_INTERNALS__.invoke = nativeInvoke;
      Element.prototype.getBoundingClientRect = nativeBounds;
      return { frames: window.__fluidFrames, txPackets: window.__fluidTxPackets, gaps: window.__fluidGaps, inputs: window.__fluidInputs, geometryReads: window.__fluidLayouts,
        renderers: [...document.querySelectorAll(".terminal-host")].map(host => ({ renderer: host.dataset.terminalRenderer, webgl: host.dataset.terminalWebgl })) };
    };
  });
  let result;
  try {
    for (let index = 0; index < 20; index++) {
      await page.keyboard.type("x");
      await page.waitForTimeout(100);
    }
    await page.waitForFunction(() => window.__fluidInputs.length === 20);
  } finally {
    result = await page.evaluate(() => window.__fluidStop());
    if (profiler) {
      const { profile } = await profiler.send("Profiler.stop");
      writeFileSync(process.env.PORTMATE_TERMINAL_CPU_PROFILE, JSON.stringify(profile));
      const traceReady = new Promise(resolve => profiler.once("Tracing.tracingComplete", resolve));
      await profiler.send("Tracing.end");
      const { stream } = await traceReady;
      let trace = "";
      while (true) {
        const chunk = await profiler.send("IO.read", { handle: stream });
        trace += chunk.base64Encoded ? Buffer.from(chunk.data, "base64").toString("utf8") : chunk.data;
        if (chunk.eof) break;
      }
      await profiler.send("IO.close", { handle: stream });
      writeFileSync(`${process.env.PORTMATE_TERMINAL_CPU_PROFILE}.trace.json`, trace);
      await profiler.detach();
    }
  }
  const frameLatency = latencySummary(result.gaps), inputLatency = latencySummary(result.inputs);
  const frameP95 = frameLatency.p95, inputP95 = inputLatency.p95;
  console.log(JSON.stringify({ dualPaneTuiFrames: result.frames, simultaneousTxMiB: Math.round(result.txPackets * 32768 / 1024 / 1024), frameP95Milliseconds: frameP95, frameP99Milliseconds: frameLatency.p99, maxFrameGapMilliseconds: frameLatency.max, inputP95Milliseconds: inputP95, inputP99Milliseconds: inputLatency.p99, frameLatency, inputLatency, keysDelivered: result.inputs.length, geometryReads: result.geometryReads, renderers: result.renderers }));
  assert(result.frames >= 40, "sustained TUI output did not continue during keyboard input");
  assert(result.txPackets >= 150, "TX stream stalled while the TUI was rendering");
  assert.equal(result.inputs.length, 20, "keyboard input was lost during output");
  assert(frameP95 < 75, `TUI output stalled animation frames: p95=${frameP95}ms`);
  assert(inputP95 < 75, `TUI output stalled keyboard dispatch: p95=${inputP95}ms`);
  assert(frameLatency.p99 < 75, `TUI frame P99 exceeded 75ms: ${JSON.stringify(frameLatency)}`);
  assert(inputLatency.p99 < 50, `keyboard dispatch P99 exceeded 50ms: ${JSON.stringify(inputLatency)}`);
  assert(Math.max(...result.gaps) < 500, "dual-pane output caused a half-second UI freeze");
  assert(result.geometryReads < result.frames, "output repeatedly forced timestamp geometry measurements without a resize");
  for (const id of ["session-a", "session-b"]) {
    await page.evaluate(sessionId => window.__fluidSend(`fluid-exit-${sessionId}`, sessionId, "\x1b[?1049l", "2026-11-07T03:01:00Z"), id);
  }
}
