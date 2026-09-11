import assert from "node:assert/strict";

export async function checkCommandSubmissions(context, appUrl) {
  for (const detached of [false, true]) {
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.route("**/src/TerminalCanvas.tsx", async route => {
      const response = await route.fetch();
      await route.fulfill({ response, body: (await response.text()).replace(
        "termRef.current = term;", "termRef.current = term; window.__historyTerminal = term;",
      ) });
    });
    try {
      await page.goto(detached
        ? appUrl + "?detachedPane=1&windowId=history&paneId=history-pane&viewId=history-view&sessionId=bench-uart&title=&color=&keyMode=remote"
        : appUrl);
      if (!detached) await page.locator(".tree-session", { hasText: "Bench UART" }).click();
      const host = page.locator('.terminal-canvas[data-terminal-focused="true"] .terminal-host');
      const textarea = host.locator(".xterm-helper-textarea");
      await textarea.waitFor();
      await page.waitForFunction(() => window.__historyTerminal);
      await page.evaluate(() => {
        Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
          readText: async () => window.__clipboardText,
          writeText: async () => {},
        } });
      });
      const records = () => page.evaluate(() => window.__invokeCalls.filter(c => c.command === "record_command_history").map(c => c.args));
      const focus = () => textarea.focus();
      await focus();
      await page.keyboard.type("abc");
      await page.keyboard.press("ArrowLeft");
      await page.keyboard.press("Delete");
      await page.keyboard.press("Enter");
      await page.waitForFunction(() => window.__invokeCalls.some(c => c.command === "record_command_history" && c.args.command === "ab"));
      assert.equal((await records()).filter(c => c.command === "ab").length, 1, "Enter double-recorded history");

      const paste = async text => {
        await page.evaluate(text => { window.__clipboardText = text; }, text);
        await focus();
        await host.dispatchEvent("auxclick", { button: 1 });
      };
      await paste("echo middle\r\n");
      await page.waitForFunction(() => window.__invokeCalls.some(c => c.command === "record_command_history" && c.args.command === "echo middle"));
      assert.deepEqual((await records()).filter(c => c.command === "echo middle").map(c => c.source), ["paste"]);

      const before = (await records()).length;
      await page.evaluate(() => new Promise(resolve => window.__historyTerminal.write("\x1b[?2004h", resolve)));
      await paste("echo bracketed\r");
      // Wait for the real transport mock, not just the clipboard promise.
      await page.waitForFunction(() => window.__invokeCalls.some(c => c.command === "send_text" && c.args.text.includes("\x1b[200~")));
      assert.equal((await records()).length, before, "bracketed paste was treated as Enter");
      await page.keyboard.press("Enter");
      await page.waitForFunction(() => window.__invokeCalls.some(c => c.command === "record_command_history" && c.args.command.trim() === "echo bracketed"));
      assert.equal((await records()).filter(c => c.command.trim() === "echo bracketed").length, 1);
      await page.evaluate(() => new Promise(resolve => window.__historyTerminal.write("\x1b[?2004l\r\nPassword: ", resolve)));
      const beforePrivate = (await records()).length;
      await paste("test-private-paste\r");
      await page.waitForFunction(() => window.__invokeCalls.some(c => c.command === "send_text"
        && c.args.text.includes("test-private-paste") && c.args.sensitive === true));
      assert.equal((await records()).length, beforePrivate, "private middle-click paste entered history");
      assert.deepEqual(errors, []);
    } finally {
      await page.close();
    }
  }
}
