import assert from "node:assert/strict";

export async function checkSerialWrap(context, appUrl) {
  for (const detached of [false, true]) {
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    // Expose the actual terminal only in the test's served module, never in
    // production. Assertions inspect parsed cells, not merely transport mocks.
    await page.route("**/src/TerminalCanvas.tsx", async route => {
      const response = await route.fetch();
      const body = (await response.text()).replace(
        "termRef.current = term;", "termRef.current = term; window.__serialWrapTerminal = term;",
      );
      await route.fulfill({ response, body });
    });
    try {
      await page.goto(detached
        ? `${appUrl}?detachedPane=1&windowId=serial-wrap&paneId=serial-wrap-pane&viewId=serial-wrap-view&sessionId=bench-uart&title=&color=&keyMode=remote`
        : appUrl);
      if (!detached) await page.locator(".tree-session", { hasText: "Bench UART" }).click();
      const canvas = page.locator('.terminal-canvas[data-terminal-focused="true"]');
      const host = canvas.locator(".terminal-host");
      const input = host.locator(".xterm-helper-textarea");
      await page.waitForFunction(() => window.__serialWrapTerminal && document.querySelector('.terminal-canvas[data-terminal-focused="true"] .terminal-host')?.dataset.terminalSize);
      const emit = async (text, split = false) => page.evaluate(({ text, split }) => {
        for (const chunk of split ? [...text] : [text]) {
          const event = { id: crypto.randomUUID(), sessionId: "bench-uart", paneId: "bench-uart:main", ts: new Date().toISOString(), direction: "inbound", stream: "stdout", text: chunk, bytesRef: null, annotations: {} };
          const bytes = [...new TextEncoder().encode(chunk)];
          window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
          window.__events.push(event);
          window.__emitTauriEvent("portmate-session-event", event);
        }
      }, { text, split });
      const waitColumns = cols => page.waitForFunction(cols => window.__serialWrapTerminal.cols === cols, cols);
      await waitColumns(100);
      const instance = await host.getAttribute("data-terminal-instance-id");
      for (const split of [false, true]) {
        await emit("\r\nBOOT SENTINEL\r\n$ " + "x".repeat(103));
        await page.waitForFunction(() => {
          const term = window.__serialWrapTerminal;
          return term.buffer.active.cursorX === 5 && term.buffer.active.getLine(term.buffer.active.baseY + term.buffer.active.cursorY)?.translateToString(true) === "xxxxx";
        });
        await page.evaluate(() => {
          const term = window.__serialWrapTerminal;
          window.__serialWrapPromptRow = term.buffer.active.baseY + term.buffer.active.cursorY - 1;
        });
        // Delete the five cells on the continuation row, then the last cell
        // of the previous row using a device-side 100-column line editor.
        for (let i = 0; i < 6; i++) await input.press("Backspace");
        await emit("\b \b".repeat(5) + "\x1b[A\x1b[100G \b", split);
        await page.waitForFunction(() => {
          const term = window.__serialWrapTerminal;
          return term.buffer.active.cursorX === 98 && term.buffer.active.baseY + term.buffer.active.cursorY === window.__serialWrapPromptRow;
        });
        const cells = await page.evaluate(() => {
          const term = window.__serialWrapTerminal;
          const row = window.__serialWrapPromptRow;
          return [-1, 0, 1].map(offset => term.buffer.active.getLine(row + offset)?.translateToString(true).trimEnd());
        });
        assert.deepEqual(cells, ["BOOT SENTINEL", "$ " + "x".repeat(97), ""]);
        await emit("Z");
        await page.waitForFunction(() => window.__serialWrapTerminal.buffer.active.cursorX === 99);
      }
      for (const width of [850, 1600, 1050]) {
        await page.setViewportSize({ width, height: 780 });
        await canvas.getByRole("button", { name: "放大终端字号", exact: true }).click();
        await page.waitForTimeout(200);
        assert.equal(await page.evaluate(() => window.__serialWrapTerminal.cols), 100);
      }
      assert.equal(await host.getAttribute("data-terminal-instance-id"), instance);
      assert.equal(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === "resize_session" && c.args.sessionId === "bench-uart").length), 0);
      assert.equal(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === "send_text" && c.args.sessionId === "bench-uart").map(c => c.args.text).join("")), "\x7f".repeat(12));
      await page.setViewportSize({ width: 740, height: 780 });
      await page.waitForTimeout(200);
      const layout = await host.evaluate(el => ({ width: el.clientWidth, content: el.scrollWidth, overflow: getComputedStyle(el).overflowX, pageWidth: document.documentElement.scrollWidth, viewport: innerWidth }));
      assert(layout.content > layout.width && layout.overflow === "auto", `narrow serial viewport must allow horizontal scrolling: ${JSON.stringify({ detached, ...layout })}`);
      assert(layout.pageWidth <= layout.viewport, `serial grid overflowed the workspace: ${JSON.stringify({ detached, ...layout })}`);
      const horizontalOffset = await host.evaluate(el => { el.scrollLeft = el.scrollWidth; return el.scrollLeft; });
      assert(horizontalOffset > 0, "serial overflow cannot be reached by horizontal scrolling");
      await page.screenshot({ path: `/tmp/portmate-serial-wrap-${detached ? "detached" : "main"}.png`, fullPage: true });
      // Explicit profile changes are live; fitting must not overwrite them.
      await page.evaluate(() => { window.__sessions.find(s => s.profile.id === "bench-uart").profile.terminal.cols = 80; });
      await waitColumns(80);
      assert.deepEqual(errors, []);
    } finally {
      await page.close();
    }
  }
}
