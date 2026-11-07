import assert from "node:assert/strict";

export async function checkStartupPerformance(context, appUrl, screenshotPrefix) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.addInitScript(() => {
    const template = structuredClone(window.__sessions.find(session => session.profile.id === "edge-router"));
    const ids = Array.from({ length: 10 }, (_, index) => `background-${index}`);
    window.__sessions = ids.map((id, index) => {
      const session = structuredClone(template);
      session.profile.id = id;
      session.profile.name = `Background ${index}`;
      session.profile.connection.identityPolicy.authOrder = ["none"];
      session.runtime = { ...session.runtime, sessionId: id, paneId: `${id}:main`, status: "disconnected", connectedSince: null };
      return session;
    });
    window.__events = [];
    const views = ids.map(id => ({ id: `view-${id}`, sessionId: id, keyMode: "remote", title: "", color: "" }));
    localStorage.setItem("portmate.workspace.v1", JSON.stringify({ version: 4, root: { kind: "pane", id: "startup-pane", views, activeViewId: views[7].id }, activePaneId: "startup-pane", activeId: ids[7], tabColors: {} }));
    localStorage.setItem("portmate.terminalPrefs", JSON.stringify({ startupMode: "last", requireMasterPassword: false, lockOnIdle: false, oneKeyCompletionEnabled: false }));
    window.__deferSessionOpens = true;
    window.__sessionOpenErrors[ids[1]] = "SSH Host Key changed: silent startup test";
    window.__popups = [];
    window.open = (...args) => { window.__popups.push(args); return null; };
    window.__startupLongTasks = [];
    new PerformanceObserver(list => window.__startupLongTasks.push(...list.getEntries().map(entry => entry.duration))).observe({ type: "longtask", buffered: true });
  });
  await page.goto(appUrl);
  await page.waitForFunction(() => window.__pendingSessionOpens.length >= 4);
  const active = page.locator('.workspace-pane-tab.active .workspace-pane-tab-label');
  assert.equal((await active.textContent())?.trim(), "Background 7");
  const initial = await page.evaluate(() => ({
    calls: window.__invokeCalls.filter(call => call.command === "open_session").length,
    pending: window.__pendingSessionOpens.length,
    saves: window.__invokeCalls.filter(call => call.command === "save_session_profile").length,
  }));
  assert.equal(initial.pending, 4, "startup must share a maximum of four in-flight connections");
  assert(initial.calls >= 5, "one failing connection must not block other startups");
  assert.equal(initial.saves, 0, "startup must not re-save already persisted profiles");
  for (let batch = 0; batch < 3; batch++) {
    await page.evaluate(() => {
      for (const pending of window.__pendingSessionOpens.splice(0)) {
        const index = window.__sessions.findIndex(session => session.profile.id === pending.result.profile.id);
        window.__sessions[index] = pending.result;
        pending.resolve(pending.result);
      }
    });
    await page.waitForTimeout(60);
    assert.equal((await active.textContent())?.trim(), "Background 7", "a background completion stole the selected tab");
  }
  await page.waitForFunction(() => window.__invokeCalls.filter(call => call.command === "open_session").length === 10);
  assert.equal(await page.locator(".credential-dialog, .notice-dialog, .host-key-dialog").count(), 0, "startup failures must not open prompts");
  const output = await page.evaluate(() => ({
    opened: window.__invokeCalls.filter(call => call.command === "open_session").map(call => call.args.request.sessionId),
    saves: window.__invokeCalls.filter(call => call.command === "save_session_profile").length,
    popups: window.__popups.length,
    maxLongTask: Math.max(0, ...window.__startupLongTasks),
  }));
  assert.equal(new Set(output.opened).size, 10, "startup duplicated a connection attempt");
  assert.equal(output.saves, 0);
  assert.equal(output.popups, 0);
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ startup: output, activeTabPreserved: true, boundedConcurrency: 4, failuresSilent: true }));
  await page.close();

  const authPage = await context.newPage();
  await authPage.goto(appUrl);
  await authPage.locator(".wind-root").waitFor();
  await authPage.locator(".menu-trigger", { hasText: "会话" }).click();
  await authPage.locator(".menu-popover button", { hasText: "会话设置" }).click();
  const dialog = authPage.locator(".session-settings-dialog");
  await dialog.getByRole("treeitem", { name: "公钥", exact: true }).click();
  const order = dialog.getByRole("group", { name: "SSH 认证顺序", exact: true });
  assert.equal(await order.locator(".ssh-auth-order-row").count(), 5);
  await order.locator('[data-auth-method="password"]').getByRole("button").first().click();
  await order.locator('[data-auth-method="password"]').getByRole("button").first().click();
  assert.equal(await order.locator(".ssh-auth-order-row").first().getAttribute("data-auth-method"), "password");
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await dialog.waitFor({ state: "detached" });
  const saved = await authPage.evaluate(() => window.__invokeCalls.findLast(call => call.command === "save_session_profile")?.args.profile.connection.identityPolicy.authOrder);
  assert.equal(saved[0], "password");
  await authPage.locator(".menu-trigger", { hasText: "会话" }).click();
  await authPage.locator(".menu-popover button", { hasText: "会话设置" }).click();
  await dialog.getByRole("treeitem", { name: "公钥", exact: true }).click();
  assert.equal(await order.locator(".ssh-auth-order-row").first().getAttribute("data-auth-method"), "password");
  // A single method is supported without a second, hidden ordering policy.
  await order.locator('[data-auth-method="public-key"]').getByRole("checkbox").uncheck();
  await order.locator('[data-auth-method="keyboard-interactive"]').getByRole("checkbox").uncheck();
  assert.equal(await order.locator('[data-auth-method="password"]').getByRole("checkbox").isDisabled(), true);
  const identitySelector = dialog.getByRole("group", { name: "客户端身份选择", exact: true });
  await identitySelector.getByRole("button", { name: "添加", exact: true }).click();
  await dialog.getByLabel("名称:(N)", { exact: true }).fill("Second key");
  await dialog.locator(".dialog-field", { hasText: "私钥文件:(F)" }).locator("input").fill("/home/operator/.ssh/second");
  const identities = identitySelector.getByRole("combobox");
  assert.match(await identities.locator("option").last().textContent(), /^2\. Second key/);
  await identitySelector.getByRole("button", { name: "上移当前密钥", exact: true }).click();
  assert.match(await identities.locator("option").first().textContent(), /^1\. Second key/);
  await identitySelector.getByRole("button", { name: "下移当前密钥", exact: true }).click();
  assert.match(await identities.locator("option").last().textContent(), /^2\. Second key/);
  await identitySelector.getByRole("button", { name: "上移当前密钥", exact: true }).click();
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await dialog.waitFor({ state: "detached" });
  const savedConnection = await authPage.evaluate(() => window.__invokeCalls.findLast(call => call.command === "save_session_profile")?.args.profile.connection);
  assert.deepEqual(savedConnection.identityPolicy.authOrder, ["password"]);
  assert.deepEqual(savedConnection.identityRefs.map(identity => identity.label), ["Second key", "Initial identity"]);
  await authPage.locator(".menu-trigger", { hasText: "会话" }).click();
  await authPage.locator(".menu-popover button", { hasText: "会话设置" }).click();
  await dialog.getByRole("treeitem", { name: "公钥", exact: true }).click();
  assert.equal(await order.getByRole("checkbox", { checked: true }).count(), 1);
  assert.match(await identities.locator("option").first().textContent(), /^1\. Second key/);
  if (screenshotPrefix) {
    await dialog.screenshot({ path: `${screenshotPrefix}-ssh-auth-order.png` });
    await authPage.setViewportSize({ width: 480, height: 820 });
    await order.scrollIntoViewIfNeeded();
    await authPage.screenshot({ path: `${screenshotPrefix}-ssh-auth-order-narrow.png`, fullPage: true });
    const overflow = await order.evaluate(element => element.scrollWidth - element.clientWidth);
    assert(overflow <= 1, `SSH authentication editor overflowed by ${overflow}px`);
    const bounds = await order.boundingBox();
    assert(bounds && bounds.x >= 0 && bounds.x + bounds.width <= 480, "SSH order controls were clipped by the dialog");
    const arrowBounds = await order.locator(".ssh-auth-order-row").first().getByRole("button").last().boundingBox();
    assert(arrowBounds && arrowBounds.x + arrowBounds.width <= 480, "order arrows were offscreen");
  }
  console.log("SSH authentication and private-key order persisted; single-method policy reopened unchanged");
  await authPage.close();
}
