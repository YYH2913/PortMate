import assert from "node:assert/strict";

export async function checkMcpManagement(context, appUrl, screenshotPrefix) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  const dialog = page.locator(".mcp-dialog");
  const confirm = page.getByRole("alertdialog", { name: "确认撤销授权", exact: true });
  const records = () => page.evaluate(() => window.__invokeCalls.map(call => ({ command: call.command, args: call.args })));
  const mutations = records => records.filter(call => ["save_mcp_grant", "revoke_mcp_grant", "save_mcp_http_settings", "rotate_mcp_http_token", "start_mcp_http", "stop_mcp_http"].includes(call.command));
  const tab = name => dialog.getByRole("tab", { name, exact: true }).click();
  const select = name => dialog.locator(".mcp-grant-select", { hasText: name }).click();
  const revoke = () => dialog.getByRole("button", { name: "撤销", exact: true }).click();
  const start = () => dialog.getByRole("button", { name: /^(保存并启动|生成 Token 并启动|启动服务)$/ }).click();
  const stop = () => dialog.getByRole("button", { name: "停止服务", exact: true }).click();
  const setConfirm = value => page.evaluate(value => { window.__mcpConfirmResult = value; }, value);
  async function createGrant(id, name) {
    await tab("授权");
    await dialog.locator(".mcp-new").click();
    await dialog.getByLabel("MCP 授权 Client ID", { exact: true }).fill(id);
    await dialog.locator(".dialog-field", { hasText: "名称:" }).locator("input").fill(name);
    await dialog.getByRole("button", { name: "保存并前往 HTTP", exact: true }).click();
    await dialog.locator(".mcp-http-view").waitFor();
  }
  async function bounds(selector) {
    const result = await page.locator(selector).evaluate(element => {
      const rect = element.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom,
        scrollWidth: element.scrollWidth, clientWidth: element.clientWidth,
        viewportWidth: innerWidth, viewportHeight: innerHeight };
    });
    assert(result.left >= 0 && result.right <= result.viewportWidth + 1, `${selector} exceeds horizontal viewport`);
    assert(result.top >= 0 && result.bottom <= result.viewportHeight + 1, `${selector} exceeds vertical viewport`);
    assert(result.scrollWidth <= result.clientWidth + 1, `${selector} overflows horizontally`);
  }
  async function grantEditorLayout() {
    const result = await dialog.locator(".mcp-editor-shell").evaluate(shell => {
      const editor = shell.querySelector(".mcp-editor");
      const actions = shell.querySelector(".mcp-grant-actions");
      editor.scrollTop = editor.scrollHeight;
      const editorRect = editor.getBoundingClientRect();
      const actionsRect = actions.getBoundingClientRect();
      const noteRect = editor.querySelector(".mcp-bridge-binding-note").getBoundingClientRect();
      const shellRect = shell.getBoundingClientRect();
      return { editorBottom: editorRect.bottom, actionsTop: actionsRect.top, actionsBottom: actionsRect.bottom,
        noteBottom: noteRect.bottom, shellBottom: shellRect.bottom };
    });
    assert(result.editorBottom <= result.actionsTop + 1, "grant actions overlap the scrollable editor");
    assert(result.noteBottom <= result.editorBottom + 1, "grant instructions are hidden at the bottom of the editor");
    assert(result.actionsBottom <= result.shellBottom + 1, "grant actions extend below the dialog");
    await dialog.locator(".mcp-editor").evaluate(editor => { editor.scrollTop = 0; });
  }
  try {
    await page.goto(appUrl);
    await page.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
    await page.evaluate(() => {
      window.__mcpPrompts = [];
      window.__mcpConfirmResult = true;
      window.confirm = message => { window.__mcpPrompts.push(String(message)); return window.__mcpConfirmResult; };
    });
    await page.getByRole("button", { name: "工具", exact: true }).click();
    await page.getByRole("button", { name: "MCP Bridge", exact: true }).click();
    await dialog.locator(".mcp-grant-select").first().waitFor();
    assert.equal(await dialog.locator(".mcp-grant-select").count(), 2);
    assert.equal(await dialog.locator(".mcp-grant-copy, .mcp-grant-cc-switch").count(), 0);
    assert.equal(await dialog.getByLabel("CC Switch Bearer 令牌", { exact: true }).count(), 0);
    assert.deepEqual(await dialog.locator(".mcp-check-grid code").allTextContents(), [
      "read-sessions", "read-logs", "read-transfers", "read-tunnels", "read-scripts", "read-mcp",
      "write-input", "transfer", "host-files", "tunnel", "manage-sessions", "run-scripts", "manage-mcp",
    ]);
    await bounds(".mcp-dialog");
    await grantEditorLayout();
    await page.screenshot({ path: `${screenshotPrefix}-mcp-grants.png`, fullPage: true, animations: "disabled" });

    // Creating and switching grants preserve the same unsaved-draft boundary.
    await dialog.locator(".mcp-new").click();
    const grantId = dialog.getByLabel("MCP 授权 Client ID", { exact: true });
    assert.equal(await grantId.inputValue(), "");
    assert(await dialog.getByRole("button", { name: "保存", exact: true }).isDisabled());
    await dialog.getByRole("button", { name: "随机生成 Client ID", exact: true }).click();
    const randomId = await grantId.inputValue();
    assert.match(randomId, /^client-[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    await setConfirm(false);
    await select("Operations Console");
    assert.equal(await grantId.inputValue(), randomId, "cancelled switch discarded the grant draft");
    await tab("HTTP");
    await tab("授权");
    assert.equal(await grantId.inputValue(), randomId, "switching tabs discarded the grant draft");
    await setConfirm(true);
    await select("Operations Console");

    const beforeCancel = mutations(await records()).length;
    await select("Audit Reader");
    await setConfirm(false);
    await revoke();
    assert.equal(await confirm.count(), 0, "cancelled first confirmation opened the second step");
    await setConfirm(true);
    await revoke();
    await confirm.waitFor();
    const finalRevoke = confirm.getByRole("button", { name: "确认撤销", exact: true });
    assert(await finalRevoke.isDisabled());
    await confirm.getByLabel("输入 Client ID", { exact: true }).fill("wrong-client");
    assert(await finalRevoke.isDisabled(), "wrong ID enabled revocation");
    await confirm.getByLabel("输入 Client ID", { exact: true }).fill("audit-reader");
    await finalRevoke.focus();
    await page.keyboard.press("Tab");
    assert(await confirm.getByRole("button", { name: "取消撤销", exact: true }).evaluate(button => button === document.activeElement), "Tab did not wrap within revocation confirmation");
    await page.keyboard.press("Shift+Tab");
    assert(await finalRevoke.evaluate(button => button === document.activeElement), "Shift+Tab did not wrap within revocation confirmation");
    await confirm.getByRole("button", { name: "取消", exact: true }).click();
    assert.equal(mutations(await records()).length, beforeCancel, "cancelling confirmation mutated authorization");

    await tab("HTTP");
    assert.equal(await dialog.getByRole("combobox", { name: "MCP HTTP 客户端 ID", exact: true }).inputValue(), "ops-console");
    const serverId = dialog.getByLabel("CC Switch 服务端 ID", { exact: true });
    assert.equal(await serverId.inputValue(), "portmate-ops-console", "default Server ID did not follow the loaded HTTP binding");
    await start();
    await dialog.locator(".mcp-http-runtime.running").waitFor();
    await tab("授权");
    await select("Audit Reader");
    await revoke();
    await confirm.getByLabel("输入 Client ID", { exact: true }).fill("audit-reader");
    await finalRevoke.click();
    await dialog.locator(".mcp-grant-select", { hasText: "Audit Reader" }).waitFor({ state: "detached" });
    assert.deepEqual(await page.evaluate(() => [window.__mcpHttpRuntime.phase, window.__mcpHttpToken]), ["running", "portmate-existing-token"], "unrelated revocation changed HTTP access");

    await select("Operations Console");
    await revoke();
    assert((await page.evaluate(() => window.__mcpPrompts.at(-1))).includes("HTTP Bridge"));
    await confirm.getByLabel("输入 Client ID", { exact: true }).fill("ops-console");
    await page.screenshot({ path: `${screenshotPrefix}-mcp-revoke.png`, fullPage: true, animations: "disabled" });
    await page.evaluate(() => { window.__deferGrantMutations = true; });
    await finalRevoke.click();
    await page.waitForFunction(() => window.__pendingGrantMutations.length === 1);
    await page.keyboard.press("Escape");
    assert(await confirm.isVisible(), "Escape dismissed a running revocation");
    assert(await confirm.getByRole("button", { name: "取消", exact: true }).isDisabled());
    await confirm.dispatchEvent("submit");
    assert.equal((await records()).filter(call => call.command === "revoke_mcp_grant" && call.args.clientId === "ops-console").length, 1, "double submission revoked twice");
    await page.evaluate(() => {
      window.__deferGrantMutations = false;
      const pending = window.__pendingGrantMutations.shift(); pending.resolve(pending.result);
    });
    await confirm.waitFor({ state: "detached" });
    await dialog.locator(".mcp-editor-empty").waitFor();
    await tab("HTTP");
    assert.deepEqual(await page.evaluate(() => [window.__mcpHttpRuntime.phase, window.__mcpHttpToken]), ["stopped", null]);
    assert(await dialog.getByRole("button", { name: "生成 Token 并启动", exact: true }).isDisabled());
    assert(await dialog.getByRole("button", { name: "复制 CC Switch JSON", exact: true }).isDisabled());
    assert.equal(await dialog.getByRole("combobox", { name: "MCP HTTP 客户端 ID", exact: true }).inputValue(), "");

    await createGrant("new-client", "New Client");
    assert.equal(await page.evaluate(() => window.__mcpHttpConfig.clientId), "ops-console", "grant creation silently rebound the HTTP identity");
    const httpClient = dialog.getByRole("combobox", { name: "MCP HTTP 客户端 ID", exact: true });
    await httpClient.selectOption("new-client");
    assert.equal(await serverId.inputValue(), "portmate-new-client", "automatic Server ID did not follow the new binding");
    const beforeStart = (await records()).length;
    await start();
    await dialog.locator(".mcp-http-runtime.running").waitFor();
    assert.deepEqual(mutations((await records()).slice(beforeStart)).map(call => call.command), ["save_mcp_http_settings", "rotate_mcp_http_token", "start_mcp_http"]);
    assert(await httpClient.isDisabled());
    assert.equal(await page.evaluate(() => window.__mcpHttpToken), "portmate-test-token-1");
    await page.evaluate(() => { window.__clipboardWriteFailures = 1; });
    await dialog.getByRole("button", { name: "复制 CC Switch JSON", exact: true }).click();
    await dialog.getByRole("alert").filter({ hasText: "simulated clipboard denial" }).waitFor();
    const beforeCopy = mutations(await records()).length;
    await dialog.getByRole("button", { name: "复制 CC Switch JSON", exact: true }).click();
    await page.waitForFunction(() => window.__clipboardText.includes("Bearer portmate-test-token-1"));
    assert.deepEqual(await page.evaluate(() => Object.keys(JSON.parse(window.__clipboardText))), ["portmate-new-client"]);
    await serverId.fill("portmate-custom-name");
    await dialog.getByRole("button", { name: "刷新 HTTP 状态", exact: true }).click();
    await page.waitForFunction(() => !document.querySelector('[aria-label="刷新 HTTP 状态"]').disabled);
    assert.equal(await serverId.inputValue(), "portmate-custom-name", "refresh overwrote a manually entered Server ID");
    await dialog.getByRole("button", { name: "复制 CC Switch JSON", exact: true }).click();
    await page.waitForFunction(() => Object.keys(JSON.parse(window.__clipboardText))[0] === "portmate-custom-name");
    assert.equal(mutations(await records()).length, beforeCopy, "copy unexpectedly changed HTTP settings");
    await bounds(".mcp-http-view");
    await page.screenshot({ path: `${screenshotPrefix}-mcp-http.png`, fullPage: true, animations: "disabled" });

    // A poll issued before stop cannot replace its completed result.
    await page.evaluate(() => { window.__deferMcpHttpRuntimeStatus = true; });
    await page.waitForFunction(() => window.__pendingMcpHttpRuntimeStatuses.length >= 1);
    await stop();
    await dialog.locator(".mcp-http-runtime.stopped").waitFor();
    await page.waitForFunction(() => window.__pendingMcpHttpRuntimeStatuses.length >= 2);
    await page.evaluate(() => { const stale = window.__pendingMcpHttpRuntimeStatuses.shift(); stale.resolve(stale.result); });
    assert.equal(await dialog.locator(".mcp-http-runtime.running").count(), 0);
    await page.evaluate(() => {
      window.__deferMcpHttpRuntimeStatus = false;
      for (const pending of window.__pendingMcpHttpRuntimeStatuses.splice(0)) pending.resolve(pending.result);
    });
    const listen = dialog.getByLabel("MCP HTTP 监听 IP", { exact: true });
    await dialog.getByLabel("MCP HTTP 监听范围", { exact: true }).selectOption("0.0.0.0");
    assert(await dialog.getByRole("button", { name: "保存配置", exact: true }).isDisabled());
    const remote = dialog.getByRole("checkbox", { name: "允许非本机监听", exact: true });
    await remote.check();
    await listen.fill("127.0.0.1");
    assert(!await remote.isChecked() && await remote.isDisabled());
    await listen.fill("0.0.0.0");
    assert(!await remote.isChecked());
    await remote.check();
    await dialog.getByLabel("MCP HTTP 客户端地址", { exact: true }).fill("0.0.0.0");
    assert(await dialog.getByRole("button", { name: "保存配置", exact: true }).isDisabled());
    await dialog.getByLabel("MCP HTTP 客户端地址", { exact: true }).fill("192.0.2.42");
    await dialog.locator(".mcp-http-network summary").click();
    await dialog.getByLabel("MCP HTTP 允许的来源", { exact: true }).fill("https://console.example.test");
    await setConfirm(false);
    await dialog.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
    assert(await dialog.isVisible());
    assert((await page.evaluate(() => window.__mcpPrompts.at(-1))).includes("HTTP 配置"));
    await setConfirm(true);
    await page.evaluate(() => { window.__deferMcpHttpMutations = true; });
    await dialog.getByRole("button", { name: "保存配置", exact: true }).click();
    await page.waitForFunction(() => window.__pendingMcpHttpMutations.length === 1);
    assert(await listen.isDisabled());
    await page.evaluate(() => {
      window.__deferMcpHttpMutations = false;
      const pending = window.__pendingMcpHttpMutations.shift(); pending.resolve(pending.result);
    });
    await page.waitForFunction(() => !document.querySelector('[aria-label="MCP HTTP 监听 IP"]').disabled);
    assert.equal(await page.evaluate(() => window.__mcpHttpToken), "portmate-test-token-1", "network save rotated the identity token");

    await createGrant("replacement-client", "Replacement Client");
    await httpClient.selectOption("replacement-client");
    await page.evaluate(() => { window.__failNextMcpHttpSave = true; });
    await dialog.getByRole("button", { name: "保存配置", exact: true }).click();
    await dialog.getByRole("alert").filter({ hasText: "simulated HTTP save failure" }).waitFor();
    assert.equal(await httpClient.inputValue(), "replacement-client", "failed save lost the user's draft");
    assert.equal(await page.evaluate(() => window.__mcpHttpConfig.clientId), "new-client");
    assert(await dialog.getByRole("button", { name: "复制 CC Switch JSON", exact: true }).isDisabled());
    await start();
    await dialog.locator(".mcp-http-runtime.running").waitFor();
    await dialog.getByRole("button", { name: "复制 CC Switch JSON", exact: true }).click();
    await page.waitForFunction(() => window.__clipboardText.includes("Bearer portmate-test-token-2"));

    await page.setViewportSize({ width: 390, height: 844 });
    await bounds(".mcp-dialog");
    await bounds(".mcp-http-view");
    await page.screenshot({ path: `${screenshotPrefix}-mcp-http-mobile.png`, fullPage: true, animations: "disabled" });
    await tab("授权");
    await select("Replacement Client");
    await grantEditorLayout();
    await page.screenshot({ path: `${screenshotPrefix}-mcp-grants-mobile.png`, fullPage: true, animations: "disabled" });
    await dialog.getByLabel("MCP 授权到期时间", { exact: true }).click();
    await dialog.locator(".mcp-expiry-editor").scrollIntoViewIfNeeded();
    await bounds(".mcp-expiry-editor");
    await dialog.getByLabel("MCP 授权到期日期", { exact: true }).press("Escape");
    await revoke();
    await confirm.waitFor();
    await bounds(".mcp-revoke-dialog");
    await page.screenshot({ path: `${screenshotPrefix}-mcp-revoke-mobile.png`, fullPage: true, animations: "disabled" });
    await page.keyboard.press("Escape");
    await confirm.waitFor({ state: "detached" });
    await revoke();
    await confirm.getByLabel("输入 Client ID", { exact: true }).fill("replacement-client");
    await page.evaluate(() => { window.__mcpCleanupWarnings = ["授权已失效，但清除旧 Token 失败：simulated keyring failure"]; });
    await finalRevoke.click();
    await confirm.waitFor({ state: "detached" });
    await dialog.locator(".utility-error", { hasText: "simulated keyring failure" }).waitFor();
    assert.equal(await dialog.locator(".mcp-grant-select", { hasText: "Replacement Client" }).count(), 0, "cleanup warning restored a revoked grant");
    assert.deepEqual(errors, []);
  } finally {
    await page.close();
  }
}
