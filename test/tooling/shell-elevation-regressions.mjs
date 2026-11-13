import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const messages = Object.fromEntries(["zh", "en", "fr", "es", "ru", "ar"].map(locale => [
  locale, JSON.parse(readFileSync(new URL(`../../src/locales/${locale}.json`, import.meta.url), "utf8")),
]));

export async function checkShellElevation(context, appUrl, screenshotPrefix) {
  const page = await context.newPage();
  const errors = [];
  const confirmations = [];
  let accept = false;
  page.on("pageerror", error => errors.push(error.message));
  page.on("dialog", async dialog => {
    confirmations.push(dialog.message());
    await (accept ? dialog.accept() : dialog.dismiss());
  });
  await page.route("**/src/TerminalCanvas.tsx", async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()).replace(
      "termRef.current = term;", "termRef.current = term; window.__elevationTerminal = term;",
    ) });
  });
  const menu = async key => {
    await page.locator('.menu-trigger[aria-controls="menu-session"]').click();
    await page.locator("#menu-session").getByRole("button", { name: messages.zh[key], exact: true }).click();
  };
  try {
    await page.goto(appUrl);
    await page.locator(".wind-root").waitFor();
    await menu("elevated-local-terminal");
    await page.waitForFunction(() => window.__sessions.some(s => s.profile.connection.elevated));
    assert.equal(confirmations.length, 1);
    assert(confirmations[0].includes("MCP") && confirmations[0].includes("UAC") && confirmations[0].includes("参数"));
    let state = await page.evaluate(() => ({
      elevated: window.__sessions.find(s => s.profile.connection.elevated),
      opens: window.__invokeCalls.filter(c => c.command === "open_elevated_shell_session"),
    }));
    assert.equal(state.elevated.runtime.status, "disconnected");
    assert.equal(state.opens.length, 0, "declined consent still launched a process");
    const id = state.elevated.profile.id;
    accept = true;
    await menu("elevated-local-terminal");
    await page.waitForFunction(id => window.__sessions.find(s => s.profile.id === id)?.runtime.status === "connected", id);
    state = await page.evaluate(id => ({
      elevated: window.__sessions.find(s => s.profile.id === id),
      opens: window.__invokeCalls.filter(c => c.command === "open_elevated_shell_session"),
      ordinary: window.__invokeCalls.filter(c => c.command === "open_session" && c.args.request?.sessionId === id),
    }), id);
    assert.equal(confirmations.length, 2);
    assert.equal(state.opens.length, 1);
    assert.deepEqual(state.opens[0].args.expectedShell, state.elevated.profile.connection);
    assert.equal(state.ordinary.length, 0);
    assert(await page.locator(".workspace-pane-tab.active .shell-elevation-indicator").isVisible());

    // sudo's fixed prompt uses the existing private-input boundary: no history
    // or synchronized password broadcast, even before a localized shell prompt.
    await page.waitForFunction(() => window.__elevationTerminal);
    await page.evaluate(() => new Promise(resolve => window.__elevationTerminal.write("\r\nPortMate sudo password: ", resolve)));
    const host = page.locator('.terminal-canvas[data-terminal-focused="true"] .terminal-host');
    await host.locator(".xterm-helper-textarea").focus();
    await page.keyboard.type("fixture-sudo-secret");
    await page.keyboard.press("Enter");
    await page.waitForFunction(() => window.__invokeCalls.some(c => c.command === "send_text" && c.args.sensitive));
    assert.equal(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === "record_command_history" && c.args.command.includes("fixture-sudo-secret")).length), 0);

    // A normal local terminal must remain a separate, unprivileged session.
    await menu("local-terminal");
    await page.waitForFunction(() => window.__sessions.some(s => s.profile.tags.includes("portmate-local-terminal") && s.runtime.status === "connected"));
    assert.equal(confirmations.length, 2, "normal terminal unexpectedly asked for elevation");
    assert.equal(await page.evaluate(id => window.__sessions.find(s => s.profile.id === id).runtime.status, id), "connected");
    assert.equal(await page.evaluate(id => window.__invokeCalls.filter(c => c.command === "close_session" && c.args.sessionId === id).length, id), 0);

    await page.getByRole("button", { name: messages.zh["elevated-local-terminal"], exact: true }).first().click();
    await page.getByRole("button", { name: `断开 ${messages.zh["elevated-local-terminal"]}`, exact: true }).click();
    await page.getByRole("button", { name: `连接 ${messages.zh["elevated-local-terminal"]}`, exact: true }).waitFor();
    // A concurrent non-overlapping save can return a different executable. It
    // must not acquire the consent just given for the old configuration.
    await page.evaluate(() => { window.__changeElevatedShellOnSave = "C:\\Windows\\System32\\cmd.exe"; });
    await page.getByRole("button", { name: `连接 ${messages.zh["elevated-local-terminal"]}`, exact: true }).click();
    const notice = page.locator(".notice-dialog");
    await notice.waitFor();
    assert((await notice.textContent()).includes(messages.zh["shell-elevation-config-changed"]));
    assert.equal(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === "open_elevated_shell_session").length), 1,
      "save silently transferred consent to a different executable");
    await notice.getByRole("button", { name: messages.zh.ok, exact: true }).click();
    assert.equal(confirmations.length, 3);

    await menu("session-settings");
    const dialog = page.locator(".session-settings-dialog");
    await dialog.getByRole("treeitem", { name: "Shell", exact: true }).click();
    const field = dialog.locator(".shell-elevation-field");
    for (const locale of ["zh", "en", "fr", "es", "ru", "ar"]) {
      await page.evaluate(async locale => (await import("/src/i18n.ts")).setLanguagePreference(locale), locale);
      const checkbox = field.getByRole("checkbox", { name: messages[locale]["shell-elevation"], exact: true });
      await checkbox.waitFor();
      assert(await checkbox.isChecked());
      const description = await checkbox.getAttribute("aria-describedby");
      assert.equal(await page.locator(`[id="${description}"]`).textContent(), messages[locale]["shell-elevation-hint"]);
      await page.setViewportSize({ width: 480, height: 820 });
      await field.scrollIntoViewIfNeeded();
      const dimensions = await field.evaluate(element => ({ width: element.clientWidth, scroll: element.scrollWidth, left: element.getBoundingClientRect().left, right: element.getBoundingClientRect().right }));
      assert(dimensions.scroll <= dimensions.width + 1 && dimensions.left >= 0 && dimensions.right <= 481, `${locale}: elevation settings overflowed`);
      if (["zh", "en", "ar"].includes(locale)) await page.screenshot({ path: `${screenshotPrefix}-elevation-${locale}-narrow.png` });
      await page.setViewportSize({ width: 1440, height: 900 });
    }
    await page.evaluate(async () => (await import("/src/i18n.ts")).setLanguagePreference("zh"));
    await field.getByRole("checkbox").uncheck();
    await dialog.getByRole("button", { name: "保存", exact: true }).click();
    await dialog.waitFor({ state: "detached" });
    assert.equal(await page.evaluate(id => window.__sessions.find(s => s.profile.id === id).profile.connection.elevated, id), false);

    // Quick-create exposes the same explicit, default-off preference.
    await menu("new-session");
    await dialog.getByRole("tab", { name: "Shell", exact: true }).click();
    const quick = dialog.locator(".shell-elevation-field").getByRole("checkbox");
    assert.equal(await quick.isChecked(), false);
    await quick.check();
    await dialog.getByRole("button", { name: messages.zh["save-only"], exact: true }).click();
    await dialog.waitFor({ state: "detached" });
    assert.equal(await page.evaluate(() => window.__invokeCalls.findLast(c => c.command === "save_session_profile").args.profile.connection.elevated), true);
    assert.equal(confirmations.length, 3, "saving a preference launched an elevated process");

    // A saved elevated startup target is skipped even with an unlocked vault.
    await page.evaluate(() => sessionStorage.setItem("portmate.workspaceUiCheck.recoverElevatedStartup", "true"));
    await page.goto(`${appUrl}?portmateTestVault=unlocked`);
    await page.locator(".tree-session", { hasText: "Local Shell" }).waitFor();
    await page.waitForFunction(() => window.__invokeCalls.some(c => c.command === "portable_vault_status") && window.__invokeCalls.some(c => c.command === "list_sessions"));
    await page.waitForTimeout(500);
    assert.equal(await page.evaluate(() => window.__sessions.find(s => s.profile.id === "local-shell").profile.connection.elevated), true);
    assert.equal(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === "open_elevated_shell_session" || (c.command === "open_session" && c.args.request?.sessionId === "local-shell")).length), 0);
    assert.equal(confirmations.length, 3, "startup requested elevation consent automatically");
    // Start without a desktop bridge rather than deleting a live bridge (which
    // would invalidate already-registered native event callback teardown).
    const previewContext = await context.browser().newContext({ locale: "zh-CN" });
    try {
      const preview = await previewContext.newPage();
      const previewErrors = [];
      preview.on("pageerror", error => previewErrors.push(error.message));
      preview.on("dialog", dialog => { confirmations.push(dialog.message()); void dialog.dismiss(); });
      await preview.goto(appUrl);
      await preview.locator(".wind-root").waitFor();
      await preview.locator('.menu-trigger[aria-controls="menu-session"]').click();
      await preview.locator("#menu-session").getByRole("button", { name: messages.zh["elevated-local-terminal"], exact: true }).click();
      await preview.locator(".notice-dialog").waitFor();
      assert((await preview.locator(".notice-dialog").textContent()).includes(messages.zh["shell-elevation-desktop-only"]));
      assert.equal(confirmations.length, 3);
      assert.deepEqual(previewErrors, []);
    } finally { await previewContext.close(); }
    assert.deepEqual(errors, []);
  } finally {
    await page.close();
  }
}
