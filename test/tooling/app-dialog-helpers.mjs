export async function respondAppConfirm(page, accept = true) {
  const dialog = page.locator('.app-dialog[data-app-dialog="confirm"]');
  await dialog.waitFor();
  const message = (await dialog.locator(".app-dialog-message").textContent()) ?? "";
  if (accept) {
    await dialog.locator('button[type="submit"]').click();
  } else {
    await dialog.locator('footer button[type="button"]').click();
  }
  await dialog.waitFor({ state: "detached" });
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  return message;
}

export async function respondAppPrompt(page, value) {
  const dialog = page.locator('.app-dialog[data-app-dialog="prompt"]');
  await dialog.waitFor();
  if (value === null) {
    await dialog.locator('footer button[type="button"]').click();
  } else {
    await dialog.locator('input[type="text"]').fill(value);
    await dialog.locator('button[type="submit"]').click();
  }
  await dialog.waitFor({ state: "detached" });
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}
