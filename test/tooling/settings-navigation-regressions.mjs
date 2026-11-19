import assert from "node:assert/strict";

export async function checkSettingsNavigation(context, appUrl, prefix) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", e => errors.push(e.message));
  try {
    await page.goto(appUrl);
    await page.locator('.terminal-host[data-terminal-ready="true"]').first().waitFor();
    const dialog = page.locator(".session-settings-dialog");
    const app = page.locator(".terminal-settings-dialog");
    async function menu(group, choice) {
      await page.locator(".menu-trigger", {hasText: group}).click();
      await page.locator(".menu-popover").getByRole("button", {name: choice, exact: true}).click();
    }
    const calls = () => page.evaluate(() => window.__invokeCalls.map(c => c.command));
    const mark = (await calls()).length;
    // The Explorer action must bind the selected serial profile, regardless of the active SSH tab.
    await page.locator(".panel-explorer .tree-session", {hasText:"Bench UART"}).click({button:"right"});
    await page.locator(".portmate-context-menu").getByRole("button", {name:/会话设置/}).click();
    assert.match(await dialog.getAttribute("aria-label"), /Bench UART/);
    const search = dialog.getByRole("searchbox");
    const name = dialog.getByLabel("名称:(N)", {exact:true});
    await name.fill("Bench preview");
    await search.fill("波特率");
    assert.equal(await name.inputValue(), "Bench preview"); // typing a search must not navigate
    await search.press("Enter");
    const baud = dialog.locator('input[list="serial-baud-rate-options"]');
    assert(await baud.evaluate(el => el === document.activeElement));
    await baud.fill("230400");
    await search.fill("nothing matches this");
    await search.press("Escape");
    assert.equal(await baud.inputValue(), "230400");
    await search.fill("字号");
    await search.press("Enter");
    assert(await dialog.getByLabel("字号:(Z)", {exact:true}).evaluate(el => el === document.activeElement));
    await dialog.locator(".dialog-field", {hasText:"主题:"}).locator("select").selectOption("graphite");
    await dialog.getByRole("button", {name:"取消",exact:true}).click();
    await dialog.getByRole("button", {name:"继续编辑"}).click();
    assert.equal(await dialog.locator(".dialog-field", {hasText:"主题:"}).locator("select").inputValue(), "graphite");
    await dialog.getByRole("button", {name:"保存",exact:true}).click();
    await dialog.waitFor({state:"detached"});
    const saved = await page.evaluate(() => window.__invokeCalls.findLast(c=>c.command === "save_session_profile").args.profile);
    assert.equal(saved.id, "bench-uart"); assert.equal(saved.connection.baudRate, 230400); assert.equal(saved.terminal.theme, "graphite");
    assert(!(await calls()).slice(mark).some(c=>["send_text","send_bytes","close_session","open_session"].includes(c)));

    // Check the tag context independently without sending a reconnect to either profile.
    await page.locator('.workspace-pane-tab[data-view-id="view-edge"]').click({button:"right"});
    await page.locator(".portmate-context-menu").getByRole("button", {name:/会话设置/}).click();
    assert.match(await dialog.getAttribute("aria-label"), /Edge Router/);
    await search.fill("自动重连");
    await search.press("Enter");
    assert(await dialog.locator('.dialog-toggle-field', {hasText:/^自动重连:$/}).getByRole('button').evaluate(el=>el===document.activeElement));
    await dialog.getByRole("button", {name:"取消",exact:true}).click();

    await menu("工具", "应用设置");
    await app.getByLabel("界面语言", {exact:true}).selectOption("fr");
    await app.getByRole("button", {name:"取消",exact:true}).click();
    await app.getByRole("button", {name:"放弃更改"}).click();
    assert.equal(await page.locator("html").getAttribute("lang"), "zh-CN", "discarded language changed the running app");
    await menu("工具", "应用设置");
    await app.getByRole("searchbox").fill("配色");
    assert.equal(await app.getByRole("heading", {name:"应用",exact:true}).count(),1);
    await app.getByRole("searchbox").press("ArrowDown");
    assert(await app.locator(".settings-results button", {hasText:"命令与输出自动多色"}).evaluate(el=>el===document.activeElement));
    await page.keyboard.press("Enter");
    const toggle = app.getByLabel("命令与输出自动多色", {exact:true});
    assert(await toggle.evaluate(el=>el===document.activeElement));
    await toggle.uncheck();
    await app.getByRole("button", {name:"保存",exact:true}).click();
    await app.waitFor({state:"detached"});
    const prefs = await page.evaluate(() => JSON.parse(localStorage.getItem("portmate.terminalPrefs")));
    assert.equal(prefs.semanticHighlightingEnabled,false);
    await menu("会话", "新建会话");
    await dialog.getByRole("tab", {name:"SSH",exact:true}).click();
    await dialog.getByRole("textbox", {name:"SSH 主机或 IP",exact:true}).fill("demo.invalid");
    assert(await dialog.getByRole("button", {name:"保存并连接",exact:true}).isEnabled());
    await dialog.getByRole("button", {name:"高级设置"}).click();
    await dialog.getByRole("treeitem", {name:"终端外观"}).click();
    assert.match(await dialog.locator(".settings-scope-note").textContent(), /已关闭/);
    await page.setViewportSize({width:650, height:660});
    assert(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth && el.getBoundingClientRect().width <= innerWidth));
    await dialog.screenshot({path:`${prefix}-appearance.png`});
    await search.fill("字号");
    await dialog.screenshot({path:`${prefix}-search.png`});
    await dialog.getByRole("button", {name:"取消",exact:true}).click();
    await dialog.getByRole("button", {name:"放弃更改"}).click();
    await dialog.waitFor({state:"detached"});
    assert.equal((await calls()).slice(mark).filter(c=>c === "save_session_profile").length,1);
    assert.deepEqual(errors,[]);
  } finally { await page.close(); }
}
