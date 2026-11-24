import assert from "node:assert/strict";

const appearanceKey = "portmate.uiAppearance.v1";
const defaultAppearance = { theme: "studio-light", layout: "docked", accent: "default" };

async function expectAppearance(page, expected) {
  await page.waitForFunction(({ theme, layout, accent }) => {
    const root = document.documentElement;
    return root.dataset.uiTheme === theme && root.dataset.uiLayout === layout && root.dataset.uiAccent === accent;
  }, expected);
}

async function readAppearance(page) {
  return page.evaluate(key => JSON.parse(localStorage.getItem(key)), appearanceKey);
}

async function assertInViewport(locator) {
  const bounds = await locator.evaluate(element => {
    const { left, right, top, bottom } = element.getBoundingClientRect();
    return { left, right, top, bottom, width: window.innerWidth, height: window.innerHeight };
  });
  assert.ok(bounds.left >= -1 && bounds.right <= bounds.width + 1
    && bounds.top >= 0 && bounds.bottom <= bounds.height + 1,
  `Control outside the viewport: ${JSON.stringify(bounds)}`);
}

async function checkButtonContrast(page) {
  const lowContrast = await page.evaluate(async () => {
    const { uiThemePresets, uiAccentColors } = await import("/src/ui-theme.ts");
    const root = document.documentElement;
    const { uiTheme, uiAccent } = root.dataset;
    const failures = [];
    const luminance = hex => [0.2126, 0.7152, 0.0722].reduce((total, weight, index) => {
      const channel = parseInt(hex.slice(1 + index * 2, 3 + index * 2), 16) / 255;
      const linear = channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
      return total + weight * linear;
    }, 0);
    try {
      for (const theme of uiThemePresets) {
        root.dataset.uiTheme = theme;
        for (const accent of uiAccentColors) {
          root.dataset.uiAccent = accent;
          const style = getComputedStyle(root);
          for (const family of ["accent", "danger"]) {
            const text = luminance(style.getPropertyValue(`--${family}-fg`).trim());
            for (const fill of ["solid", "solid-hover"]) {
              const background = luminance(style.getPropertyValue(`--${family}-${fill}`).trim());
              const contrast = (Math.max(text, background) + 0.05) / (Math.min(text, background) + 0.05);
              if (!(contrast >= 4.5)) failures.push({ theme, accent, family, fill, contrast });
            }
          }
        }
      }
    } finally {
      root.dataset.uiTheme = uiTheme;
      root.dataset.uiAccent = uiAccent;
    }
    return failures;
  });
  assert.deepEqual(lowContrast, []);
}

export async function checkUiAppearance(context, appUrl) {
  const page = await context.newPage();
  const peer = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  peer.on("pageerror", error => errors.push(error.message));
  const quick = page.locator(".menu-appearance-trigger");
  const palette = page.locator("#menu-appearance");
  const preferences = page.locator(".terminal-settings-dialog");
  const t = key => page.evaluate(async key => (await import("/src/i18n.ts")).t(key), key);

  try {
    // These browser fixtures reset storage on their first navigation.
    // Open both before making any appearance changes.
    await page.goto(appUrl);
    await peer.goto(appUrl);
    await quick.waitFor();
    await peer.locator(".menu-appearance-trigger").waitFor();
    await expectAppearance(page, defaultAppearance);
    await checkButtonContrast(page);
    await page.setViewportSize({ width: 390, height: 430 });
    await page.evaluate(() => { window.__mcpHttpRuntime.phase = "running"; });
    await page.locator(".menu-mcp-start .menu-status-dot").waitFor({ timeout: 10_000 });

    for (const [locale, direction] of [["en", "ltr"], ["ar", "rtl"]]) {
      await page.evaluate(async locale => (await import("/src/i18n.ts")).setLanguagePreference(locale), locale);
      await page.waitForFunction(direction => document.documentElement.dir === direction, direction);
      for (const width of [320, 850, 960, 1100]) {
        await page.setViewportSize({ width, height: 430 });
        await assertInViewport(quick);
        await assertInViewport(page.locator(".menu-command-pill"));
        await quick.click();
        await assertInViewport(palette);
        await page.waitForFunction(() => document.querySelector("#menu-appearance")?.contains(document.activeElement));
        await page.keyboard.press("Escape");
        await palette.waitFor({ state: "detached" });
        await page.waitForFunction(() => document.activeElement?.matches(".menu-appearance-trigger"));
        const tools = page.locator('[aria-controls="menu-tools"]');
        await tools.click();
        const toolsMenu = page.locator("#menu-tools");
        await assertInViewport(toolsMenu);
        await page.waitForFunction(() => document.querySelector("#menu-tools")?.contains(document.activeElement));
        await page.keyboard.press("End");
        const selected = toolsMenu.locator(":focus");
        await assertInViewport(selected);
        await page.keyboard.press("Escape");
        await toolsMenu.waitFor({ state: "detached" });
      }
    }

    await page.setViewportSize({ width: 1440, height: 900 });
    await page.evaluate(async () => (await import("/src/i18n.ts")).setLanguagePreference("en"));
    await quick.click();
    await page.waitForFunction(() => document.activeElement?.getAttribute("data-theme-choice") === "studio-light");
    await page.keyboard.press("ArrowDown");
    const carbon = { ...defaultAppearance, theme: "carbon-islands" };
    await expectAppearance(page, carbon);
    await expectAppearance(peer, carbon);
    assert.deepEqual(await readAppearance(page), carbon);
    await page.keyboard.press("Tab");
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("data-layout-choice")), "docked");
    await page.keyboard.press("Escape");
    await palette.waitFor({ state: "detached" });

    const railExplorer = page.locator('.workspace-rail-btn[data-rail-panel="explorer"]');
    await railExplorer.click();
    const changedSelection = await railExplorer.getAttribute("aria-pressed");
    await railExplorer.click();
    assert.notEqual(await railExplorer.getAttribute("aria-pressed"), changedSelection);

    const advanced = async () => {
      await quick.click();
      await palette.locator(".ui-appearance-more").click();
      await preferences.locator(".ui-appearance-picker").waitFor();
    };
    await advanced();
    await preferences.locator('[data-accent-choice="rose"]').click();
    await expectAppearance(page, { ...carbon, accent: "rose" });
    assert.deepEqual(await readAppearance(page), carbon, "a settings preview was saved before confirmation");
    await expectAppearance(peer, carbon);
    await preferences.getByRole("button", { name: await t("cancel"), exact: true }).click();
    await page.locator("dialog[open].settings-discard").getByRole("button", { name: await t("discard-changes"), exact: true }).click();
    await preferences.waitFor({ state: "detached" });
    await expectAppearance(page, carbon);

    await advanced();
    await preferences.locator('[data-accent-choice="emerald"]').click();
    await peer.locator(".menu-appearance-trigger").click();
    await peer.locator('#menu-appearance [data-layout-choice="islands"]').click();
    const preview = { ...carbon, layout: "islands", accent: "emerald" };
    await expectAppearance(page, preview);
    assert.deepEqual(await readAppearance(page), { ...preview, accent: "default" });
    await preferences.getByRole("button", { name: await t("save"), exact: true }).click();
    await preferences.waitFor({ state: "detached" });
    await expectAppearance(peer, preview);
    assert.deepEqual(await readAppearance(page), preview, "saving a draft lost another window's layout change");
    await page.reload();
    await quick.waitFor();
    await expectAppearance(page, preview);
    assert.deepEqual(errors, []);
  } finally {
    await Promise.all([page.close(), peer.close()]);
  }
}
