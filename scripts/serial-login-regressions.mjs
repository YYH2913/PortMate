import assert from "node:assert/strict";

export async function checkSerialLogin(context, appUrl) {
  for (const detached of [false, true]) {
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    try {
      await page.goto(detached
        ? `${appUrl}?detachedPane=1&windowId=serial-login&paneId=serial-login-pane&viewId=serial-login-view&sessionId=bench-uart&title=&color=&keyMode=remote`
        : appUrl);
      if (!detached) await page.locator(".tree-session", { hasText: "Bench UART" }).click();
      const canvas = page.locator('.terminal-canvas[data-terminal-focused="true"]');
      const host = canvas.locator(".terminal-host");
      const input = host.locator(".xterm-helper-textarea");
      await input.waitFor();
      const waitMode = mode => page.waitForFunction(mode => document.querySelector('.terminal-canvas[data-terminal-focused="true"] .terminal-host')?.dataset.terminalKeyMode === mode, mode);
      const inbound = async text => page.evaluate(text => {
        const event = { id: crypto.randomUUID(), sessionId: "bench-uart", paneId: "bench-uart:main", ts: new Date().toISOString(), direction: "inbound", stream: "stdout", bytesRef: null, text, annotations: {} };
        window.__events.push(event);
        window.__emitTauriEvent("portmate-session-event", event);
      }, text);
      const sent = () => page.evaluate(() => window.__invokeCalls.filter(c => c.command === "send_text" && c.args.sessionId === "bench-uart").map(c => c.args.text).join(""));
      await waitMode("remote");
      await inbound("\r\nU-Boot: hit any key to stop autoboot\r\n");
      await input.press("Escape");
      await page.waitForFunction(() => window.__invokeCalls.some(c => c.command === "send_text" && c.args.text === "\x1b"));
      await waitMode("remote");
      assert.equal(await sent(), "\x1b", "serial Escape must reach the device exactly once");
      await inbound(Array.from({ length: 180 }, (_, i) => `Boot line ${i}\r\n`).join("") + "Linux device login: ");
      await page.waitForFunction(() => {
        const canvas = document.querySelector('.terminal-canvas[data-terminal-focused="true"]');
        return canvas?.dataset.terminalPrivateInput === "true" && Number(canvas.querySelector(".terminal-host")?.dataset.terminalBaseY) > 100;
      });
      await input.press("r"); await input.press("o"); await input.press("o"); await input.press("t"); await input.press("Enter");
      await page.waitForFunction(() => window.__invokeCalls.filter(c => c.command === "send_text" && c.args.sessionId === "bench-uart").map(c => c.args.text).join("") === "\x1broot\r");
      await inbound("\r\nPassword: ");
      await page.waitForFunction(() => document.querySelector('.terminal-canvas[data-terminal-focused="true"]')?.dataset.terminalPrivateInput === "true");
      await input.press("x"); await input.press("Enter");
      await page.waitForFunction(() => window.__invokeCalls.filter(c => c.command === "send_text" && c.args.sessionId === "bench-uart").map(c => c.args.text).join("") === "\x1broot\rx\r");
      assert(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === "send_text" && c.args.text === "x").every(c => c.args.sensitive === true)), "password input lost its privacy flag");
      // Explicit local browsing remains available but must not jump back to
      // the old boot row after the device produces another screenful of output.
      await input.press("Shift+Escape");
      await waitMode("command");
      await canvas.getByRole("button", { name: "恢复终端输入", exact: true }).waitFor();
      const before = await sent();
      await inbound("\r\n" + Array.from({ length: 180 }, (_, i) => `Next boot line ${i}\r\n`).join("") + "Linux device login: ");
      await page.waitForFunction(() => Number(document.querySelector('.terminal-canvas[data-terminal-focused="true"] .terminal-host')?.dataset.terminalBaseY) > 280);
      await input.press("Enter");
      assert.equal(await sent(), before, "explicit Normal mode unexpectedly sent Enter");
      const viewport = await host.evaluate(el => ({ top: Number(el.dataset.terminalViewportY), bottom: Number(el.dataset.terminalBaseY) }));
      assert.equal(viewport.top, viewport.bottom, `Normal Enter jumped to stale boot history: ${JSON.stringify(viewport)}`);
      await canvas.getByRole("button", { name: "恢复终端输入", exact: true }).click();
      await waitMode("remote");
      await input.press("r"); await input.press("Enter");
      await page.waitForFunction(expected => window.__invokeCalls.filter(c => c.command === "send_text" && c.args.sessionId === "bench-uart").map(c => c.args.text).join("") === expected, before + "r\r");
      // i remains a local mode-exit key, not part of the username.
      await input.press("Shift+Escape"); await waitMode("command");
      const beforeRecovery = await sent();
      await input.press("i"); await waitMode("remote");
      assert.equal(await sent(), beforeRecovery);
      assert.deepEqual(errors, []);
    } finally { await page.close(); }
  }
}
