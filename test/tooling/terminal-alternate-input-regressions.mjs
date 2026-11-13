import assert from "node:assert/strict";

export async function checkTerminalAlternateInput(page) {
  const canvas = page.locator('[data-pane-id="pane-a"] .terminal-canvas');
  const host = canvas.locator(".terminal-host");
  await canvas.locator(".xterm-helper-textarea").focus();
  if (await host.getAttribute("data-terminal-key-mode") === "command") await page.keyboard.press("i");
  const screen = async (alternate, id) => {
    await page.evaluate(({ alternate, id }) => {
      const text = alternate ? "\x1b[?1049h\x1b[2J\x1b[H" : "\x1b[?1049l\r\nTUI-EXIT$ ";
      const event = { id, sessionId: "session-a", paneId: "session-a:main", ts: new Date().toISOString(),
        direction: "inbound", stream: "stdout", text, bytesRef: null, annotations: {} };
      const bytes = [...new TextEncoder().encode(text)];
      window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
    }, { alternate, id });
    await page.waitForFunction(alternate => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalBuffer
      === (alternate ? "alternate" : "normal"), alternate);
  };
  const clearSends = () => page.evaluate(() => { window.__invokeCalls = []; });
  const input = () => page.evaluate(() => window.__invokeCalls.filter(call => call.command === "send_text").map(call => call.args.text).join(""));
  await screen(true, "typing-alternate-initial");
  await page.keyboard.type("git st", { delay: 110 });
  await page.waitForTimeout(150);
  assert.equal(await canvas.locator(".terminal-completion").count(), 0, "TUI typing opened a shell completion overlay");
  await clearSends();
  await page.keyboard.press("Tab");
  await page.keyboard.press("ArrowDown");
  await page.waitForFunction(() => window.__invokeCalls.filter(call => call.command === "send_text").map(call => call.args.text).join("").endsWith("\t\x1b[B"));
  assert.equal(await input(), "\t\x1b[B", "shell completion intercepted TUI navigation");
  await screen(false, "typing-alternate-exit");
  await page.keyboard.press("Enter");
  await page.keyboard.type("git stat");
  await canvas.locator(".terminal-completion").waitFor();
  await screen(true, "typing-alternate-reenter");
  await canvas.locator(".terminal-completion").waitFor({ state: "detached" });
  await clearSends();
  await page.keyboard.press("Tab");
  await screen(false, "typing-alternate-final-exit");
  await page.keyboard.press("Tab");
  await page.waitForFunction(() => window.__invokeCalls.filter(call => call.command === "send_text").map(call => call.args.text).join("").endsWith("\t\t"));
  assert.equal(await input(), "\t\t", "a stale normal-buffer candidate survived a TUI transition");
  await page.keyboard.press("Enter");
  console.log(JSON.stringify({ alternateInput: { shellOverlay: false, navigationPreserved: true, staleCandidatesCleared: true } }));
}
