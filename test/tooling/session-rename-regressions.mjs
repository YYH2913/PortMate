import assert from "node:assert/strict";

export async function checkSessionRename(context, appUrl) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  const tab = id => page.locator(`.workspace-pane-tab[data-view-id="${id}"]`);
  const label = id => tab(id).locator(".workspace-pane-tab-label");
  const rename = page.locator(".workspace-view-rename-dialog");
  const tree = name => page.locator(".tree-session", { hasText: name });
  async function synchronized(name, ids = ["rename-original", "rename-mirror", "rename-closed"]) {
    await tree(name).waitFor();
    for (const id of ids) {
      await page.waitForFunction(({ id, name }) => document.querySelector(`.workspace-pane-tab[data-view-id="${id}"] .workspace-pane-tab-label`)?.textContent.trim() === name, { id, name });
    }
    assert.equal(await page.evaluate(() => window.__sessions.find(session => session.profile.id === "edge-router").profile.name), name);
    assert.equal(await label("rename-unrelated").textContent(), "Unrelated alias");
  }
  try {
    await page.goto(appUrl);
    await tree("Edge Router").waitFor();
    await page.evaluate(() => {
      localStorage.setItem("portmate.workspace.v1", JSON.stringify({
        version: 4, activePaneId: "rename-left", activeId: "edge-router",
        root: {
          kind: "split", id: "rename-split", direction: "vertical", ratio: 0.5,
          first: { kind: "pane", id: "rename-left", activeViewId: "rename-original", views: [
            { id: "rename-original", sessionId: "edge-router", title: "Old alias", color: "#4169E1", keyMode: "remote" },
            { id: "rename-unrelated", sessionId: "local-shell", title: "Unrelated alias", color: "", keyMode: "remote" },
          ] },
          second: { kind: "pane", id: "rename-right", activeViewId: "rename-mirror", views: [
            { id: "rename-mirror", sessionId: "edge-router", title: "Mirror alias", color: "", keyMode: "remote" },
            { id: "rename-closed", sessionId: "edge-router", title: "Closed alias", color: "", keyMode: "remote" },
          ] },
        }, tabColors: {},
      }));
    });
    await page.reload();
    await label("rename-original").waitFor();
    await page.waitForFunction(() => document.querySelectorAll('.terminal-host[data-terminal-ready="true"]').length === 2);
    const before = await page.evaluate(() => ({
      connectedSince: window.__sessions.find(session => session.profile.id === "edge-router").runtime.connectedSince,
      terminals: [...document.querySelectorAll(".terminal-host")].map(host => host.dataset.terminalInstanceId),
    }));

    // A failed save must leave both the Profile and all existing aliases intact.
    await label("rename-original").dblclick();
    await rename.waitFor();
    assert.equal(await rename.getByLabel("会话名称", { exact: true }).inputValue(), "Edge Router");
    await rename.getByLabel("会话名称", { exact: true }).fill("Failed rename");
    await page.evaluate(() => { window.__failNextProfileSave = true; });
    await rename.getByRole("button", { name: "保存", exact: true }).click();
    const notice = page.locator(".notice-dialog");
    await notice.waitFor();
    assert.equal(await label("rename-original").textContent(), "Old alias");
    assert.equal(await page.evaluate(() => window.__sessions.find(session => session.profile.id === "edge-router").profile.name), "Edge Router");
    await notice.getByRole("button", { name: "确定", exact: true }).click();
    await rename.getByRole("button", { name: "取消", exact: true }).click();

    // Explorer rename must update every open view and closed-view history.
    await tab("rename-closed").locator(".workspace-pane-tab-close").click();
    await tab("rename-closed").waitFor({ state: "detached" });
    await page.evaluate(() => { window.prompt = () => "Renamed from Explorer"; });
    await tree("Edge Router").click({ button: "right" });
    await page.locator(".portmate-context-menu").getByRole("button", { name: "重命名会话(R)", exact: true }).click();
    await synchronized("Renamed from Explorer", ["rename-original", "rename-mirror"]);
    const after = await page.evaluate(() => ({
      connectedSince: window.__sessions.find(session => session.profile.id === "edge-router").runtime.connectedSince,
      terminals: [...document.querySelectorAll(".terminal-host")].map(host => host.dataset.terminalInstanceId),
    }));
    assert.deepEqual(after, before, "metadata rename must preserve live terminal instances and connection");
    await tab("rename-original").click({ button: "right" });
    await page.locator(".workspace-view-context-menu").getByRole("button", { name: "重新打开已关闭视图", exact: true }).click();
    await synchronized("Renamed from Explorer");

    // Tab rename must persist the Profile, fence duplicate submissions, and
    // update siblings rather than creating another local title override.
    await label("rename-original").dispatchEvent("auxclick", { button: 1 });
    await rename.getByLabel("会话名称", { exact: true }).fill("Renamed from Tab");
    await page.evaluate(() => { window.__deferSessionProfileSaves = true; });
    await rename.getByRole("button", { name: "保存", exact: true }).click();
    await page.waitForFunction(() => window.__pendingSessionProfileSaves.length === 1);
    assert.equal(await rename.getByRole("button", { name: "保存", exact: true }).isDisabled(), true);
    assert.equal(await rename.getByRole("button", { name: "取消", exact: true }).isDisabled(), true);
    const request = await page.evaluate(() => window.__pendingSessionProfileSaves[0].args);
    assert.equal(request.expectedProfile.name, "Renamed from Explorer");
    assert.equal(request.profile.name, "Renamed from Tab");
    await page.evaluate(() => {
      window.__deferSessionProfileSaves = false;
      window.__pendingSessionProfileSaves.splice(0).forEach(pending => pending.resolve());
    });
    await rename.waitFor({ state: "detached" });
    await synchronized("Renamed from Tab");

    // Session Settings and external updates follow the same state path.
    await label("rename-original").click();
    await page.getByRole("button", { name: "会话", exact: true }).click();
    await page.locator(".menu-popover").getByRole("button", { name: "会话设置", exact: true }).click();
    const settings = page.locator(".session-settings-dialog");
    await settings.getByLabel("名称:(N)", { exact: true }).fill("Renamed in Settings");
    await settings.getByRole("button", { name: "保存", exact: true }).click();
    await settings.waitFor({ state: "detached" });
    await synchronized("Renamed in Settings");
    await page.evaluate(() => {
      const session = window.__sessions.find(session => session.profile.id === "edge-router");
      session.profile.name = "Renamed externally";
      window.__emitTauriEvent("portmate-session-profile-updated", structuredClone(session));
    });
    await synchronized("Renamed externally");
    await page.waitForFunction(() => {
      const workspace = JSON.parse(localStorage.getItem("portmate.workspace.v1"));
      const views = node => node.kind === "pane" ? node.views : [...views(node.first), ...views(node.second)];
      return views(workspace.root).filter(view => view.sessionId === "edge-router").every(view => view.title === "");
    });
    // This browser backend resets its Profiles on reload. Persisted layout must
    // resolve the refreshed Profile name, not resurrect any old tab alias.
    await page.reload();
    await synchronized("Edge Router");
    assert.deepEqual(errors, []);
  } finally { await page.close(); }

  const detached = await context.newPage();
  const detachedErrors = [];
  detached.on("pageerror", error => detachedErrors.push(error.message));
  try {
    const url = new URL(appUrl);
    for (const [key, value] of Object.entries({ detachedPane: "1", windowId: "rename-detached", ownerWindowId: "main", paneId: "rename-left", viewId: "rename-original", sessionId: "edge-router", title: "Old detached alias", color: "", keyMode: "remote" })) url.searchParams.set(key, value);
    await detached.goto(url.toString());
    const heading = detached.locator(".detached-pane-toolbar strong");
    await heading.waitFor();
    assert.equal(await heading.textContent(), "Old detached alias");
    await detached.waitForFunction(() => window.__tauriEventListeners.get("portmate-session-profile-updated")?.length > 0);
    await detached.locator('.terminal-host[data-terminal-ready="true"]').waitFor();
    const instance = await detached.locator(".terminal-host").getAttribute("data-terminal-instance-id");
    await detached.evaluate(() => {
      const session = window.__sessions.find(session => session.profile.id === "edge-router");
      session.profile.name = "Renamed while detached";
      window.__emitTauriEvent("portmate-session-profile-updated", structuredClone(session));
      window.__detachedReattachResult = { ok: true, error: "" };
    });
    await heading.getByText("Renamed while detached", { exact: true }).waitFor();
    assert.equal(await detached.title(), "Renamed while detached - PortMate");
    assert.equal(new URL(detached.url()).searchParams.get("title"), "", "reload must not resurrect the detached alias");
    assert.equal(await detached.locator(".terminal-host").getAttribute("data-terminal-instance-id"), instance);
    await detached.getByRole("button", { name: "返回工作区", exact: true }).click();
    const command = await detached.evaluate(() => window.__invokeCalls.find(call => call.command === "plugin:event|emit_to"
      && call.args.event === "portmate-detached-pane-command" && call.args.payload.action === "reattach")?.args.payload);
    assert.equal(command?.title, "", "reattach must not restore a stale title alias");
    assert.deepEqual(detachedErrors, []);
  } finally { await detached.close(); }
}
