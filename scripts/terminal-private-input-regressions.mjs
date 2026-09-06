import assert from "node:assert/strict";

const paneSelector = '[data-pane-id="pane-a"]';

export async function checkTerminalPrivateInputRegressions(page) {
  const pane = page.locator(paneSelector);
  const host = pane.locator(".terminal-host");
  const textarea = pane.locator(".xterm-helper-textarea");
  const completion = pane.locator(".terminal-completion");
  const privateButton = pane.locator(".terminal-private-input");
  let eventSequence = 0;
  const prompt = async (text = "probe$ ") => {
    await page.evaluate(({ text, sequence }) => {
      const event = {
        id: `private-input-regression-${sequence}`, sessionId: "session-a", paneId: "session-a:main",
        ts: new Date().toISOString(), direction: "inbound", stream: "stdout", text,
        bytesRef: null, annotations: {},
      };
      const bytes = [...new TextEncoder().encode(text)];
      window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
    }, { text: `\x1b[?1049l\r\x1b[2K${text}`, sequence: ++eventSequence });
    await page.waitForFunction((selector) => document.querySelector(`${selector} .terminal-host`)?.dataset.terminalBuffer === "normal", paneSelector);
  };
  const history = () => page.evaluate(() => localStorage.getItem("portmate.commandHistory") ?? "");
  await textarea.focus();
  if (await host.getAttribute("data-terminal-key-mode") === "command") await page.keyboard.press("i");
  await page.keyboard.press("Enter");
  await prompt();
  await page.keyboard.type("echo public-history-probe");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => (localStorage.getItem("portmate.commandHistory") ?? "").includes("echo public-history-probe"));

  await page.keyboard.type("git stat");
  await completion.waitFor();
  await page.evaluate(() => { window.__invokeCalls = []; });
  await privateButton.click();
  const hiddenOnEnable = await completion.count() === 0;
  await textarea.focus();
  await page.keyboard.press("Control+u");
  await page.keyboard.type("echo private-keyboard-probe");
  await page.waitForTimeout(160); // allow the completion debounce to publish, if incorrectly enabled
  const hiddenDuringInput = await completion.count() === 0;
  await privateButton.click(); // turning protection off must not publish an already-private line
  await textarea.focus();
  await page.keyboard.press("Control+d"); // readline can treat this as delete, not end-of-line
  assert.equal(await privateButton.getAttribute("aria-label"), "本行仍为私密输入");
  await page.keyboard.type("-suffix");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => window.__invokeCalls
    .filter((call) => call.command === "send_text").map((call) => call.args.text).join("")
    .endsWith("private-keyboard-probe\x04-suffix\r"));
  const privateKeyboardWrites = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "send_text"));
  assert.ok(privateKeyboardWrites.every((call) => call.args.sensitive === true && call.args.sessionId === "session-a"),
    "disabling the toggle mid-line removed privacy from the remaining terminal writes");

  await privateButton.click();
  await page.evaluate(async () => {
    const { requestTerminalFreeInput } = await import("/src/terminal-free-input.ts");
    requestTerminalFreeInput(window, "echo private-editor-probe");
  });
  await pane.getByRole("textbox", { name: "自由输入内容" }).waitFor();
  await privateButton.click(); // the already-private editor value remains protected
  await page.evaluate(() => { window.__invokeCalls = []; });
  await pane.getByRole("button", { name: "发送自由输入", exact: true }).click();
  await pane.locator(".terminal-free-input").waitFor({ state: "detached" });
  await page.waitForFunction(() => window.__invokeCalls.some((call) => call.command === "send_text"
    && call.args.text === "echo private-editor-probe\r" && call.args.sensitive === true));

  await privateButton.click();
  await page.evaluate(async () => {
    const { requestTerminalFreeInput } = await import("/src/terminal-free-input.ts");
    requestTerminalFreeInput(window, "echo cancelled-private-editor");
  });
  await pane.getByRole("textbox", { name: "自由输入内容" }).waitFor();
  await pane.getByRole("button", { name: "取消自由输入", exact: true }).click();
  await privateButton.click(); // leave only the cancelled editor's old marker to test
  await page.evaluate(async () => {
    const { requestTerminalFreeInput } = await import("/src/terminal-free-input.ts");
    requestTerminalFreeInput(window, "echo public-editor-after-cancel");
  });
  await pane.getByRole("textbox", { name: "自由输入内容" }).waitFor();
  await page.evaluate(() => { window.__invokeCalls = []; });
  await pane.getByRole("button", { name: "发送自由输入", exact: true }).click();
  await pane.locator(".terminal-free-input").waitFor({ state: "detached" });
  await page.waitForFunction(() => window.__invokeCalls.some((call) => call.command === "send_text"
    && call.args.text === "echo public-editor-after-cancel\r" && !call.args.sensitive));

  await prompt("Password: ");
  await page.waitForFunction((selector) => document.querySelector(`${selector} .terminal-canvas`)?.dataset.terminalPrivateInput === "true", paneSelector);
  await textarea.focus();
  await page.keyboard.type("git status private-auto-probe");
  await page.waitForTimeout(160);
  const hiddenAtPassword = await completion.count() === 0;
  // Prompt output can change while the user is entering a secret. Keep that
  // input line private until its submit even after automatic detection ends.
  await prompt();
  await page.keyboard.press("Enter");
  await textarea.focus();
  await page.keyboard.type("echo public-after-private-probe");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => (localStorage.getItem("portmate.commandHistory") ?? "").includes("echo public-after-private-probe"));
  const recorded = await history();
  const results = {
    hiddenOnEnable, hiddenDuringInput, hiddenAtPassword,
    privateKeyboardHistory: recorded.includes("private-keyboard-probe"),
    privateEditorHistory: recorded.includes("private-editor-probe"),
    privateAutoHistory: recorded.includes("private-auto-probe"),
    cancelledEditorHistory: recorded.includes("cancelled-private-editor"),
    normalHistoryResumed: recorded.includes("echo public-after-private-probe"),
  };
  assert.ok(hiddenOnEnable && hiddenDuringInput && hiddenAtPassword
    && !results.privateKeyboardHistory && !results.privateEditorHistory && !results.privateAutoHistory
    && !results.cancelledEditorHistory,
    `private input reached completion or history: ${JSON.stringify(results)}`);
  return results;
}
