import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const messages = Object.fromEntries(["zh", "en", "fr", "es", "ru", "ar"].map(locale => [
  locale, JSON.parse(readFileSync(new URL(`../../src/locales/${locale}.json`, import.meta.url), "utf8")),
]));
const key = "portmate.sessionExplorerGrouping.v1";

export async function checkSessionExplorerGrouping(context, appUrl, screenshotPrefix) {
  const page = await context.newPage();
  const peer = await context.newPage();
  const errors = [];
  for (const current of [page, peer]) current.on("pageerror", error => errors.push(error.message));
  const explorer = page.locator(".workspace-dock-content.panel-explorer");
  const grouping = explorer.locator(".explorer-grouping select");
  const groups = () => explorer.locator(".tree-folder > span").allTextContents();
  try {
    await page.goto(appUrl);
    await grouping.waitFor();
    await peer.goto(appUrl);
    await peer.locator(".explorer-grouping select").waitFor();
    assert.equal(await grouping.inputValue(), "profile", "existing custom grouping must remain the default");
    assert.deepEqual(await groups(), ["Network", "Lab", "Local"]);
    await page.waitForFunction(() => window.__tauriEventListeners.get("portmate-session-profile-updated")?.length);
    await page.evaluate(async () => {
      const { createTcpConnection } = await import("/src/session-profile-helpers.ts");
      const source = window.__sessions.find(item => item.profile.id === "edge-router");
      for (const kind of ["ssh", "tmux", "tcp", "telnet"]) {
        const session = structuredClone(source);
        session.profile.id = `grouping-${kind}`;
        session.profile.name = `Grouping ${kind}`;
        session.profile.group = kind === "ssh" ? "Lab" : "Mixed";
        session.profile.kind = kind;
        session.profile.connection = ["tcp", "telnet"].includes(kind)
          ? { ...createTcpConnection(kind), host: "localhost", reconnect: false }
          : { ...source.profile.connection, kind };
        session.runtime.sessionId = session.profile.id;
        session.runtime.status = "disconnected";
        session.runtime.connectedSince = null;
        window.__sessions.push(session);
        window.__emitTauriEvent("portmate-session-profile-updated", structuredClone(session));
      }
    });
    await explorer.getByRole("button", { name: "Grouping telnet", exact: true }).waitFor();
    const profilesBefore = await page.evaluate(() => JSON.stringify(window.__sessions.map(session => session.profile)));
    const terminalBefore = await page.locator('.terminal-host[data-terminal-ready="true"]').evaluateAll(elements => elements.map(element => element.dataset.terminalInstanceId));
    await grouping.selectOption("type");
    await peer.waitForFunction(() => document.querySelector(".explorer-grouping select")?.value === "type");
    assert.deepEqual(await groups(), ["SSH", messages.zh["explorer-serial-type"], "Shell", "Telnet", "TCP", "Tmux"]);
    assert.equal(await explorer.locator('[data-session-group="type:ssh"] .tree-session').count(), 2);
    assert.equal(await explorer.locator('[data-session-group="type:serial"] .tree-session').count(), 1);
    assert.equal(await explorer.locator(".tree-session").count(), 7, "grouping lost or duplicated a session");
    assert.equal(await page.evaluate(key => localStorage.getItem(key), key), "type");
    assert.equal(await page.evaluate(() => window.__invokeCalls.filter(call => call.command === "save_session_profile").length), 0,
      "view grouping must not modify saved profiles");
    assert.equal(await page.evaluate(() => JSON.stringify(window.__sessions.map(session => session.profile))), profilesBefore);
    assert.deepEqual(await page.locator('.terminal-host[data-terminal-ready="true"]').evaluateAll(elements => elements.map(element => element.dataset.terminalInstanceId)), terminalBefore,
      "changing the explorer should not rebuild live terminals");

    const filter = explorer.getByRole("textbox");
    await filter.fill("hardware");
    await explorer.locator('[data-session-group="type:serial"]').waitFor();
    assert.deepEqual(await groups(), [messages.zh["explorer-serial-type"]]);
    await filter.fill("no-matching-session-fixture");
    assert.equal(await explorer.locator(".tree-folder").count(), 0);
    assert((await explorer.textContent()).includes(messages.zh["no-matching-sessions"]));
    await filter.fill("");
    await grouping.selectOption("profile");
    assert.deepEqual(await groups(), ["Network", "Lab", "Local", "Mixed"]);
    await grouping.selectOption("type");

    // Authoritative updates must move a session immediately without losing its row.
    await page.evaluate(() => {
      const session = window.__sessions.find(item => item.profile.id === "grouping-telnet");
      session.profile.kind = "tcp";
      session.profile.connection.kind = "tcp";
      window.__emitTauriEvent("portmate-session-profile-updated", structuredClone(session));
    });
    await page.waitForFunction(() => document.querySelectorAll('[data-session-group="type:tcp"] .tree-session').length === 2);
    assert.equal(await explorer.locator('[data-session-group="type:telnet"]').count(), 0);
    await explorer.getByRole("button", { name: "Bench UART", exact: true }).click();
    assert(await explorer.getByRole("button", { name: "Bench UART", exact: true }).evaluate(element => element.classList.contains("active")));
    await explorer.getByRole("button", { name: "Bench UART", exact: true }).click({ button: "right" });
    const contextMenu = page.locator('.portmate-context-menu[data-interaction-layer="active"]');
    await contextMenu.waitFor();
    await contextMenu.focus();
    await contextMenu.press("Escape");
    await page.locator(".portmate-context-menu").waitFor({ state: "detached" });

    // The preference survives hiding/remounting and reloading.
    await page.locator('.menu-trigger[aria-controls="menu-workspace"]').click();
    await page.locator("#menu-workspace").getByRole("button", { name: messages.zh.explorer, exact: true }).click();
    await explorer.waitFor({ state: "hidden" });
    await page.locator('.menu-trigger[aria-controls="menu-workspace"]').click();
    await page.locator("#menu-workspace").getByRole("button", { name: messages.zh.explorer, exact: true }).click();
    await grouping.waitFor();
    assert.equal(await grouping.inputValue(), "type");
    await page.reload();
    await grouping.waitFor();
    assert.equal(await grouping.inputValue(), "type");
    assert.deepEqual(await groups(), ["SSH", messages.zh["explorer-serial-type"], "Shell"]);

    // Invalid shared preferences fail back to the existing grouping mode.
    await peer.evaluate(key => localStorage.setItem(key, "invalid-mode"), key);
    await page.waitForFunction(() => document.querySelector(".explorer-grouping select")?.value === "profile");
    await grouping.selectOption("type");
    await peer.evaluate(key => localStorage.removeItem(key), key);
    await page.waitForFunction(() => document.querySelector(".explorer-grouping select")?.value === "profile");
    await grouping.selectOption("type");

    for (const locale of ["zh", "en", "fr", "es", "ru", "ar"]) {
      await page.evaluate(async locale => (await import("/src/i18n.ts")).setLanguagePreference(locale), locale);
      await explorer.getByRole("combobox", { name: messages[locale]["explorer-grouping"], exact: true }).waitFor();
      assert.deepEqual(await groups(), ["SSH", messages[locale]["explorer-serial-type"], "Shell"]);
      assert.equal(await grouping.locator('option[value="type"]').textContent(), messages[locale]["explorer-grouping-type"]);
      // Check the minimum 200px side dock and RTL, not just the wide canvas.
      await explorer.evaluate(element => {
        const dock = element.closest(".workspace-dock");
        dock.style.width = "200px";
        dock.style.maxWidth = "200px";
      });
      const geometry = await explorer.locator(".explorer-grouping").evaluate(element => {
        const select = element.querySelector("select");
        const bounds = element.getBoundingClientRect();
        const control = select.getBoundingClientRect();
        return { width: element.clientWidth, scroll: element.scrollWidth, left: bounds.left, right: bounds.right, controlLeft: control.left, controlRight: control.right, controlWidth: control.width };
      });
      assert(geometry.scroll <= geometry.width + 1 && geometry.controlWidth > 70 && geometry.controlLeft >= geometry.left && geometry.controlRight <= geometry.right,
        `${locale}: grouping control overflows the narrow dock: ${JSON.stringify(geometry)}`);
      if (["zh", "en", "ar"].includes(locale)) await page.screenshot({ path: `${screenshotPrefix}-session-grouping-${locale}.png` });
    }
    assert.deepEqual(errors, []);
  } catch (error) {
    await page.screenshot({ path: `${screenshotPrefix}-session-grouping-failure.png` }).catch(() => {});
    throw error;
  } finally {
    await page.evaluate(async () => {
      (await import("/src/session-explorer-grouping.ts")).writeSessionExplorerGrouping("profile");
      (await import("/src/i18n.ts")).setLanguagePreference("zh");
    }).catch(() => {});
    await page.close();
    await peer.close();
  }
}
