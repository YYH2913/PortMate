import assert from "node:assert/strict";

export async function checkWorkspaceDragOverflow(context, appUrl) {
  const page = await context.newPage();
  const errors = [];
  let savedWorkspace;
  page.on("pageerror", error => errors.push(error.message));
  const snapshot = () => page.evaluate(() => JSON.parse(localStorage.getItem("portmate.workspace.v1")));
  try {
    await page.goto(appUrl);
    await page.locator('.terminal-host[data-terminal-ready="true"]').first().waitFor();
    savedWorkspace = await snapshot();
    await page.evaluate(() => {
      const workspace = JSON.parse(localStorage.getItem("portmate.workspace.v1"));
      function firstPane(node) { return node.kind === "pane" ? node : firstPane(node.first); }
      const pane = firstPane(workspace.root);
      const base = pane.views[0];
      workspace.root = {
        ...pane,
        views: Array.from({ length: 32 }, (_, index) => ({ ...base, id: `overflow-view-${index}`, title: `Overflow ${index}` })),
        activeViewId: "overflow-view-0",
      };
      workspace.activePaneId = pane.id;
      localStorage.setItem("portmate.workspace.v1", JSON.stringify(workspace));
    });
    await page.reload();
    await page.locator('.workspace-pane-tab[data-view-id="overflow-view-0"]').waitFor();
    await page.evaluate(() => {
      document.addEventListener("pointerdown", event => { window.__overflowPointerId = event.pointerId; }, true);
      window.__overflowKeyboardClicks = [];
      document.addEventListener("click", event => {
        if (event.target.closest?.(".workspace-pane-tab-label") && event.detail === 0) {
          window.__overflowKeyboardClicks.push({ prevented: event.defaultPrevented });
        }
      }, true);
    });
    const strip = page.locator(".workspace-pane-tabs");
    const tab = id => page.locator(`.workspace-pane-tab[data-view-id="${id}"]`);
    const clean = () => page.waitForFunction(() => !document.querySelector(".workspace-pointer-dragging, [data-drop-position], [data-view-drop-zone]"));
    async function start(id) {
      const bounds = await tab(id).locator(".workspace-pane-tab-label").boundingBox();
      assert(bounds);
      await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
      await page.mouse.down();
      await page.mouse.move(bounds.x + bounds.width / 2 + 8, bounds.y + bounds.height / 2, { steps: 3 });
      await page.locator(".workspace-pointer-dragging").waitFor();
    }
    async function edge(right) {
      const bounds = await strip.boundingBox();
      assert(bounds);
      await page.mouse.move(bounds.x + (right ? bounds.width - 2 : 2), bounds.y + bounds.height / 2, { steps: 10 });
    }
    const baseline = (await snapshot()).root.views.map(view => view.id);
    assert(await strip.evaluate(element => element.scrollWidth > element.clientWidth + 200), "overflow fixture does not overflow");

    await start("overflow-view-0");
    await edge(true);
    await page.waitForFunction(() => document.querySelector(".workspace-pane-tabs").scrollLeft > 100);
    const moving = await strip.evaluate(element => element.scrollLeft);
    await page.waitForTimeout(120);
    assert(await strip.evaluate(element => element.scrollLeft) > moving, "scrolling stopped when the pointer stopped moving");
    await page.waitForFunction(() => {
      const element = document.querySelector(".workspace-pane-tabs");
      return element.scrollLeft >= element.scrollWidth - element.clientWidth - 1;
    });
    await page.mouse.up();
    await clean();
    assert.equal((await snapshot()).root.views.at(-1).id, "overflow-view-0", "drop at scrolled edge did not use the newly revealed tab position");
    const released = await strip.evaluate(element => element.scrollLeft);
    await page.waitForTimeout(90);
    assert.equal(await strip.evaluate(element => element.scrollLeft), released, "auto-scroll continued after pointerup");

    // Pointer cancellation must clear scheduled frames without committing a move.
    await start("overflow-view-0");
    await edge(false);
    await page.waitForFunction(previous => document.querySelector(".workspace-pane-tabs").scrollLeft < previous - 30, released);
    await page.evaluate(() => window.dispatchEvent(new PointerEvent("pointercancel", { pointerId: window.__overflowPointerId })));
    await clean();
    const cancelled = await strip.evaluate(element => element.scrollLeft);
    await page.waitForTimeout(90);
    assert.equal(await strip.evaluate(element => element.scrollLeft), cancelled, "auto-scroll continued after pointercancel");
    await page.mouse.up();
    assert.equal((await snapshot()).root.views.at(-1).id, "overflow-view-0", "cancelled scrolling committed a view move");

    // Reset only scroll position; Enter must still activate a tab during the
    // pointer-compatibility-click suppression window after Escape.
    await strip.evaluate(element => { element.scrollLeft = 0; });
    await start("overflow-view-1");
    await page.keyboard.press("Escape");
    await page.keyboard.press("Enter");
    assert.equal((await snapshot()).root.activeViewId, "overflow-view-1", "Escape then keyboard activation was swallowed");
    assert.deepEqual(await page.evaluate(() => window.__overflowKeyboardClicks), [{ prevented: false }]);
    await page.mouse.up();
    await clean();

    // Force the strip's direction, independent of the shell's physical LTR pane
    // geometry, to exercise the negative RTL scroll range as well.
    await strip.evaluate(element => { element.style.direction = "rtl"; element.scrollLeft = 0; });
    await start("overflow-view-1");
    await edge(false);
    await page.waitForFunction(() => document.querySelector(".workspace-pane-tabs").scrollLeft < -100);
    await page.keyboard.press("Escape");
    await clean();
    const rtlCancelled = await strip.evaluate(element => element.scrollLeft);
    await page.waitForTimeout(90);
    assert.equal(await strip.evaluate(element => element.scrollLeft), rtlCancelled, "RTL auto-scroll continued after Escape");
    await page.mouse.up();
    assert.deepEqual((await snapshot()).root.views.map(view => view.id).sort(), baseline.slice().sort(), "overflow dragging lost or duplicated a view");
    assert.deepEqual(errors, [], "overflow drag produced browser exceptions");
  } finally {
    await page.mouse.up().catch(() => {});
    if (savedWorkspace) await page.evaluate(saved => localStorage.setItem("portmate.workspace.v1", JSON.stringify(saved)), savedWorkspace).catch(() => {});
    await page.close();
  }
}
