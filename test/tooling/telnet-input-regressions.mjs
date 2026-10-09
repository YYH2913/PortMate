import assert from "node:assert/strict";

export async function checkTelnetInput(context, appUrl) {
  for (const detached of [false, true]) {
    const page = await context.newPage();
    try {
      await page.addInitScript(() => {
        const session = window.__sessions.find(item => item.profile.id === "bench-uart");
        session.profile.kind = "telnet";
        session.profile.connection = { kind: "telnet", host: "127.0.0.1", port: 23, telnetBinary: true, telnetNaws: true };
        session.runtime.activeTransport = "telnet";
      });
      await page.goto(detached
        ? `${appUrl}?detachedPane=1&windowId=telnet-input&paneId=telnet-pane&viewId=telnet-view&sessionId=bench-uart&title=&color=&keyMode=remote`
        : appUrl);
      if (!detached) await page.locator(".tree-session", { hasText: "Bench UART" }).click();
      const canvas = page.locator('.terminal-canvas[data-terminal-focused="true"]');
      const input = canvas.locator(".xterm-helper-textarea");
      await input.waitFor();
      await input.press("a");
      await input.press("Enter");
      await page.waitForFunction(() => window.__invokeCalls
        .filter(call => call.command === "send_text" && call.args.sessionId === "bench-uart")
        .map(call => call.args.text).join("") === "a\r\n");
      await page.evaluate(() => {
        const event = { id: crypto.randomUUID(), sessionId: "bench-uart", paneId: "bench-uart:main",
          ts: new Date().toISOString(), direction: "inbound", stream: "stdout", bytesRef: null,
          text: "\r\nPassword: ", annotations: {} };
        window.__events.push(event);
        window.__emitTauriEvent("portmate-session-event", event);
      });
      await page.waitForFunction(() => document.querySelector('.terminal-canvas[data-terminal-focused="true"]')
        ?.dataset.terminalPrivateInput === "true");
      await input.press("p");
      await input.press("Enter");
      await page.waitForFunction(() => window.__invokeCalls
        .filter(call => call.command === "send_text" && call.args.sessionId === "bench-uart")
        .map(call => call.args.text).join("") === "a\r\np\r\n");
      const privateText = await page.evaluate(() => window.__invokeCalls
        .filter(call => call.command === "send_text" && call.args.sessionId === "bench-uart" && call.args.sensitive)
        .map(call => call.args.text).join(""));
      assert.equal(privateText, "p\r\n", "Telnet password submission must preserve privacy and CRLF");
    } finally {
      await page.close();
    }
  }
}
