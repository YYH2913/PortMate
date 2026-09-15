import assert from "node:assert/strict";

export async function checkModuleAuditRegressions(context, appUrl) {
  async function withPage(test, url = appUrl) {
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.route("**/src/TerminalCanvas.tsx", async route => {
      const response = await route.fetch();
      await route.fulfill({ response, body: (await response.text()).replace(
        "termRef.current = term;", "termRef.current = term; window.__auditTerminal = term;",
      ) });
    });
    try {
      await page.goto(url);
      await page.locator(".terminal-host[data-terminal-ready=true]").first().waitFor().catch(async cause => {
        throw new Error(`audit page failed to initialize: ${url}; errors=${JSON.stringify(errors)}; body=${(await page.locator("body").innerText()).slice(0,1800)}; hosts=${JSON.stringify(await page.locator(".terminal-host").evaluateAll(hosts => hosts.map(host => ({ ...host.dataset }))))}`, { cause });
      });
      await page.evaluate(() => { window.confirm = () => true; });
      await test(page);
      assert.deepEqual(errors, [], "module audit produced browser exceptions");
    } finally { await page.close(); }
  }
  const openTool = async (page, name) => {
    await page.getByRole("button", { name: "工具", exact: true }).click();
    await page.getByRole("button", { name, exact: true }).click();
  };

  await withPage(async page => {
    await page.getByRole("button", { name: "工作区", exact: true }).click();
    await page.locator(".menu-popover").getByRole("button", { name: "文件管理器", exact: true }).click();
    const local = page.locator('[data-file-pane="local"]');
    const remote = page.locator('[data-file-pane="remote"]');
    await local.locator('[aria-label="新建文件"]:not(:disabled)').waitFor();
    const calls = command => page.evaluate(command => window.__invokeCalls.filter(call => call.command === command), command);
    const entry = (path, name) => ({ path: `${path}/${name}`, name, isDir: false, isSymlink: false, size: 32 });
    async function load(pane, path, entries) {
      await page.evaluate(() => { window.__deferFileLoads = true; });
      const input = pane.getByRole("textbox");
      await input.fill(path);
      await input.press("Enter");
      await page.waitForFunction(path => window.__pendingFileLoads.some(item => item.args.request.path === path), path);
      await page.evaluate(({ path, entries }) => {
        const index = window.__pendingFileLoads.findIndex(item => item.args.request.path === path);
        window.__pendingFileLoads.splice(index, 1)[0].resolve(entries);
        window.__deferFileLoads = false;
      }, { path, entries });
      await pane.locator('.file-actions button[aria-label="新建文件"]:not(:disabled)').waitFor();
    }
    await load(local, "/audit-loaded", [entry("/audit-loaded", "file.txt")]);
    await local.getByRole("textbox").fill("/unconfirmed-draft");
    await page.evaluate(() => { window.prompt = () => "new.txt"; });
    await local.getByRole("button", { name: "新建文件", exact: true }).click();
    assert.equal((await calls("create_file")).at(-1).args.request.path, "/audit-loaded/new.txt", "creation used an unsubmitted directory draft");
    await local.locator('[aria-label="新建文件"]:not(:disabled)').waitFor();
    assert.equal(await local.getByRole("textbox").inputValue(), "/unconfirmed-draft", "mutation refresh discarded the address draft");
    await load(local, "/audit-selected", [entry("/audit-selected", "file.txt")]);
    await local.getByRole("option", { name: /file.txt/ }).click();
    await local.locator("summary").click();
    await page.evaluate(() => { window.prompt = () => "648junk"; });
    const chmodBefore = (await calls("chmod_path")).length;
    await local.getByRole("button", { name: "修改权限", exact: true }).click();
    await local.getByRole("alert").filter({ hasText: "八进制" }).waitFor();
    assert.equal((await calls("chmod_path")).length, chmodBefore, "partially valid permissions were sent");
    await page.evaluate(() => { window.__deferFileLoads = true; });
    await local.getByRole("textbox").fill("/missing-directory");
    await local.getByRole("textbox").press("Enter");
    await page.waitForFunction(() => window.__pendingFileLoads.some(item => item.args.request.path === "/missing-directory"));
    await page.evaluate(() => {
      const index = window.__pendingFileLoads.findIndex(item => item.args.request.path === "/missing-directory");
      window.__pendingFileLoads.splice(index, 1)[0].reject(new Error("missing directory"));
      window.__deferFileLoads = false;
    });
    await local.getByRole("alert").filter({ hasText: "missing directory" }).waitFor();
    assert(await local.getByRole("button", { name: "删除", exact: true }).isDisabled(), "failed navigation left invisible files selected");
    assert(await local.getByRole("button", { name: "新建文件", exact: true }).isDisabled(), "failed directory remained a mutation target");

    await page.evaluate(() => { window.__deferFileLoads = true; });
    await remote.getByRole("textbox").fill("/old-connection");
    await remote.getByRole("textbox").press("Enter");
    await page.waitForFunction(() => window.__pendingFileLoads.some(item => item.args.request.path === "/old-connection"));
    await page.evaluate(() => {
      const session = window.__sessions.find(item => item.profile.id === "edge-router");
      session.runtime.connectedSince = new Date(Date.now() + 1000).toISOString();
      window.__emitTauriEvent("portmate-session-profile-updated", structuredClone(session));
    });
    await page.waitForFunction(() => window.__pendingFileLoads.some(item => item.args.request.remote && item.args.request.path === "."));
    await page.evaluate(entries => {
      const index = window.__pendingFileLoads.findIndex(item => item.args.request.remote && item.args.request.path === ".");
      window.__pendingFileLoads.splice(index, 1)[0].resolve(entries);
    }, [entry(".", "NEW-REMOTE")]);
    await remote.getByRole("option", { name: /NEW-REMOTE/ }).waitFor();
    await page.evaluate(entries => {
      const index = window.__pendingFileLoads.findIndex(item => item.args.request.path === "/old-connection");
      window.__pendingFileLoads.splice(index, 1)[0].resolve(entries);
      window.__deferFileLoads = false;
    }, [entry("/old-connection", "STALE-REMOTE")]);
    assert.equal(await remote.getByRole("option", { name: /STALE-REMOTE/ }).count(), 0, "old connection overwrote the new directory");
    await load(local, "/audit-upload", [entry("/audit-upload", "upload.txt")]);
    await remote.getByRole("textbox").fill("/unconfirmed-upload-destination");
    await local.getByRole("option", { name: /upload.txt/ }).click();
    await local.getByRole("button", { name: "上传", exact: true }).click();
    await page.waitForFunction(() => window.__invokeCalls.some(call => call.command === "start_file_batch"));
    assert.equal((await calls("start_file_batch")).at(-1).args.request.destination, ".", "upload used an unsubmitted target directory");
  });

  await withPage(async page => {
    const task = { id: "audit-queued", sessionId: "edge-router", protocol: "sftp", source: "/audit.bin", destination: "/srv/audit.bin", bytesTotal: 32, bytesDone: 0, status: "queued", message: "queued", startedAt: null, finishedAt: null, averageBytesPerSecond: null };
    await page.evaluate(task => { window.__transfers.push(task); window.__emitTauriEvent("portmate-transfer-task", task); }, task);
    await openTool(page, "传输任务");
    const row = page.locator(".transfer-dialog .transfer-row", { hasText: "/audit.bin" });
    await row.getByRole("button", { name: "取消", exact: true }).click();
    await page.waitForFunction(() => window.__transfers.find(task => task.id === "audit-queued")?.status === "cancelled");
    await page.locator(".notice-dialog").getByRole("button", { name: "确定", exact: true }).click();
    await page.evaluate(task => {
      for (const status of ["queued", "running"]) {
        const next = { ...task, id: `audit-batch-${status}`, status };
        window.__transfers.push(next);
        window.__emitTauriEvent("portmate-transfer-task", next);
      }
    }, task);
    await page.getByRole("button", { name: "取消未完成", exact: true }).click();
    await page.waitForFunction(() => ["queued", "running"].every(status =>
      window.__transfers.find(task => task.id === `audit-batch-${status}`)?.status === "cancelled"));
  });

  async function editorFailure(page) {
    await page.evaluate(async () => {
      const { requestTerminalFreeInput } = await import("/src/terminal-free-input.ts");
      requestTerminalFreeInput(window, "audit_command_unique");
      window.__deferTerminalSends = true;
    });
    const editor = page.getByRole("form", { name: "自由输入编辑器" });
    const input = editor.getByRole("textbox", { name: "自由输入内容" });
    await editor.getByRole("button", { name: "发送自由输入" }).click();
    await page.waitForFunction(() => window.__pendingTerminalSends.length === 1);
    assert(await input.isDisabled(), "in-flight editor remained editable");
    const terminalInput = editor.locator("..").locator(".xterm-helper-textarea");
    await terminalInput.press("x");
    await terminalInput.press("Shift+Escape");
    assert(await editor.isVisible(), "background terminal hotkeys discarded an in-flight editor");
    await terminalInput.evaluate(element => {
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", "audit_unexpected_paste\r");
      element.dispatchEvent(new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }));
    });
    assert.equal(await page.evaluate(() => window.__invokeCalls.filter(call => call.command === "record_command_history" && call.args.command?.includes("audit_unexpected_paste")).length), 0,
      "background paste mutated command tracking during an acknowledged editor send");
    await editor.dispatchEvent("submit");
    assert.equal(await page.evaluate(() => window.__pendingTerminalSends.length), 1, "editor submitted twice");
    const historyCalls = () => page.evaluate(() => window.__invokeCalls.filter(call => call.command === "record_command_history" && call.args.command === "audit_command_unique").length);
    assert.equal(await historyCalls(), 0, "history was recorded before acknowledgement");
    await page.evaluate(() => window.__pendingTerminalSends.shift().reject(new Error("simulated transport failure")));
    await editor.getByRole("alert").filter({ hasText: "simulated transport failure" }).waitFor();
    assert.equal(await input.inputValue(), "audit_command_unique", "failed send lost the editor draft");
    assert(await input.isVisible() && (await input.boundingBox()).height >= 40, "failure message hid the editor input");
    const windowKind = new URL(page.url()).searchParams.has("detachedPane") ? "detached" : "main";
    await editor.screenshot({ path: `${process.env.PORTMATE_WORKSPACE_UI_SCREENSHOT_PREFIX ?? "/tmp/portmate-workspace-ui"}-audit-editor-${windowKind}.png` });
    assert.equal(await historyCalls(), 0, "failed send was recorded as a command");
    await editor.getByRole("button", { name: "发送自由输入" }).click();
    await page.waitForFunction(() => window.__pendingTerminalSends.length === 1);
    await page.evaluate(() => new Promise(resolve => window.__auditTerminal.write("\x1b[5n", resolve)));
    await page.evaluate(() => { window.__pendingTerminalSends.shift().resolve(null); window.__deferTerminalSends = false; });
    await editor.waitFor({ state: "detached" });
    await page.waitForFunction(() => window.__invokeCalls.some(call => call.command === "send_text" && call.args.text === "\x1b[0n"));
    await page.waitForFunction(() => window.__invokeCalls.some(call => call.command === "record_command_history" && call.args.command === "audit_command_unique"));
    assert.equal(await historyCalls(), 1, "successful retry did not commit history exactly once");

    await page.evaluate(async () => {
      const { requestTerminalFreeInput } = await import("/src/terminal-free-input.ts");
      requestTerminalFreeInput(window, "audit_replaced_connection");
      window.__deferTerminalSends = true;
    });
    await editor.getByRole("button", { name: "发送自由输入" }).click();
    await page.waitForFunction(() => window.__pendingTerminalSends.length === 1);
    await page.evaluate(() => {
      const sessionId = window.__pendingTerminalSends[0].args.sessionId;
      const session = window.__sessions.find(item => item.profile.id === sessionId);
      session.runtime.connectedSince = new Date(Date.now() + 2000).toISOString();
      window.__emitTauriEvent("portmate-session-profile-updated", structuredClone(session));
    });
    await page.waitForFunction(() => document.querySelector('[aria-label="发送自由输入"]')?.disabled === false);
    await page.evaluate(async () => {
      window.__pendingTerminalSends.shift().resolve(null);
      window.__deferTerminalSends = false;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    });
    assert.equal(await input.inputValue(), "audit_replaced_connection", "old connection acknowledgement cleared the current draft");
    assert.equal(await page.evaluate(() => window.__invokeCalls.filter(call => call.command === "record_command_history" && call.args.command === "audit_replaced_connection").length), 0,
      "old connection acknowledgement committed history on the replacement");
    await editor.getByRole("button", { name: "取消自由输入" }).click();
  }
  await withPage(editorFailure);
  const detached = new URL(appUrl);
  for (const [key, value] of Object.entries({ detachedPane: "1", windowId: "audit-window", ownerWindowId: "main", paneId: "audit-pane", viewId: "audit-view", sessionId: "local-shell", keyMode: "remote" })) detached.searchParams.set(key, value);
  await withPage(async page => {
    await editorFailure(page);
    const host = page.locator(".terminal-host");
    const instance = await host.getAttribute("data-terminal-instance-id");
    await page.evaluate(async () => {
      // A peer window's cache is a hint, not authoritative native session state.
      localStorage.setItem("portmate.sessions", "invalid-cache");
      window.dispatchEvent(new StorageEvent("storage", { key: "portmate.sessions", newValue: "invalid-cache" }));
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    });
    assert.equal(await host.getAttribute("data-terminal-ready"), "true", "peer cache invalidated a live native terminal");
    assert.equal(await host.getAttribute("data-terminal-instance-id"), instance, "peer cache recreated a live native terminal");
  }, detached.href);

  await withPage(async page => {
    await page.evaluate(() => window.__customScripts.push({ ...structuredClone(window.__customScripts[0]), id: "a7b0c320-d43f-4b56-9a4e-3f67d659cb12", name: "Second script" }));
    await openTool(page, "自定义脚本");
    const dialog = page.locator(".custom-script-dialog");
    await dialog.getByLabel("脚本运行参数").fill('{"old":"value"}');
    await dialog.getByRole("button", { name: "运行自定义脚本", exact: true }).click();
    await page.locator(".notice-dialog").getByRole("button", { name: "确定", exact: true }).click();
    await dialog.getByLabel("脚本运行结果").waitFor();
    await dialog.getByRole("button", { name: "删除自定义脚本", exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[aria-label="脚本名称"]')?.value === "Second script");
    assert.equal(await dialog.getByLabel("脚本运行结果").count(), 0, "another script inherited the deleted script's result");
    assert.equal(await dialog.getByLabel("脚本运行参数").inputValue(), "{}");
    await dialog.getByLabel("脚本运行参数").fill('{"second":true}');
    await dialog.getByRole("button", { name: "刷新自定义脚本", exact: true }).click();
    await dialog.getByRole("button", { name: "运行自定义脚本", exact: true }).waitFor();
    assert.equal(await dialog.getByLabel("脚本运行参数").inputValue(), '{"second":true}', "same-script refresh discarded runtime parameters");
    await page.evaluate(() => {
      window.__customScripts = [{ ...window.__customScripts[0], id: "78b62a79-b4fd-4b78-b616-d43d66839901", name: "Replacement script" }];
    });
    await dialog.getByRole("button", { name: "刷新自定义脚本", exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[aria-label="脚本名称"]')?.value === "Replacement script");
    assert.equal(await dialog.getByLabel("脚本运行参数").inputValue(), "{}", "refresh leaked deleted script parameters into its replacement");
  });

  await withPage(async page => {
    await openTool(page, "MCP Bridge");
    const dialog = page.locator(".mcp-dialog");
    await dialog.getByRole("tab", { name: "HTTP", exact: true }).click();
    const refresh = dialog.getByRole("button", { name: "刷新 HTTP 状态" });
    const copy = dialog.getByRole("button", { name: "复制 CC Switch JSON" });
    for (const action of ["copy", "rotate", "start", "save"]) {
      await page.evaluate(() => { window.__mcpHttpConfig = window.__buildMcpHttpConfig({ ...window.__mcpHttpConfig, clientId: "ops-console" }); window.__mcpHttpToken = "old-window-token"; });
      await refresh.click();
      await page.waitForFunction(() => document.querySelector('[aria-label="MCP HTTP Client ID"]')?.value === "ops-console");
      if (action === "save") await dialog.getByLabel("MCP HTTP 端口").fill("9876");
      const before = await page.evaluate(() => {
        window.__mcpHttpConfig = window.__buildMcpHttpConfig({ ...window.__mcpHttpConfig, clientId: "audit-reader" });
        window.__mcpHttpToken = "other-window-token";
        return { clipboard: window.__clipboardText, rotations: window.__mcpHttpTokenSequence };
      });
      if (action === "copy") await copy.click();
      else if (action === "rotate") await dialog.getByRole("button", { name: "轮换 Token", exact: true }).click();
      else if (action === "start") await dialog.getByRole("button", { name: "启动服务", exact: true }).click();
      else await dialog.getByRole("button", { name: "保存配置", exact: true }).click();
      await dialog.getByRole("alert").filter({ hasText: "其他窗口" }).waitFor();
      assert.deepEqual(await page.evaluate(() => ({ clipboard: window.__clipboardText, rotations: window.__mcpHttpTokenSequence })), before, `${action} changed another client's access`);
      assert.equal(await page.evaluate(() => window.__mcpHttpRuntime.phase), "stopped");
      assert.equal(await page.evaluate(() => window.__mcpHttpConfig.clientId), "audit-reader");
      if (action === "save") await dialog.getByRole("button", { name: "放弃更改", exact: true }).click();
    }
    await page.evaluate(() => { window.__mcpHttpToken = null; });
    await refresh.click();
    await dialog.getByRole("button", { name: "生成 Token", exact: true }).waitFor();
    await dialog.locator("summary", { hasText: "进程信息与手动启动" }).click();
    const command = await dialog.getByLabel("MCP HTTP 启动命令").inputValue();
    await dialog.getByRole("button", { name: "复制命令", exact: true }).click();
    await page.waitForFunction(command => window.__clipboardText === command, command);
    assert.equal(await page.evaluate(() => window.__mcpHttpToken), null, "copying a startup command generated a token");
  });
}
