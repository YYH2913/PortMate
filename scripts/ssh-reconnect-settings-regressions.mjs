import assert from "node:assert/strict";

export async function checkSshReconnectSettings(context, appUrl) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.goto(appUrl);
    await page.locator(".wind-root").waitFor();
    const openSettings = async () => {
      await page.locator(".menu-trigger", { hasText: "会话" }).click();
      await page.locator(".menu-popover button", { hasText: "会话设置" }).click();
      const dialog = page.locator(".session-settings-dialog");
      await dialog.waitFor();
      await dialog.getByRole("treeitem", { name: "SSH", exact: true }).click();
      return dialog;
    };
    const ignoreToggle = dialog => dialog.locator(".dialog-field", { hasText: "自动重连时忽略指纹变化:" }).getByRole("button");
    let dialog = await openSettings();
    const reconnectToggle = dialog.locator(".dialog-field", { hasText: /^自动重连:$/ }).getByRole("button");
    assert.equal(await ignoreToggle(dialog).getAttribute("aria-pressed"), "false", "bypass must default to off");
    if (await reconnectToggle.getAttribute("aria-pressed") !== "true") await reconnectToggle.click();
    await reconnectToggle.click();
    assert(await ignoreToggle(dialog).isDisabled(), "bypass must be disabled when reconnect is off");
    await reconnectToggle.click();
    await ignoreToggle(dialog).click();
    assert.equal(await ignoreToggle(dialog).getAttribute("aria-pressed"), "true");
    assert((await dialog.textContent()).includes("可能连接到冒充服务器"), "bypass risk hint is missing");
    await dialog.getByRole("button", { name: "保存", exact: true }).click();
    await dialog.waitFor({ state: "detached" });
    const saved = await page.evaluate(() => window.__invokeCalls.findLast(call => call.command === "save_session_profile")?.args.profile.connection);
    assert.equal(saved.reconnectIgnoreHostKeyChanges, true, "save omitted reconnect fingerprint preference");
    assert.equal(saved.reconnect, true);
    dialog = await openSettings();
    assert.equal(await ignoreToggle(dialog).getAttribute("aria-pressed"), "true", "reopening settings lost the saved preference");
    await ignoreToggle(dialog).click();
    await dialog.getByRole("button", { name: "保存", exact: true }).click();
    await dialog.waitFor({ state: "detached" });
    const disabled = await page.evaluate(() => window.__invokeCalls.findLast(call => call.command === "save_session_profile")?.args.profile.connection.reconnectIgnoreHostKeyChanges);
    assert.equal(disabled, false, "disabling the preference did not persist");
    assert.deepEqual(errors, [], "reconnect settings produced browser exceptions");
  } finally {
    await page.close();
  }
}
