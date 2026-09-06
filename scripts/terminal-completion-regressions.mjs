import assert from "node:assert/strict";

const paneSelector = '[data-pane-id="pane-a"]';

async function emitCursor(page, id, text) {
  await page.evaluate(({ id, text }) => {
    const event = {
      id, text, sessionId: "session-a", paneId: "session-a:main",
      ts: new Date().toISOString(), direction: "inbound", stream: "stdout",
      bytesRef: null, annotations: {},
    };
    const bytes = [...new TextEncoder().encode(text)];
    window.__emitTauriEvent("portmate-terminal-live", {
      event, bytes, originalLength: bytes.length, truncated: false,
    });
  }, { id, text });
}

async function waitForLine(page, line) {
  await page.waitForFunction(({ paneSelector, line }) => (
    document.querySelector(`${paneSelector} .terminal-completion-preview code`)?.textContent === line
  ), { paneSelector, line });
}

async function readProbe(page) {
  return page.evaluate(() => window.__completionRegressionProbe.read());
}

function assertPlacement(state) {
  assert.ok(state.samePanel, `completion panel was replaced: ${JSON.stringify(state)}`);
  assert.ok(state.gap >= 1 && state.gap <= 6, `completion obscured the cursor: ${JSON.stringify(state)}`);
  assert.ok(state.bottom <= state.regionBottom + 1, `completion overflowed the region: ${JSON.stringify(state)}`);
  assert.equal(state.terminalTransform, state.gutterTransform, "timestamps moved separately from the terminal");
}

async function checkPendingCompletionAcceptance(page, textarea, panel) {
  const results = [];
  for (const { initial, edit, click, navigate, expected } of [
    { initial: "git stat", edit: "u", expected: "us " },
    { initial: "git statu", edit: "Backspace", expected: "\x7fus " },
    { initial: "git stat", edit: "u", click: true, expected: "us " },
    { initial: "git stat", edit: "z", expected: "z\t" },
    { initial: "git stat", edit: "us", expected: "us " },
    { initial: "git stat", edit: "Control+u", expected: "\x15\t" },
    { initial: "git s", edit: "tat", navigate: true, expected: "tatus " },
  ]) {
    await textarea.focus();
    await page.keyboard.press("Enter");
    await page.keyboard.type(initial);
    await waitForLine(page, initial);
    // Keep the visible snapshot old regardless of machine/CI speed. Input and
    // native sends still run normally; only the known completion debounce waits.
    await page.evaluate(() => {
      window.__completionAcceptanceTimeout = window.setTimeout;
      window.setTimeout = (handler, delay, ...args) => window.__completionAcceptanceTimeout(
        handler, delay === 80 ? 60_000 : delay, ...args,
      );
      window.__invokeCalls = [];
    });
    try {
      if (edit === "Backspace" || edit === "Control+u") await page.keyboard.press(edit);
      else await page.keyboard.type(edit);
      assert.equal(await panel.locator(".terminal-completion-preview code").first().textContent(), initial);
      if (navigate) {
        await page.keyboard.press("ArrowDown");
        assert.equal(await panel.locator('[role="option"][aria-selected="true"] code').textContent(), "status",
          "deferred candidate navigation highlighted a different command than Tab would accept");
      }
      if (click) await panel.getByRole("option").click();
      else await page.keyboard.press("Tab");
      await page.waitForFunction(() => window.__invokeCalls
        .filter((call) => call.command === "send_text")
        .map((call) => call.args.text).join("").match(/[ \t]$/));
      const sent = await page.evaluate(() => window.__invokeCalls
        .filter((call) => call.command === "send_text").map((call) => call.args.text).join(""));
      assert.equal(sent, expected, `stale completion after ${initial} + ${edit} (${click ? "click" : "Tab"})`);
      results.push({ initial, edit, action: click ? "click" : navigate ? "ArrowDown+Tab" : "Tab", sent });
    } finally {
      await page.evaluate(() => {
        window.setTimeout = window.__completionAcceptanceTimeout;
        delete window.__completionAcceptanceTimeout;
      });
      await textarea.focus();
      await page.keyboard.press("Enter");
      await panel.waitFor({ state: "detached" });
    }
  }
  return results;
}

export async function checkTerminalCompletionRegressions(page) {
  const originalPrefs = await page.evaluate(() => localStorage.getItem("portmate.terminalPrefs"));
  const originalViewport = page.viewportSize();
  const reducedMotion = await page.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches);
  const host = page.locator(`${paneSelector} .terminal-host`);
  const textarea = page.locator(`${paneSelector} .xterm-helper-textarea`);
  const panel = page.locator(`${paneSelector} .terminal-completion`);
  try {
    // An input preview gives a deterministic acknowledgement of each React
    // completion update; do not infer it from an arbitrary sleep or IPC reply.
    await page.evaluate(() => {
      const prefs = JSON.parse(localStorage.getItem("portmate.terminalPrefs"));
      localStorage.setItem("portmate.terminalPrefs", JSON.stringify({
        ...prefs, completionEnabled: true, completionCommandNames: true,
        completionCommandOptions: true, completionCommandArgs: true,
        completionHistory: false, completionQuickCommands: false,
        completionTriggerChars: "1 字符", completionListHeight: "7 行",
        completionPreviewMode: "输入框",
      }));
    });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await page.reload();
    await page.waitForFunction((selector) => (
      document.querySelector(`${selector} .terminal-host`)?.dataset.terminalReady === "true"
    ), paneSelector);
    await textarea.focus();
    if (await host.getAttribute("data-terminal-key-mode") === "command") await page.keyboard.press("i");
    await page.keyboard.press("Enter");
    await emitCursor(page, "completion-regression-bottom", "\x1b[?1049l\x1b[999;1H");
    await page.keyboard.type("git stat");
    await waitForLine(page, "git stat");
    await panel.evaluate(async (element) => {
      await Promise.all(element.getAnimations().map((animation) => animation.finished.catch(() => {})));
    });

    await panel.evaluate((element) => {
      const region = element.closest(".terminal-terminal-region");
      const canvas = element.closest(".terminal-canvas");
      const host = region.querySelector(".terminal-host");
      const gutter = region.querySelector(".terminal-timestamp-gutter");
      const regionRect = region.getBoundingClientRect;
      let measurements = 0;
      let mounts = 0;
      let removals = 0;
      let animationStarts = 0;
      let transformChanges = 0;
      // Only the anchor reads this region's rect, unlike the host/screen rects
      // which are also used by timestamps and xterm itself.
      region.getBoundingClientRect = function () {
        measurements += 1;
        return regionRect.call(this);
      };
      const countPanels = (nodes) => [...nodes].reduce((count, node) => count + (node.nodeType === 1
        ? Number(node.matches(".terminal-completion")) + node.querySelectorAll(".terminal-completion").length
        : 0), 0);
      const observer = new MutationObserver((records) => {
        for (const record of records) {
          mounts += countPanels(record.addedNodes);
          removals += countPanels(record.removedNodes);
          if (record.type === "attributes" && (record.target === host || record.target === gutter)) {
            transformChanges += 1;
          }
        }
      });
      observer.observe(region, { subtree: true, childList: true, attributes: true, attributeFilter: ["style"] });
      const animationListener = (event) => {
        if (event.target.matches(".terminal-completion")) animationStarts += 1;
      };
      region.addEventListener("animationstart", animationListener);
      window.__invokeCalls = [];
      window.__completionRegressionProbe = {
        read: () => {
          const rect = regionRect.call(region);
          const panelRect = element.getBoundingClientRect();
          return {
            measurements, mounts, removals, animationStarts, transformChanges,
            samePanel: element.isConnected && region.querySelector(".terminal-completion") === element,
            height: panelRect.height, shift: Number(canvas.dataset.completionShift),
            gap: panelRect.top - rect.top - Number(canvas.dataset.completionCursorBottom),
            bottom: panelRect.bottom, regionBottom: rect.bottom,
            terminalTransform: host.style.transform, gutterTransform: gutter.style.transform,
          };
        },
        dispose: () => {
          observer.disconnect();
          region.removeEventListener("animationstart", animationListener);
          delete region.getBoundingClientRect;
        },
      };
    });

    // Different text and suffixes, but one candidate and the same panel size.
    // Wait for every update so the 80ms debounce cannot mask a remount.
    for (let index = 0; index < 6; index += 1) {
      await page.keyboard.type("u");
      await waitForLine(page, "git statu");
      await page.keyboard.press("Backspace");
      await waitForLine(page, "git stat");
    }
    const stableEdits = await readProbe(page);
    assertPlacement(stableEdits);
    assert.equal(stableEdits.measurements, 0, "text-only edits recomputed completion geometry");
    assert.equal(stableEdits.mounts, 0, "text-only edits remounted the completion panel");
    assert.equal(stableEdits.removals, 0, "text-only edits removed the completion panel");
    assert.equal(stableEdits.animationStarts, 0, "text-only edits restarted the entrance animation");
    assert.equal(stableEdits.transformChanges, 0, "text-only edits moved the terminal or timestamps");

    // Expanding and shrinking the candidates must re-anchor the same panel.
    const candidateHeights = [stableEdits.height];
    for (const line of ["git sta", "git st", "git s"]) {
      await page.keyboard.press("Backspace");
      await waitForLine(page, line);
      const state = await readProbe(page);
      assertPlacement(state);
      candidateHeights.push(state.height);
    }
    await page.keyboard.type("tat");
    await waitForLine(page, "git stat");
    const resizedCandidates = await readProbe(page);
    assertPlacement(resizedCandidates);
    assert.ok(new Set(candidateHeights).size > 1, "candidate test did not change the panel height");
    assert.ok(resizedCandidates.measurements > 0, "candidate size changes did not refresh the anchor");
    assert.equal(resizedCandidates.mounts + resizedCandidates.removals + resizedCandidates.animationStarts, 0);
    assert.equal(await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "resize_session").length), 0,
      "completion updates resized the remote PTY");

    await emitCursor(page, "completion-regression-up", "\x1b[8A");
    await page.waitForFunction((shift) => window.__completionRegressionProbe.read().shift < shift, resizedCandidates.shift);
    assertPlacement(await readProbe(page));
    await emitCursor(page, "completion-regression-down", "\x1b[999;1H");
    await page.waitForFunction((shift) => window.__completionRegressionProbe.read().shift === shift, resizedCandidates.shift);
    assertPlacement(await readProbe(page));

    // A pixel resize often leaves rows/cols unchanged. Observe several small
    // resizes and require the anchor to follow even without an xterm onResize.
    let sameGridResize = false;
    for (let height = 901; height <= 903; height += 1) {
      const previousSize = await host.getAttribute("data-terminal-size");
      const previous = await readProbe(page);
      await page.setViewportSize({ width: 1440, height });
      await page.waitForFunction((bottom) => window.__completionRegressionProbe.read().regionBottom !== bottom, previous.regionBottom);
      await page.waitForFunction((count) => window.__completionRegressionProbe.read().measurements > count, previous.measurements);
      assertPlacement(await readProbe(page));
      if (previousSize === await host.getAttribute("data-terminal-size")) sameGridResize = true;
    }
    assert.ok(sameGridResize, "pixel resize test did not exercise an unchanged terminal grid");
    const geometryUpdates = await readProbe(page);
    assert.equal(geometryUpdates.mounts + geometryUpdates.removals + geometryUpdates.animationStarts, 0);
    await page.evaluate(() => window.__completionRegressionProbe.dispose());

    await page.evaluate(() => { window.__invokeCalls = []; });
    await page.keyboard.press("Tab");
    await page.waitForFunction(() => window.__invokeCalls.some((call) => call.command === "send_text" && call.args.text === "us "));
    await page.keyboard.press("Enter");
    await panel.waitFor({ state: "detached" });
    assert.equal(await host.evaluate((element) => element.style.transform), "", "closing completion left the terminal shifted");

    // Reopen at a different row, then return to the row measured before close.
    // A separate, stale onWriteParsed row cache used to skip this update.
    await emitCursor(page, "completion-regression-hidden-move", "\x1b[5;1H");
    await page.keyboard.type("git stat");
    await waitForLine(page, "git stat");
    await page.waitForFunction((selector) => (
      document.querySelector(`${selector} .terminal-canvas`)?.dataset.completionShift === "0"
    ), paneSelector);
    await emitCursor(page, "completion-regression-reopen-bottom", "\x1b[999;1H");
    await page.waitForFunction((selector) => (
      Number(document.querySelector(`${selector} .terminal-canvas`)?.dataset.completionShift) > 0
    ), paneSelector);
    // A settled resize can add a terminal row after the earlier snapshot.
    // Check the current cursor/region, not the pre-settlement pixel offset.
    const reopenedPlacement = await panel.evaluate((element) => {
      const region = element.closest(".terminal-terminal-region").getBoundingClientRect();
      const panel = element.getBoundingClientRect();
      const cursorBottom = Number(element.closest(".terminal-canvas").dataset.completionCursorBottom);
      return { gap: panel.top - region.top - cursorBottom, bottom: panel.bottom, regionBottom: region.bottom };
    });
    assert.ok(reopenedPlacement.gap >= 1 && reopenedPlacement.gap <= 6
      && reopenedPlacement.bottom <= reopenedPlacement.regionBottom + 1,
    `reopened completion obscured the cursor: ${JSON.stringify(reopenedPlacement)}`);
    await page.keyboard.press("Enter");
    await panel.waitFor({ state: "detached" });
    const pendingAcceptance = await checkPendingCompletionAcceptance(page, textarea, panel);
    return { stableEdits, candidateHeights, geometryUpdates, tabSuffix: "us ", reopenedAnchor: true, pendingAcceptance };
  } finally {
    await page.evaluate((prefs) => {
      window.__completionRegressionProbe?.dispose();
      delete window.__completionRegressionProbe;
      if (prefs === null) localStorage.removeItem("portmate.terminalPrefs");
      else localStorage.setItem("portmate.terminalPrefs", prefs);
    }, originalPrefs);
    await page.emulateMedia({ reducedMotion: reducedMotion ? "reduce" : "no-preference" });
    if (originalViewport) await page.setViewportSize(originalViewport);
    await page.reload();
    await page.waitForFunction((selector) => (
      document.querySelector(`${selector} .terminal-host`)?.dataset.terminalReady === "true"
    ), paneSelector);
    await textarea.focus();
  }
}
