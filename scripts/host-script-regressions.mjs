export async function checkHostScripts(context, appUrl, screenshotPrefix) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.goto(appUrl);
  await page.waitForSelector('.terminal-host[data-terminal-size] .xterm-screen');
  await page.evaluate(() => {
    window.__mcpGrants.push({ ...window.__mcpGrants[0], clientId: "revoked-client", revokedAt: new Date().toISOString() });
    window.__customScripts[0].host.allowedClientIds.push("revoked-client", "missing-client");
  });
  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.getByRole("button", { name: "自定义脚本", exact: true }).click();
  const dialog = page.locator(".custom-script-dialog");
  await dialog.getByLabel("脚本正文", { exact: true }).waitFor();
  for (const id of ["revoked-client", "missing-client"]) {
    const checkbox = dialog.getByRole("checkbox", { name: "移除客户端 " + id, exact: true });
    assert(await checkbox.isEnabled(), id + " cannot be removed");
    await checkbox.click();
    await checkbox.waitFor({ state: "detached" });
    assert(await checkbox.count() === 0, id + " was not removed from the draft");
  }
  await dialog.getByRole("button", { name: "保存自定义脚本", exact: true }).click();
  await page.waitForFunction(() => window.__customScripts.every(script =>
    !script.host.allowedClientIds.includes("revoked-client") && !script.host.allowedClientIds.includes("missing-client")));
  assert(await dialog.getByLabel("运行脚本的会话").count() === 0, "host skills still target terminal sessions");
  await dialog.getByRole("button", { name: "添加自定义脚本", exact: true }).click();
  assert(await dialog.getByLabel("脚本语言").inputValue() === "python", "new skills must default to Python");
  await dialog.getByLabel("脚本名称", { exact: true }).fill("Host JSON skill");
  await dialog.getByLabel("脚本正文", { exact: true }).fill("import json,sys\nprint(json.load(sys.stdin))");
  await dialog.getByRole("button", { name: "添加参数", exact: true }).click();
  await dialog.getByLabel("参数 1 名称", { exact: true }).fill("message");
  await dialog.getByLabel("参数 1 说明", { exact: true }).fill("Input text");
  await dialog.getByRole("checkbox", { name: "开放给选定 MCP 客户端", exact: true }).check();
  await dialog.getByRole("button", { name: "保存自定义脚本", exact: true }).click();
  await dialog.getByRole("alert").filter({ hasText: "请选择至少一个 MCP 客户端" }).waitFor();
  await dialog.locator('[aria-label="脚本允许 MCP 客户端"] input').first().check();
  await dialog.getByRole("button", { name: "保存自定义脚本", exact: true }).click();
  await page.waitForFunction(() => window.__customScripts.some((s) => s.name === "Host JSON skill"));
  const saved = await page.evaluate(() => window.__customScripts.find((s) => s.name === "Host JSON skill"));
  assert(saved.host.allowedClientIds.length === 1 && saved.host.parameters[0].name === "message" && !Object.hasOwn(saved, "allowedSessionIds"), "host save contract was lost");
  await dialog.getByLabel("脚本运行参数").fill('{"message":"$(whoami); quoted"}');
  await page.evaluate(() => { window.__deferCustomScriptRuns = true; });
  await dialog.getByRole("button", { name: "运行自定义脚本", exact: true }).click();
  await page.waitForFunction(() => window.__pendingCustomScriptRuns.length === 1);
  const request = await page.evaluate(() => window.__invokeCalls.filter((c) => c.command === "run_host_script").at(-1).args.request);
  assert(!Object.hasOwn(request, "sessionId") && !Object.hasOwn(request, "content") && request.parameters.message === "$(whoami); quoted", "run input must remain data without source/target overrides");
  await dialog.getByRole("button", { name: "停止运行", exact: true }).click();
  await dialog.getByRole("alert").filter({ hasText: "cancelled" }).waitFor();
  await page.evaluate(() => { window.__deferCustomScriptRuns = false; });
  await dialog.getByRole("button", { name: "运行自定义脚本", exact: true }).click();
  await page.locator(".notice-dialog").getByRole("button", { name: "确定", exact: true }).click();
  await dialog.getByLabel("脚本运行结果").filter({ hasText: "host result" }).waitFor();
  await dialog.getByLabel("脚本语言").selectOption("shell");
  assert(await dialog.getByRole("button", { name: "运行自定义脚本", exact: true }).isDisabled(), "unsaved runtime change allowed execution");
  await dialog.getByRole("button", { name: "保存自定义脚本", exact: true }).click();
  await page.waitForFunction(() => window.__customScripts.find((s) => s.name === "Host JSON skill").host.language === "shell");
  await page.screenshot({ path: `${screenshotPrefix}-host-script.png`, fullPage: true });
  assert(errors.length === 0, `host script browser errors: ${errors.join("; ")}`);
  await page.close();
}
