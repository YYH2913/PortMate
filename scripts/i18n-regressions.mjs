import assert from "node:assert/strict";

export async function checkI18n(context, appUrl, screenshotPrefix = "/tmp/portmate-i18n") {
  const page = await context.newPage();
  const errors = [];
  let preferencesBefore;
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.goto(appUrl);
    const host = page.locator('.terminal-pane.active .terminal-host[data-terminal-ready="true"]');
    await host.waitFor();
    const instance = await host.getAttribute("data-terminal-instance-id");
    const select = page.locator(".menu-primary .language-selector select");
    preferencesBefore = await page.evaluate(() => localStorage.getItem("portmate.terminalPrefs"));
    const expected = {
      ar: ["جلسة", "الطرفية", "مساحة العمل", "الأدوات"],
      zh: ["会话", "终端", "工作区", "工具"],
      en: ["Session", "Terminal", "Workspace", "Tools"],
      fr: ["Session", "Terminal", "Espace de travail", "Outils"],
      ru: ["Сеанс", "Терминал", "Рабочая область", "Инструменты"],
      es: ["Sesión", "Terminal", "Espacio de trabajo", "Herramientas"],
    };
    for (const locale of ["ar", "en", "fr", "ru", "es", "zh"]) {
      await select.selectOption(locale);
      await page.waitForFunction(locale => document.documentElement.lang === (locale === "zh" ? "zh-CN" : locale), locale);
      assert.deepEqual(await page.locator(".menu-trigger").allTextContents(), expected[locale]);
      assert.equal(await page.locator("html").getAttribute("dir"), locale === "ar" ? "rtl" : "ltr");
      assert.equal(await host.getAttribute("data-terminal-instance-id"), instance, "language switching recreated the terminal");
      assert.equal(await host.evaluate(element => getComputedStyle(element).direction), "ltr", "UI direction leaked into terminal content");
      assert.equal(await page.evaluate(() => localStorage.getItem("portmate.language.v1")), locale);
      assert.equal(await page.evaluate(() => localStorage.getItem("portmate.terminalPrefs")), preferencesBefore,
        "changing UI language rewrote terminal preferences");
      assert.deepEqual(await page.locator(".menu-trigger").evaluateAll(elements => elements.map(element => element.getAttribute("aria-controls"))),
        ["menu-session", "menu-terminal", "menu-workspace", "menu-tools"], "menu identifiers changed with language");
    }
    await page.evaluate(async () => {
      const { requestTerminalFreeInput } = await import("/src/terminal-free-input.ts");
      requestTerminalFreeInput(window, 'printf "العربية 中文 Tools /tmp/file.txt"');
    });
    const draft = page.locator(".terminal-free-input textarea");
    const value = await draft.inputValue();
    await select.selectOption("ar");
    assert.equal(await draft.inputValue(), value, "language switching altered an editor draft");
    assert.equal(await draft.evaluate(element => getComputedStyle(element).direction), "ltr");
    await page.screenshot({ path: `${screenshotPrefix}-arabic.png`, fullPage: true });
    await page.locator(".terminal-free-input button[type=button]").click();
    const message = key => page.evaluate(async key => (await import("/src/i18n.ts")).t(key), key);
    await page.locator(".menu-trigger[aria-controls=menu-tools]").click();
    await page.locator("#menu-tools").getByRole("button", { name: await message("terminal-settings"), exact: true }).click();
    const settings = page.locator(".terminal-settings-dialog");
    await settings.getByRole("tab", { name: await message("autocomplete"), exact: true }).click();
    await page.screenshot({ path: `${screenshotPrefix}-arabic-settings.png`, fullPage: true });
    await settings.locator("select").filter({ has: page.locator('option[value="3"]') }).selectOption("3");
    await settings.locator("select").filter({ has: page.locator('option[value="5"]') }).selectOption("5");
    await settings.locator("select").filter({ has: page.locator('option[value="top"]') }).selectOption("top");
    await settings.getByRole("button", { name: await message("save"), exact: true }).click();
    await settings.waitFor({ state: "detached" });
    const saved = await page.evaluate(() => JSON.parse(localStorage.getItem("portmate.terminalPrefs")));
    assert.equal(saved.completionTriggerChars, 3);
    assert.equal(saved.completionListHeight, 5);
    assert.equal(saved.completionPreviewMode, "top");
    assert.equal(await host.getAttribute("data-terminal-instance-id"), instance, "saving translated settings recreated the terminal");
    await page.locator(".menu-trigger[aria-controls=menu-session]").click();
    await page.locator("#menu-session").getByRole("button", { name: await message("session-settings"), exact: true }).click();
    const sessionSettings = page.locator(".session-settings-dialog");
    await sessionSettings.getByRole("combobox", { name: await message("session-type"), exact: true }).selectOption("Serial");
    await sessionSettings.getByRole("combobox", { name: await message("session-settings-pages"), exact: true }).selectOption("serial");
    const parity = sessionSettings.locator("select").filter({ has: page.locator('option[value="odd"]') });
    const flow = sessionSettings.locator("select").filter({ has: page.locator('option[value="hardware"]') });
    assert.deepEqual(await parity.locator("option").evaluateAll(options => options.map(option => option.value)), ["none", "odd", "even"]);
    assert.deepEqual(await flow.locator("option").evaluateAll(options => options.map(option => option.value)), ["none", "software", "hardware"]);
    await parity.selectOption("odd");
    await flow.selectOption("hardware");
    await page.evaluate(async () => (await import("/src/i18n.ts")).setLanguagePreference("fr"));
    assert.equal(await parity.inputValue(), "odd");
    assert.equal(await flow.inputValue(), "hardware");
    await page.screenshot({ path: `${screenshotPrefix}-french-serial-settings.png`, fullPage: true });
    await sessionSettings.locator(".dialog-title button").click();
    await sessionSettings.waitFor({ state: "detached" });
    await page.evaluate(async () => {
      const { updateSystemLanguage, setLanguagePreference } = await import("/src/i18n.ts");
      updateSystemLanguage("de-DE");
      setLanguagePreference("system");
    });
    await page.waitForFunction(() => document.documentElement.lang === "en");
    assert.deepEqual(await page.locator(".menu-trigger").allTextContents(), expected.en);
    await page.evaluate(async () => {
      const { updateSystemLanguage } = await import("/src/i18n.ts");
      updateSystemLanguage("ar-SA");
    });
    await page.waitForFunction(() => document.documentElement.dir === "rtl");
    const peer = await context.newPage();
    await peer.goto(appUrl);
    await peer.locator(".menu-primary .language-selector select").waitFor();
    await select.selectOption("fr");
    await peer.waitForFunction(() => document.documentElement.lang === "fr");
    await peer.reload();
    await peer.waitForFunction(() => document.documentElement.lang === "fr");
    await peer.close();
    await page.setViewportSize({ width: 390, height: 844 });
    for (const locale of ["fr", "ru", "es", "ar"]) {
      await select.selectOption(locale);
      const geometry = await page.evaluate(() => {
        const tools = document.querySelector(".menu-tools").getBoundingClientRect();
        return [...document.querySelectorAll(".menu-trigger")].map(element => {
          const rect = element.getBoundingClientRect();
          return { left: rect.left, right: rect.right, overlap: rect.left < tools.right && rect.right > tools.left && rect.top < tools.bottom && rect.bottom > tools.top,
            clipped: element.scrollWidth > element.clientWidth };
        });
      });
      assert(geometry.every(rect => rect.left >= 0 && rect.right <= 390 && !rect.overlap && !rect.clipped),
        `${locale} menu controls overlap or are clipped: ${JSON.stringify(geometry)}`);
    }
    await page.screenshot({ path: `${screenshotPrefix}-arabic-mobile.png`, fullPage: true });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth), 390, "Arabic layout overflows the viewport");
    assert.deepEqual(errors, []);
  } finally {
    await page.evaluate(async preferencesBefore => {
      if (preferencesBefore === null) localStorage.removeItem("portmate.terminalPrefs");
      else if (preferencesBefore !== undefined) localStorage.setItem("portmate.terminalPrefs", preferencesBefore);
      const { setLanguagePreference } = await import("/src/i18n.ts");
      setLanguagePreference("zh");
    }, preferencesBefore).catch(() => {});
    await page.close();
  }
}
