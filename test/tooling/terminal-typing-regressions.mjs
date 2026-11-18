import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import { latencySummary } from "./latency-summary.mjs";

/** Real keyboard events with per-key remote echoes, not just IPC dispatches. */
export async function checkTerminalTyping(page) {
  const cpuRate = Number(process.env.PORTMATE_UI_TYPING_CPU_RATE ?? 4);
  assert(Number.isFinite(cpuRate) && cpuRate >= 1 && cpuRate <= 8);
  const cdp = await page.context().newCDPSession(page);
  const host = page.locator('[data-pane-id="pane-a"] .terminal-host');
  const textarea = page.locator('[data-pane-id="pane-a"] .xterm-helper-textarea');
  await textarea.focus();
  if (await host.getAttribute("data-terminal-key-mode") === "command") await page.keyboard.press("i");
  await page.evaluate(() => {
    const send = (text, id) => {
      const event = { id: `typing-${id}`, sessionId: "session-a", paneId: "session-a:main", ts: new Date().toISOString(),
        direction: "inbound", stream: "stdout", text, bytesRef: null, annotations: {} };
      const bytes = [...new TextEncoder().encode(text)];
      window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
      const logs = window.__terminalCompatLogs["session-a"];
      logs.push(event);
      if (logs.length > 600) logs.splice(0, logs.length - 600);
      const session = window.__terminalCompatSessions.find(item => item.profile.id === "session-a");
      session.logLines++;
      session.lastLine = text;
      session.runtime.lastActivity = event.ts;
    };
    window.__typingSend = send;
    send("\x1b[?1049l\x1b[?1000l\x1b[?1006l\r\nTYPE$ ", "start");
  });
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalBuffer === "normal");
  await page.keyboard.press("Control+u");
  await page.waitForTimeout(300);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: cpuRate });
  if (process.env.PORTMATE_TERMINAL_CPU_PROFILE) {
    await cdp.send("Profiler.enable");
    await cdp.send("Profiler.start");
  }
  await page.evaluate(() => {
    const nativeInvoke = window.__TAURI_INTERNALS__.invoke;
    const state = window.__typing = { phase: "short", frames: [], dispatch: [], echoDom: [], sent: "", received: "", layerScans: 0,
      keys: [], echoes: [], eraseTurns: [], repeats: 0, keyCount: 0, delivered: 0, rendered: 0, longTasks: [], sequence: 0 };
    let lastFrame = performance.now();
    const frame = () => {
      const now = performance.now();
      state.frames.push(now - lastFrame);
      lastFrame = now;
      state.raf = requestAnimationFrame(frame);
    };
    state.raf = requestAnimationFrame(frame);
    state.key = event => {
      const text = state.phase === "erase" && event.key === "Backspace" ? "\x7f" : event.key;
      if (text.length !== 1 || event.ctrlKey || event.altKey || event.metaKey) return;
      if (event.repeat && text === "\x7f") state.repeats++;
      state.keys.push({ text, at: performance.now(), phase: state.phase });
      state.sent += text;
      state.keyCount++;
    };
    document.addEventListener("keydown", state.key, true);
    state.longObserver = new PerformanceObserver(list => state.longTasks.push(...list.getEntries().map(entry => entry.duration)));
    state.longObserver.observe({ entryTypes: ["longtask"] });
    state.domObserver = new MutationObserver(() => {
      if (!state.echoes.length) return;
      const rows = document.querySelector('[data-pane-id="pane-a"] .xterm-rows');
      const visible = (state.phase === "erase" ? rows.lastElementChild : rows).textContent.replace(/\s/g, "");
      const now = performance.now();
      state.echoes = state.echoes.filter(echo => {
        // A shortened string is already a prefix of the OLD row; deletion
        // isn't visible until the entire current prompt line matches.
        if (echo.phase === "erase" ? visible !== echo.tail : !visible.includes(echo.tail)) return true;
        state.echoDom.push({ phase: echo.phase, ms: now - echo.at });
        state.rendered++;
        return false;
      });
    });
    state.domObserver.observe(document.querySelector('[data-pane-id="pane-a"] .xterm-rows'), { childList: true, subtree: true, characterData: true });
    let line = "", prompt = 0;
    window.__typingSend("\r\x1b[2KTYPE-0$ ", "prompt");
    window.__TAURI_INTERNALS__.invoke = async function(command, args, ...rest) {
      if (command === "send_text" && args?.sessionId === "session-a") {
        const now = performance.now();
        for (const text of args.text) {
          if (text < " ") {
            if (text === "\x15" || text === "\r") { line = ""; prompt++; }
            continue;
          }
          const key = state.keys[0];
          // insertText() prefills a long line without pretending those are real
          // keydown samples. Timed keyboard input follows only after it drains.
          line = text === "\x7f" ? line.slice(0, -1) : line + text;
          if (key?.text === text) {
            state.keys.shift();
            state.dispatch.push({ phase: key.phase, ms: now - key.at });
            state.received += text;
            state.delivered++;
            const row = `TYPE-${prompt}$${line}`;
            state.echoes.push({ ...key, tail: (key.phase === "erase" ? row : row.slice(-24)).replace(/\s/g, "") });
          }
        }
        // A small deterministic transport delay makes the echo path distinct
        // from send_text ACK. This is a fixture, not a real device measurement.
        await new Promise(resolve => setTimeout(resolve, 3));
        const text = args.text === "\x15" ? `\r\x1b[2KTYPE-${prompt}$ `
          : args.text.replace(/\r/g, `\r\nTYPE-${prompt}$ `).replace(/\x7f/g, "\b\x1b[K");
        const start = performance.now();
        window.__typingSend(text, state.sequence++);
        if (state.phase === "erase" && args.text === "\x7f") {
          queueMicrotask(() => state.eraseTurns.push(performance.now() - start));
        }
      }
      return nativeInvoke.call(this, command, args, ...rest);
    };
    state.nativeInvoke = nativeInvoke;
    const query = Document.prototype.querySelectorAll;
    Document.prototype.querySelectorAll = function(selector, ...args) {
      if (selector.includes(".dialog-backdrop") || selector.includes(".menu-popover")) state.layerScans++;
      return query.call(this, selector, ...args);
    };
    state.nativeQuery = query;
  });
  let random = 0x12345678;
  const letters = length => Array.from({ length }, () => {
    random ^= random << 13; random ^= random >>> 17; random ^= random << 5;
    return String.fromCharCode(97 + (random >>> 0) % 26);
  }).join("");
  let result;
  try {
    for (let line = 0; line < 12; line++) {
      // Human cadence crosses the 80ms completion debounce. A rapid key burst
      // alone hides preview/layout work by perpetually postponing that timer.
      await page.keyboard.type(`git status --short ${letters(13)}`, { delay: 110 });
      await page.waitForFunction(() => !window.__typing.echoes.length);
      await page.keyboard.press("Enter");
    }
    await page.evaluate(() => { window.__typing.phase = "long"; });
    await page.keyboard.insertText("a".repeat(6000));
    await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .xterm-rows')?.textContent.includes("a".repeat(24)));
    await page.keyboard.type(letters(640), { delay: 20 });
    await page.waitForFunction(() => !window.__typing.echoes.length && !window.__typing.keys.length);

    // Fill the screen with actual text: a whole-viewport timestamp scan on
    // each readline Backspace (BS + CSI K) is not visible on mostly empty rows.
    await page.evaluate(() => {
      window.__typing.phase = "erase";
      window.__typingSend("\r\n" + ("previous output ".repeat(8) + "\r\n").repeat(100), "scrollback");
    });
    await page.keyboard.press("Control+u");
    const lastRow = page.locator('[data-pane-id="pane-a"] .xterm-rows > :last-child');
    await lastRow.filter({ hasText: /TYPE-\d+\$/ }).waitFor();
    let rendered = await page.evaluate(() => window.__typing.rendered);
    for (let burst = 0; burst < 32; burst++) {
      const seed = letters(16);
      await page.keyboard.insertText(seed);
      await lastRow.filter({ hasText: seed }).waitFor();
      for (let i = 0; i < seed.length; i++) {
        await page.keyboard.down("Backspace");
        await page.waitForFunction(n => window.__typing.rendered === n, ++rendered, { polling: 8, timeout: 2000 });
      }
      await page.keyboard.up("Backspace");
    }
  } finally {
    result = await page.evaluate(() => {
      const state = window.__typing;
      cancelAnimationFrame(state.raf);
      document.removeEventListener("keydown", state.key, true);
      state.domObserver.disconnect();
      state.longObserver.disconnect();
      window.__TAURI_INTERNALS__.invoke = state.nativeInvoke;
      Document.prototype.querySelectorAll = state.nativeQuery;
      return { frames: state.frames, dispatch: state.dispatch, echoDom: state.echoDom, sent: state.sent, received: state.received,
        eraseTurns: state.eraseTurns, repeats: state.repeats, keyCount: state.keyCount, delivered: state.delivered, rendered: state.rendered, longTasks: state.longTasks, layerScans: state.layerScans };
    });
    if (process.env.PORTMATE_TERMINAL_CPU_PROFILE) {
      const { profile } = await cdp.send("Profiler.stop");
      writeFileSync(process.env.PORTMATE_TERMINAL_CPU_PROFILE, JSON.stringify(profile));
    }
    await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
    await cdp.detach();
  }
  const summary = { cpuRate, frames: latencySummary(result.frames), keys: result.keyCount, delivered: result.delivered, rendered: result.rendered,
    short: {}, long: {}, erase: {}, eraseEchoAdmissionTurn: latencySummary(result.eraseTurns), repeatedBackspaces: result.repeats, longTasks: result.longTasks, documentLayerScans: result.layerScans,
    scope: "Chromium production frontend, 3ms simulated native echo; DOM mutation, not physical display/device RTT" };
  for (const phase of ["short", "long", "erase"]) {
    summary[phase] = { dispatch: latencySummary(result.dispatch.filter(sample => sample.phase === phase).map(sample => sample.ms)),
      keyToEchoDom: latencySummary(result.echoDom.filter(sample => sample.phase === phase).map(sample => sample.ms)) };
  }
  console.log(JSON.stringify({ continuousTyping: summary }));
  if (process.env.PORTMATE_UI_TYPING_REPORT) writeFileSync(process.env.PORTMATE_UI_TYPING_REPORT, `${JSON.stringify(summary, null, 2)}\n`);
  assert.equal(result.received, result.sent, "continuous keyboard input was lost, duplicated or reordered");
  assert.equal(summary.keys, 1024 + 512);
  assert.equal(summary.repeatedBackspaces, 32 * 15);
  assert.equal(summary.erase.keyToEchoDom.samples, 512);
  assert.equal(summary.rendered, summary.keys, "some echoes were never rendered");
  assert(summary.frames.p99 < 50 && summary.frames.max < 250, "continuous typing froze the UI");
  for (const phase of ["short", "long", "erase"]) {
    assert(summary[phase].dispatch.p99 < 50, `${phase} command input dispatch stalled`);
    assert(summary[phase].keyToEchoDom.p99 < 75, `${phase} command echo DOM P99 exceeded 75ms`);
  }
  assert(summary.documentLayerScans <= 4, "typing/echo repeatedly scanned the whole document for modal layers");
}
