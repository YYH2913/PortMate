import assert from "node:assert/strict";

/** Real pointer gestures: synthetic HTML drag/drop misses Windows native-drop interception. */
export async function checkWorkspaceDragging(context, appUrl, screenshotPrefix = "/tmp/portmate-workspace-drag") {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.goto(appUrl);
    await page.locator('.terminal-host[data-terminal-ready="true"]').first().waitFor();
    const message = key => page.evaluate(async key => (await import("/src/i18n.ts")).t(key), key);
    const dock = id => page.locator(`.workspace-dock[data-dock="${id}"]`);
    const dockTab = id => page.locator(`.workspace-dock-tab[data-panel="${id}"]`);
    const viewTab = id => page.locator(`.workspace-pane-tab[data-view-id="${id}"]`);
    const panels = () => page.evaluate(() => JSON.parse(localStorage.getItem("portmate.workspacePanels.v2")));
    const workspace = () => page.evaluate(() => JSON.parse(localStorage.getItem("portmate.workspace.v1")));
    const clean = () => page.waitForFunction(() => !document.querySelector(".workspace-pointer-dragging, [data-panel-drop-position], [data-drop-position], [data-view-drop-zone], [data-pointer-dock-target], [data-dock-target]"), null, { timeout: 5000 });
    async function toggle(key) {
      await page.locator('.menu-trigger[aria-controls="menu-workspace"]').click();
      await page.locator("#menu-workspace").getByRole("button", { name: await message(key), exact: true }).click();
    }
    async function point(locator, x = 0.5, y = 0.5) {
      const bounds = await locator.boundingBox();
      assert(bounds, `missing drag geometry: ${locator}`);
      return { x: bounds.x + bounds.width * x, y: bounds.y + bounds.height * y };
    }
    async function start(locator) {
      const p = await point(locator);
      await page.mouse.move(p.x, p.y);
      await page.mouse.down();
      await page.mouse.move(p.x + 8, p.y + 8, { steps: 3 });
      await page.locator(".workspace-pointer-dragging").waitFor();
    }
    async function releaseAt(locator, x = 0.5, y = 0.5) {
      const p = await point(locator, x, y);
      await page.mouse.move(p.x, p.y, { steps: 12 });
      await page.mouse.up();
      await clean();
    }
    async function drag(source, target, x = 0.5, y = 0.5) {
      await start(source);
      await releaseAt(target, x, y);
    }
    await page.locator(".tree-session", { hasText: "Bench UART" }).click();
    await page.locator(".workspace-pane-tab", { hasText: "Bench UART" }).waitFor();
    const benchId = await page.locator(".workspace-pane-tab", { hasText: "Bench UART" }).getAttribute("data-view-id");
    await toggle("file-manager");
    await toggle("command-history");
    await toggle("send");
    await dock("right").waitFor();
    await dock("bottom").waitFor();
    await page.evaluate(() => {
      window.__workspaceNativeDragStarts = 0;
      // Reproduce the effective Windows restriction: HTML dragstart cannot start a drop.
      document.addEventListener("dragstart", event => { window.__workspaceNativeDragStarts += 1; event.preventDefault(); }, true);
      document.addEventListener("pointerdown", event => { window.__workspaceTestPointerId = event.pointerId; }, true);
    });
    const transportBaseline = await page.evaluate(() => window.__invokeCalls.length);

    // A sub-threshold gesture must still act as a normal tab click.
    const clickPoint = await point(dockTab("explorer").locator(".workspace-dock-tab-label"));
    await page.mouse.move(clickPoint.x, clickPoint.y);
    await page.mouse.down();
    await page.mouse.move(clickPoint.x + 2, clickPoint.y + 1);
    await page.mouse.up();
    assert.equal(await dock("left").getAttribute("data-active-panel"), "explorer");
    await clean();

    await drag(dockTab("fileManager").locator(".workspace-dock-tab-label"), dockTab("explorer"), 0.2);
    assert.deepEqual((await panels()).docks.left, ["fileManager", "explorer"]);
    await drag(dockTab("fileManager").locator(".workspace-dock-tab-label"), dockTab("explorer"), 0.8);
    assert.deepEqual((await panels()).docks.left, ["explorer", "fileManager"]);

    // Hidden sysmon is first in the persisted array, not in the visible tab list.
    await drag(dockTab("sender").locator(".workspace-dock-tab-label"), dockTab("history"), 0.8);
    assert.deepEqual((await panels()).docks.right, ["sysmon", "history", "sender"], "visible target index was confused with hidden panel order");
    assert.equal(await dock("bottom").count(), 0);

    await page.evaluate(async () => (await import("/src/i18n.ts")).setLanguagePreference("ar"));
    assert.equal(await dockTab("history").evaluate(element => getComputedStyle(element).direction), "rtl");
    await drag(dockTab("sender").locator(".workspace-dock-tab-label"), dockTab("history"), 0.8);
    assert.deepEqual((await panels()).docks.right, ["sysmon", "sender", "history"], "RTL before drop used the LTR edge");
    await drag(dockTab("sender").locator(".workspace-dock-tab-label"), dockTab("history"), 0.2);
    assert.deepEqual((await panels()).docks.right, ["sysmon", "history", "sender"], "RTL after drop used the LTR edge");
    await page.evaluate(async () => (await import("/src/i18n.ts")).setLanguagePreference("zh"));

    for (const cancellation of ["escape", "blur", "pointercancel", "lostcapture"]) {
      const before = await panels();
      await start(dockTab("sender").locator(".workspace-dock-tab-label"));
      const p = await point(page.locator('[data-dock-target="left"]'));
      await page.mouse.move(p.x, p.y, { steps: 10 });
      if (cancellation === "escape") await page.keyboard.press("Escape");
      else await page.evaluate(kind => {
        if (kind === "blur") window.dispatchEvent(new Event("blur"));
        else if (kind === "pointercancel") window.dispatchEvent(new PointerEvent("pointercancel", { pointerId: window.__workspaceTestPointerId }));
        else document.querySelector(".wind-root").releasePointerCapture(window.__workspaceTestPointerId);
      }, cancellation);
      await page.mouse.up();
      await clean();
      assert.deepEqual(await panels(), before, `${cancellation} committed an unfinished drag`);
    }
    await dockTab("history").locator(".workspace-dock-tab-label").click();
    assert.equal(await dock("right").getAttribute("data-active-panel"), "history", "cancelled drag swallowed the next real click");
    await start(dockTab("sender").locator(".workspace-dock-tab-label"));
    await releaseAt(page.locator('[data-dock-target="bottom"]'));
    assert.deepEqual((await panels()).docks.bottom, ["sender"], "pointer drag could not create an empty dock");

    // Both pointer and keyboard resize must be based on visible, viewport-limited geometry.
    for (const id of ["left", "right", "bottom"]) {
      const resizer = dock(id).locator(".workspace-dock-resizer");
      const measure = async () => {
        const bounds = await dock(id).boundingBox();
        return id === "bottom" ? bounds.height : bounds.width;
      };
      await resizer.focus();
      await resizer.press("End");
      const maximum = await measure();
      await resizer.press(id === "left" ? "ArrowLeft" : id === "right" ? "ArrowRight" : "ArrowDown");
      assert(Math.abs(maximum - await measure() - 16) <= 2, `${id} keyboard resize had a dead zone after End`);
      await resizer.dblclick();
      const before = await measure();
      const p = await point(resizer);
      const delta = id === "left" ? 48 : -48;
      const end = id === "bottom" ? { x: p.x, y: p.y + delta } : { x: p.x + delta, y: p.y };
      await page.mouse.move(p.x, p.y);
      await page.mouse.down({ button: "right" });
      await page.mouse.move(end.x, end.y, { steps: 5 });
      await page.mouse.up({ button: "right" });
      assert.equal(await measure(), before, `${id} resize accepted a secondary mouse button`);
      // Releasing the right button over a terminal may open its context menu.
      // Close that independent interaction before starting the next left-button drag.
      await page.locator(".portmate-context-menu").waitFor();
      await page.keyboard.press("Escape");
      await page.locator(".portmate-context-menu").waitFor({ state: "detached" });
      await page.mouse.move(p.x, p.y);
      await page.mouse.down();
      await page.mouse.move(end.x, end.y, { steps: 8 });
      await page.mouse.up();
      const resized = await measure();
      assert(resized - before >= 44, `${id} pointer resize did not change visible geometry: ${JSON.stringify({ before, resized, p, end, panels: await panels() })}`);
      await page.mouse.move(end.x + 70, end.y + 20, { steps: 5 });
      assert.equal(await measure(), resized, `${id} resize continued after pointerup`);
      await resizer.dblclick();
    }

    await drag(viewTab(benchId).locator(".workspace-pane-tab-label"), viewTab("view-edge"), 0.2);
    assert.deepEqual((await workspace()).root.views.map(view => view.id), [benchId, "view-edge"], "real view-tab drag did not reorder its group");
    const pane = page.locator(".terminal-pane");
    await start(viewTab("view-edge").locator(".workspace-pane-tab-label"));
    const splitPoint = await point(pane, 0.98, 0.55);
    await page.mouse.move(splitPoint.x, splitPoint.y, { steps: 12 });
    assert.equal(await pane.getAttribute("data-view-drop-zone"), "right");
    await page.mouse.up();
    await page.waitForFunction(() => document.querySelectorAll(".terminal-pane").length === 2);
    await clean();
    const splitter = page.locator(".terminal-splitter");
    const splitBox = await page.locator(".terminal-split").boundingBox();
    const s = await point(splitter);
    await page.mouse.move(s.x, s.y);
    await page.mouse.down();
    await page.mouse.move(splitBox.x + splitBox.width * 0.65, s.y, { steps: 10 });
    await page.mouse.up();
    assert(Math.abs((await workspace()).root.ratio - 0.65) < 0.015, "split divider did not persist real pointer movement");
    const splitRatio = (await workspace()).root.ratio;
    await page.mouse.move(s.x, s.y);
    assert.equal((await workspace()).root.ratio, splitRatio, "splitter kept resizing after release");
    await page.screenshot({ path: `${screenshotPrefix}-pointer-split.png`, fullPage: true });
    // The target pane is inactive; its inert terminal must not block a group drop.
    const benchPane = page.locator(".terminal-pane", { has: viewTab(benchId) });
    await drag(viewTab("view-edge").locator(".workspace-pane-tab-label"), benchPane, 0.5, 0.55);
    await page.waitForFunction(() => document.querySelectorAll(".terminal-pane").length === 1);
    assert.deepEqual((await workspace()).root.views.map(view => view.id), [benchId, "view-edge"]);
    assert.equal(await page.evaluate(() => window.__workspaceNativeDragStarts), 0, "internal dragging still relied on native HTML dragstart");
    assert.deepEqual(await page.evaluate(start => window.__invokeCalls.slice(start).filter(call => ["send_text", "connect_session", "disconnect_session", "reconnect_session"].includes(call.command)), transportBaseline), [], "layout gestures changed transport state or sent input");
    const savedPanels = await panels();
    const savedWorkspace = await workspace();
    await page.reload();
    await page.locator('.terminal-host[data-terminal-ready="true"]').first().waitFor();
    assert.deepEqual(await panels(), savedPanels, "dock order did not survive reload");
    assert.deepEqual(await workspace(), savedWorkspace, "view order did not survive reload");
    assert.deepEqual(errors, [], "pointer workspace gestures caused a browser error");
  } catch (error) {
    console.error("Workspace pointer failure context:", await page.evaluate(() => ({
      overlays: [...document.querySelectorAll(".dialog-backdrop, .screen-lock-overlay, .mcp-approval-backdrop, .menu-popover, .portmate-context-menu")].map(element => ({ cls: element.className, text: element.textContent, visible: !!element.getClientRects().length })),
      active: document.activeElement?.outerHTML,
      errors: window.__invokeCalls.slice(-8),
    })));
    await page.screenshot({ path: `${screenshotPrefix}-pointer-failure.png`, fullPage: true });
    throw error;
  } finally {
    await page.mouse.up().catch(() => {});
    await page.close();
  }
}
