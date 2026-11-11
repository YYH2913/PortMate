import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const messages = Object.fromEntries(["zh", "en", "fr", "es", "ru", "ar"].map(locale => [
  locale, JSON.parse(readFileSync(new URL(`../../src/locales/${locale}.json`, import.meta.url), "utf8")),
]));

async function checkBounds(dialog) {
  const geometry = await dialog.evaluate(element => {
    const rect = element.getBoundingClientRect();
    const action = element.querySelector(".utility-actions button[type=submit]").getBoundingClientRect();
    const content = element.querySelector(".utility-content");
    return {
      left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom,
      width: window.innerWidth, height: window.innerHeight,
      overflow: content.scrollWidth - content.clientWidth,
      actionTop: action.top, actionBottom: action.bottom,
    };
  });
  assert(geometry.left >= 0 && geometry.right <= geometry.width
    && geometry.top >= 0 && geometry.bottom <= geometry.height,
  `tunnel dialog escaped the viewport: ${JSON.stringify(geometry)}`);
  assert(geometry.overflow <= 1, `tunnel content overflows horizontally: ${JSON.stringify(geometry)}`);
  assert(geometry.actionTop >= geometry.top && geometry.actionBottom <= geometry.bottom,
    `create action is clipped outside the dialog: ${JSON.stringify(geometry)}`);
  assert(await dialog.locator(".utility-actions button[type=submit]").isVisible(), "create action is inaccessible");
}

export async function checkTunnelHelp(context, appUrl, screenshotPrefix) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.goto(appUrl);
    await page.locator(".wind-root").waitFor();
    await page.locator('.menu-trigger[aria-controls="menu-tools"]').click();
    await page.locator("#menu-tools").getByRole("button", { name: messages.zh["port-forwarding"], exact: true }).click();
    const dialog = page.locator(".tunnel-dialog");
    await dialog.waitFor();
    const mode = dialog.getByRole("combobox");
    const flow = dialog.locator(".tunnel-flow");
    const sessionName = await dialog.getByLabel(messages.zh["tunnel-session"], { exact: true }).inputValue();
    assert.equal(sessionName, "Edge Router");
    assert((await flow.textContent()).includes("10.0.0.1:2222"), "direction preview omitted the actual SSH endpoint");

    for (const locale of ["zh", "en", "fr", "es", "ru", "ar"]) {
      await page.evaluate(async locale => (await import("/src/i18n.ts")).setLanguagePreference(locale), locale);
      const m = messages[locale];
      await dialog.getByLabel(m["tunnel-session"], { exact: true }).waitFor();
      assert.equal(await page.locator("html").getAttribute("dir"), locale === "ar" ? "rtl" : "ltr");
      for (const kind of ["local", "remote", "dynamic"]) {
        await mode.selectOption(kind);
        assert((await dialog.locator(".tunnel-guidance > p").textContent()).includes(m[`tunnel-help-${kind}`]));
        const bindSide = kind === "remote" ? "remote" : "local";
        const bind = dialog.getByLabel(m[`tunnel-bind-${bindSide}`], { exact: true });
        assert(await bind.isVisible(), `${locale}/${kind}: entry has no matching accessible label`);
        assert.equal(await bind.inputValue(), "127.0.0.1", "mode change modified the bind address");
        if (kind === "dynamic") {
          assert.equal(await dialog.getByLabel(m["tunnel-target-service"], { exact: true }).count(), 0);
          assert((await flow.textContent()).includes(m["tunnel-app-selected-target"]));
          assert((await dialog.locator(".tunnel-routes").textContent()).includes(m["tunnel-socks-hint"]));
        } else {
          const target = dialog.getByLabel(m["tunnel-target-service"], { exact: true });
          assert.equal(await target.inputValue(), "127.0.0.1", "mode change modified the target address");
          assert((await dialog.locator(".tunnel-field-hint").allTextContents()).includes(
            m[kind === "remote" ? "tunnel-target-local-hint" : "tunnel-target-ssh-hint"],
          ));
        }
        const brokenDescriptions = await dialog.locator("[aria-describedby]").evaluateAll(elements => elements
          .flatMap(element => element.getAttribute("aria-describedby").split(/\s+/))
          .filter(id => !document.getElementById(id)?.textContent?.trim()));
        assert.deepEqual(brokenDescriptions, [], `${locale}/${kind}: hints lost their accessible association`);
        await checkBounds(dialog);
        if (["zh", "en", "ar"].includes(locale)) {
          await page.screenshot({ path: `${screenshotPrefix}-tunnel-${locale}-${kind}.png` });
        }
      }
      const help = dialog.locator(".tunnel-connection-help");
      await help.locator("summary").click();
      assert(await help.getByText(m["tunnel-bind-warning"], { exact: true }).isVisible());
      assert(await help.getByText(m["tunnel-source-note"], { exact: true }).isVisible());
      await mode.selectOption("remote");
      assert(await help.getByText(m["tunnel-remote-policy"], { exact: true }).isVisible());
      await mode.selectOption("local");
      assert.equal(await help.getByText(m["tunnel-remote-policy"], { exact: true }).count(), 0);
      await help.locator("summary").click();

      await page.setViewportSize({ width: 390, height: 844 });
      for (const kind of ["local", "remote", "dynamic"]) {
        await mode.selectOption(kind);
        await checkBounds(dialog);
      }
      if (["zh", "en", "ar"].includes(locale)) {
        await mode.selectOption("local");
        await dialog.locator(".utility-content").evaluate(element => { element.scrollTop = 0; });
        await page.screenshot({ path: `${screenshotPrefix}-tunnel-${locale}-narrow.png` });
      }
      await page.setViewportSize({ width: 1280, height: 900 });
    }

    await page.evaluate(async () => (await import("/src/i18n.ts")).setLanguagePreference("zh"));
    await mode.selectOption("local");
    const bind = dialog.getByLabel(messages.zh["tunnel-bind-local"], { exact: true });
    const port = dialog.getByLabel(messages.zh["tunnel-entry-port"], { exact: true });
    const target = dialog.getByLabel(messages.zh["tunnel-target-service"], { exact: true });
    const targetPort = dialog.getByLabel(messages.zh["tunnel-target-service-port"], { exact: true });
    await bind.fill("::1");
    await port.fill("0");
    await target.fill("2001:db8::2");
    await targetPort.fill("8080");
    assert((await flow.textContent()).includes("[::1]:自动分配"));
    assert((await flow.textContent()).includes("[2001:db8::2]:8080"));
    await targetPort.fill("0");
    assert((await flow.textContent()).includes("[2001:db8::2]:0"), "invalid target port was presented as automatically assigned");
    assert(await dialog.locator(".utility-actions button[type=submit]").isDisabled());
    await targetPort.fill("8080");
    await page.setViewportSize({ width: 390, height: 844 });
    await target.fill(`${"a".repeat(63)}.${"b".repeat(63)}.${"c".repeat(63)}.${"d".repeat(61)}`);
    await checkBounds(dialog);
    await target.fill("2001:db8::2");
    await page.setViewportSize({ width: 320, height: 568 });
    await dialog.locator(".tunnel-connection-help summary").click();
    await checkBounds(dialog);
    await targetPort.fill("8081");
    assert(await targetPort.isVisible(), "short-window form cannot scroll to the target port");
    await targetPort.fill("8080");
    await dialog.locator(".tunnel-connection-help summary").click();
    await page.setViewportSize({ width: 1280, height: 900 });

    for (const kind of ["local", "remote", "dynamic"]) {
      // Successful creation closes the dialog; reopen and retain explicit test values.
      if (await dialog.count() === 0) {
        await page.locator('.menu-trigger[aria-controls="menu-tools"]').click();
        await page.locator("#menu-tools").getByRole("button", { name: messages.zh["port-forwarding"], exact: true }).click();
        await dialog.waitFor();
        await bind.fill("::1");
        await port.fill("0");
        await target.fill("2001:db8::2");
        await targetPort.fill("8080");
      }
      await mode.selectOption(kind);
      if (kind === "dynamic") {
        await dialog.getByRole("button", { name: messages.zh["add-target-route"], exact: true }).click();
        await dialog.getByRole("textbox", { name: "路由目标 1", exact: true }).fill("10.9.8.7/8");
        await dialog.getByRole("spinbutton", { name: "路由端口 1", exact: true }).fill("443");
      }
      await dialog.locator(".utility-actions button[type=submit]").click();
      const notice = page.locator(".notice-dialog");
      await notice.waitFor();
      const request = await page.evaluate(() => window.__invokeCalls.findLast(call => call.command === "create_tunnel").args.request);
      assert.deepEqual(request, {
        sessionId: "edge-router", egress: "ssh", mode: kind, bindHost: "::1", bindPort: 0,
        targetHost: kind === "dynamic" ? "" : "2001:db8::2", targetPort: kind === "dynamic" ? 0 : 8080,
        routeRules: kind === "dynamic" ? [{ host: "10.0.0.0/8", port: 443 }] : [],
      }, "presentation changed the backend tunnel request");
      await notice.getByRole("button", { name: "确定", exact: true }).click();
      await notice.waitFor({ state: "detached" });
    }
    assert.deepEqual(errors, [], "tunnel help produced browser exceptions");
  } finally {
    await page.evaluate(async () => (await import("/src/i18n.ts")).setLanguagePreference("zh")).catch(() => {});
    await page.close();
  }
}
