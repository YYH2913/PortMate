import assert from "node:assert/strict";

const shell = "\x1b[?2004h";
const redraw = columns => "\r\x1b[K\x1b[A" + "\x1b[C".repeat(columns - 1) + "\x1b[K";

export async function checkSerialColumnDetection(context, appUrl) {
  for (const detached of [false, true]) {
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.route("**/src/TerminalCanvas.tsx", async route => {
      const response = await route.fetch();
      await route.fulfill({ response, body: (await response.text()).replace(
        "termRef.current = term;", "termRef.current = term; window.__columnTerminal = term;",
      ) });
    });
    try {
      await page.goto(detached
        ? `${appUrl}?detachedPane=1&windowId=serial-columns&paneId=serial-columns-pane&viewId=serial-columns-view&sessionId=bench-uart&title=&color=&keyMode=remote`
        : appUrl);
      if (!detached) await page.locator(".tree-session", { hasText: "Bench UART" }).click();
      const canvas = page.locator('.terminal-canvas[data-terminal-focused="true"]');
      const host = canvas.locator(".terminal-host");
      await host.locator(".xterm-helper-textarea").waitFor();
      const waitCols = columns => page.waitForFunction(columns => window.__columnTerminal?.cols === columns, columns);
      const apply = canvas.getByRole("button", { name: "应用 80 列", exact: true });
      const idle = () => page.waitForTimeout(200);
      const absent = async () => { await idle(); assert.equal(await apply.count(), 0); };
      const emit = (text, options = {}) => page.evaluate(({ text, options }) => {
        for (const chunk of options.split ? [...text] : [text]) {
          const event = {
            id: crypto.randomUUID(), sessionId: options.sessionId ?? "bench-uart", paneId: (options.sessionId ?? "bench-uart") + ":main",
            ts: options.ts ?? new Date().toISOString(), direction: options.direction ?? "inbound", stream: "stdout", text: chunk, bytesRef: null, annotations: {},
          };
          window.__events.push(event);
          if (options.history) continue;
          const bytes = [...new TextEncoder().encode(chunk)];
          if (!options.textOnly) {
            window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length + (options.truncated ? 1 : 0), truncated: Boolean(options.truncated) });
            // The canonical packet wins over this legacy frame and the
            // structured event. They must not triple the evidence count.
            window.__emitTauriEvent("portmate-terminal-bytes", { id: crypto.randomUUID(), eventId: event.id, sessionId: event.sessionId, ts: event.ts, direction: event.direction, stream: event.stream, bytes, originalLength: bytes.length, truncated: false });
          }
          window.__emitTauriEvent("portmate-session-event", event);
        }
      }, { text, options });
      await waitCols(100);
      const instance = await host.getAttribute("data-terminal-instance-id");

      // Initial historical records cannot establish a width or shell mode.
      await emit(shell + redraw(80).repeat(2), { history: true });
      await page.waitForTimeout(1300);
      await absent();
      await emit(redraw(80).repeat(2));
      await absent();

      await emit(shell + redraw(80));
      await absent();
      assert.equal(await page.evaluate(() => window.__columnTerminal.cols), 100);
      await emit(redraw(80), { split: true });
      await apply.waitFor();
      assert.equal(await page.evaluate(() => window.__columnTerminal.cols), 100, "detection resized without confirmation");
      await apply.click();
      await waitCols(80);
      assert.equal(await host.getAttribute("data-terminal-instance-id"), instance);
      await absent();
      await emit("\r\nPASSIVE WRAP SENTINEL\r\n$ " + "q".repeat(83));
      await page.waitForFunction(() => window.__columnTerminal.buffer.active.cursorX === 5);
      await page.evaluate(() => {
        const b = window.__columnTerminal.buffer.active;
        window.__columnPromptRow = b.baseY + b.cursorY - 1;
      });
      await emit("\b \b".repeat(5) + "\x1b[A\x1b[80G \b", { split: true });
      await page.waitForFunction(() => window.__columnTerminal.buffer.active.cursorX === 78);
      assert.deepEqual(await page.evaluate(() => [-1, 0, 1].map(offset => window.__columnTerminal.buffer.active
        .getLine(window.__columnPromptRow + offset)?.translateToString(true).trimEnd())),
      ["PASSIVE WRAP SENTINEL", "$ " + "q".repeat(77), ""]);
      await canvas.getByRole("button", { name: "放大终端字号", exact: true }).click();
      await page.setViewportSize({ width: 740, height: 780 });
      await idle();
      assert.equal(await page.evaluate(() => window.__columnTerminal.cols), 80, "font/viewport resize lost the applied width");
      await page.screenshot({ path: `/tmp/portmate-passive-columns-${detached ? "detached" : "main"}.png`, fullPage: true });
      await canvas.getByRole("button", { name: "恢复配置列数", exact: true }).click();
      await waitCols(100);
      await emit(redraw(80).repeat(2));
      await absent();

      const profileColumns = columns => page.evaluate(columns => {
        window.__sessions.find(s => s.profile.id === "bench-uart").profile.terminal.cols = columns;
      }, columns);
      await profileColumns(99);
      await waitCols(99);
      await emit(shell + redraw(80).repeat(2));
      await apply.waitFor();
      await canvas.getByRole("button", { name: "忽略列宽建议", exact: true }).click();
      await emit(redraw(80).repeat(2));
      await absent();
      await profileColumns(100);
      await waitCols(100);

      await emit(shell + redraw(80), { textOnly: true });
      await absent();
      await emit(redraw(80), { textOnly: true });
      await apply.waitFor();
      await apply.click();
      await waitCols(80);
      // Reconnection of the same saved profile resets both evidence and its
      // temporary override, without recreating the xterm/history buffer.
      await page.evaluate(() => {
        const session = window.__sessions.find(s => s.profile.id === "bench-uart");
        session.runtime.connectedSince = new Date().toISOString();
      });
      await waitCols(100);
      await absent();
      await emit(shell + redraw(80).repeat(2), { ts: "2020-01-01T00:00:00.000Z" });
      await absent();
      await emit(shell + redraw(80).repeat(2), { truncated: true });
      await absent();
      await emit(shell + redraw(80).repeat(2), { direction: "outbound" });
      await absent();
      await emit(shell + "\x1b[?1049h" + redraw(80).repeat(2));
      await absent();
      await emit("\x1b[?1049l");
      await emit(shell + redraw(80));
      await absent();
      await emit(redraw(80));
      await apply.waitFor();
      await canvas.getByRole("button", { name: "Hex", exact: true }).click();
      await absent();
      await canvas.getByRole("button", { name: "文本", exact: true }).click();
      await apply.waitFor();

      const mutations = await page.evaluate(() => window.__invokeCalls.filter(call => [
        "send_text", "send_bytes", "run_command", "resize_session", "save_session_profile", "serial_set_lines", "serial_send_break",
      ].includes(call.command) && (call.args.sessionId === "bench-uart" || call.args.profile?.id === "bench-uart")));
      assert.deepEqual(mutations, [], "passive detection/application must not write to the device or profile");
      if (!detached) {
        await page.setViewportSize({ width: 1440, height: 900 });
        await page.locator(".tree-session", { hasText: "Edge Router" }).click();
        await page.waitForFunction(() => document.querySelector('.terminal-canvas[data-terminal-focused="true"]')?.dataset.terminalSessionId === "edge-router");
        await emit(shell + redraw(80).repeat(3), { sessionId: "edge-router" });
        await absent();
        await page.locator(".tree-session", { hasText: "Bench UART" }).click();
        await waitCols(100);
        await absent();
      }
      assert.deepEqual(errors, []);
    } finally { await page.close(); }
  }
}
