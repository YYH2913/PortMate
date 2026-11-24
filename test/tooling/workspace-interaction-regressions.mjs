import assert from "node:assert/strict";

export async function checkWorkspaceInteractions(context, appUrl) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.goto(appUrl);
    await page.locator('.terminal-host[data-terminal-ready="true"]').first().waitFor();
    const label = key => page.evaluate(async key => (await import("/src/i18n.ts")).t(key), key);
    const chooseDock = async key => {
      await page.locator('[aria-controls="menu-workspace"]').click();
      await page.locator('#menu-workspace').getByRole("button", { name: await label(key), exact: true }).click();
    };
    const treeRow = page.getByRole("button", { name: "Edge Router", exact: true }).and(page.locator(".tree-session"));
    await treeRow.click({ button: "right" });
    const menu = page.locator(".portmate-context-menu");
    const trigger = menu.getByRole("button", { name: await label("set-tab-color"), exact: true });
    await trigger.hover();
    const submenu = menu.locator(".context-submenu-panel:visible");
    await submenu.waitFor();
    const first = submenu.locator("button").first();
    const hit = await first.evaluate(button => {
      const rect = button.getBoundingClientRect();
      const x = rect.x + rect.width / 2, y = rect.y + rect.height / 2;
      return { x, y, inside: button.contains(document.elementFromPoint(x, y)) };
    });
    assert(hit.inside, `submenu painted outside its hit area: ${JSON.stringify(hit)}`);
    await page.mouse.move(hit.x, hit.y, { steps: 18 });
    assert(await first.isVisible(), "moving from the trigger to the submenu dismissed it");
    await first.focus();
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowUp");
    assert(await submenu.locator("button").last().evaluate(button => button === document.activeElement), "ArrowUp escaped the submenu");
    await page.keyboard.press("Escape");
    await submenu.waitFor({ state: "hidden" });
    await trigger.press("ArrowRight");
    await first.waitFor();
    await first.click();
    await menu.waitFor({ state: "detached" });

    // A short desktop window must allow access to the bottom rows by wheel.
    await page.setViewportSize({ width: 1050, height: 400 });
    await treeRow.click({ button: "right" });
    await menu.hover();
    await page.mouse.wheel(0, 900);
    await page.waitForFunction(() => document.querySelector(".portmate-context-menu")?.scrollTop > 0);
    await page.keyboard.press("Escape");
    await menu.waitFor({ state: "detached" });

    await page.setViewportSize({ width: 550, height: 650 });
    await chooseDock("explorer");
    await page.locator(".workspace-dock.mobile-drawer-open").waitFor();
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.locator(".mobile-dock-backdrop").waitFor({ state: "detached" });

    // A prompt can outlive the connection that opened it. It must not submit
    // a mutation to the replacement connection, even with the same profile ID.
    await chooseDock("file-manager");
    const remote = page.locator('[data-file-pane="remote"]');
    const create = remote.getByRole("button", { name: await label("new-file"), exact: true });
    await create.click();
    const prompt = page.locator('.app-dialog[data-app-dialog="prompt"]');
    await prompt.locator("input").fill("stale-file");
    const baseline = await page.evaluate(() => {
      const edge = window.__sessions.find(item => item.profile.id === "edge-router");
      edge.runtime.connectedSince = "2030-01-01T00:00:00Z";
      const count = window.__invokeCalls.filter(call => call.command === "list_files" && call.args.request.remote).length;
      window.__emitTauriEvent("portmate-session-profile-updated", structuredClone(edge));
      return count;
    });
    await page.waitForFunction(n => window.__invokeCalls.filter(call => call.command === "list_files" && call.args.request.remote).length > n, baseline);
    await prompt.locator('button[type="submit"]').click();
    await prompt.waitFor({ state: "detached" });
    assert.equal(await page.evaluate(() => window.__invokeCalls.some(call => call.command === "create_file" && call.args.request.path.endsWith("stale-file"))), false);

    // While another modal is open, cancel a nested destructive confirmation
    // with Enter and restore keyboard focus to the exact originating control.
    await page.locator('[aria-controls="menu-tools"]').click();
    await page.locator('#menu-tools').getByRole("button", { name: await label("custom-scripts"), exact: true }).click();
    const parent = page.locator(".custom-script-dialog");
    const button = parent.locator(".dialog-title button");
    await button.focus();
    await page.evaluate(async () => {
      const { requestAppConfirm } = await import("/src/app-dialog.ts");
      window.__nestedDecision = requestAppConfirm({ message: "Discard draft?", danger: true });
    });
    const confirm = page.locator('.app-dialog[data-app-dialog="confirm"]');
    await confirm.waitFor();
    await page.waitForFunction(() => document.activeElement?.matches('.app-dialog footer button[type="button"]'));
    await page.keyboard.press("Enter");
    assert.equal(await page.evaluate(() => window.__nestedDecision), false);
    await page.waitForFunction(() => document.activeElement?.matches('.custom-script-dialog .dialog-title button'));
    await button.click();
    await parent.waitFor({ state: "detached" });

    await page.locator('.workspace-dock-tab[data-panel="explorer"] [role="tab"]').click();
    const lab = page.locator('.tree-folder[title="Lab"]');
    await lab.click();
    assert.equal(await lab.getAttribute("aria-expanded"), "false");
    await treeRow.click({ button: "right" });
    await menu.getByText(await label("disconnect-sessions-below"), { exact: true }).click();
    await page.waitForFunction(() => window.__invokeCalls.some(call => call.command === "close_session" && call.args.sessionId === "local-shell"));
    assert.equal(await page.evaluate(() => window.__invokeCalls.some(call => call.command === "close_session" && call.args.sessionId === "bench-uart")), false,
      "disconnect-below must not disconnect sessions in collapsed groups");
    await lab.click();
  } finally {
    await page.close();
  }
  assert.deepEqual(errors, []);
}
