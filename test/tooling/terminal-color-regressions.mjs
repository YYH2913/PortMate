import assert from "node:assert/strict";

const sample = [
  "edge:~$ ip -br address show",
  "eth0     UP     192.0.2.24/24    2001:db8::24/64",
  "",
  "edge:~$ ls -l /etc/portmate",
  "-rw-r-----  1 root dialout  512  2026-11-17  router.conf",
  "",
  "edge:~$ tail -f /var/log/link.log",
  "14:08:21  INFO   peer=192.0.2.24  port=443",
  "14:08:22  WARN   retrying  delay=250ms",
  "14:08:23  ERROR  HTTP/1.1 503 Service Unavailable",
  "14:08:24  OK     HTTP/1.1 200  latency=12ms",
];

// Sample glyph colors *in the specified terminal cells*, not just colors present elsewhere in the screenshot.
async function checkPixels(page, screen, size, regions) {
  // Paint may be deferred during a burst. Poll the actual glyphs, not a fixed sleep or just a decoration count.
  const deadline = Date.now() + 3000;
  let missing;
  do {
    const png = (await screen.screenshot()).toString("base64");
    missing = await page.evaluate(async ({ png, size: [cols, rows], regions }) => {
      const image = new Image(); image.src = `data:image/png;base64,${png}`; await image.decode();
      const canvas = document.createElement("canvas"); canvas.width = image.width; canvas.height = image.height;
      const ctx = canvas.getContext("2d"); ctx.drawImage(image, 0, 0);
      const { data } = ctx.getImageData(0, 0, image.width, image.height);
      return regions.filter(([row, col, count, hex]) => {
        const rgb = hex.match(/../g).map(pair => parseInt(pair, 16));
        for (let y = Math.ceil(row * image.height / rows); y < Math.floor((row + 1) * image.height / rows); y++) {
          for (let x = Math.ceil(col * image.width / cols); x < Math.floor((col + count) * image.width / cols); x++) {
            const i = (y * image.width + x) * 4;
            if (rgb.every((v, ch) => Math.abs(v - data[i + ch]) < 6)) return false;
          }
        }
        return true;
      });
    }, { png, size, regions });
  } while (missing.length && Date.now() < deadline);
  assert.deepEqual(missing, [], "expected glyph/background colors are missing from the selected cell ranges");
}

/** Production workspace and real xterm, with deterministic local transport input (no device). */
export async function checkTerminalColors(page) {
  const host = page.locator('[data-pane-id="pane-a"] .terminal-host');
  const screen = host.locator(".xterm-screen");
  await page.setViewportSize({ width: 1900, height: 1000 });
  for (const theme of ["portmate-dark", "portmate-light"]) {
    await page.evaluate(theme => localStorage.setItem("portmate.compat.terminalTheme", theme), theme);
    await page.reload();
    await page.waitForFunction(theme => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalTheme === theme
      && window.__tauriEventListeners.get("portmate-terminal-live")?.length, theme);
    await page.evaluate(() => {
      let n = 0;
      window.__colorFrame = text => {
        const event = { id: `color-${n++}`, sessionId: "session-a", paneId: "session-a:main", ts: new Date().toISOString(), direction: "inbound", stream: "stdout", text, bytesRef: null, annotations: {} };
        const bytes = Array.from(new TextEncoder().encode(text));
        window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
      };
    });
    await page.evaluate(sample => window.__colorFrame("\x1b[?1049l\x1b[2J\x1b[H" + sample.join("\r\n") + "\r\n"), sample);
    await page.waitForFunction(() => Number(document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalSemanticDecorationCount) >= 12);
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const rect = await screen.boundingBox();
    const size = (await host.getAttribute("data-terminal-size")).split("x").map(Number);
    const [cw, ch] = [rect.width / size[0], rect.height / size[1]];
    if (process.env.PORTMATE_COLOR_SCREENSHOT_PREFIX) {
      await page.screenshot({ path: `${process.env.PORTMATE_COLOR_SCREENSHOT_PREFIX}-${theme}.png`, clip: { x: rect.x, y: rect.y, width: Math.min(rect.width, Math.ceil(cw * 60)), height: Math.min(rect.height, Math.ceil(ch * 12)) } });
    }
    const light = theme === "portmate-light";
    const error = light ? "b13942" : "ff8884", normal = light ? "202630" : "d7e1eb";
    const at = (row, word, color) => [row, sample[row].indexOf(word), word.length, color];
    await checkPixels(page, screen, size, [at(4, "-rw-r-----", light ? "53674d" : "abbba6"),
      at(7, "14:08:21", light ? "5a6879" : "8b9aae"), at(7, "443", light ? "90477a" : "dba8cb"), at(9, "ERROR", error)]);
    const x = rect.x + sample[9].indexOf("ERROR") * cw, y = rect.y + 9.5 * ch;
    await page.keyboard.down("Shift"); await page.mouse.move(x, y); await page.mouse.down();
    await page.mouse.move(x + 5 * cw, y, { steps: 6 }); await page.mouse.up(); await page.keyboard.up("Shift");
    await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalHasSelection === "true");
    await checkPixels(page, screen, size, [at(9, "ERROR", light ? "111827" : "f8fafc")]);
    await page.mouse.click(rect.x + cw * 2, rect.y + ch / 2);

    // A background-only SGR and a real truecolor foreground must not acquire our ERROR paint.
    await page.evaluate(() => window.__colorFrame("\x1b[12;1H\x1b[48;2;19;71;87mERROR\x1b[0m  \x1b[38;2;246;45;191mERROR\x1b[0m  ERROR"));
    await checkPixels(page, screen, size, [[11, 0, 5, normal], [11, 0, 5, "134757"], [11, 7, 5, "f62dbf"], [11, 14, 5, error]]);
    // Same text, different attributes: reusing the lexer result must not reuse stale decorations.
    await page.evaluate(() => window.__colorFrame("\x1b[12;1HERROR  \x1b[38;5;129mERROR\x1b[0m  \x1b[1mERROR\x1b[0m"));
    await checkPixels(page, screen, size, [[11, 0, 5, error], [11, 7, 5, "af00ff"], [11, 14, 5, normal]]);
    // Never-ending CR rewrites (no scrolling) must still acquire color *during* output.
    await page.evaluate(() => {
      window.__colorTicks = 0;
      window.__colorTimer = setInterval(() => window.__colorFrame(`\x1b[13;1HWARN reading ${++window.__colorTicks}`), 40);
    });
    try {
      await page.waitForFunction(() => window.__colorTicks >= 12);
      await checkPixels(page, screen, size, [[12, 0, 4, light ? "905509" : "ffb75e"]]);
    } finally { await page.evaluate(() => clearInterval(window.__colorTimer)); }
    await page.evaluate(() => window.__colorFrame("\x1b[?1049h\x1b[HERROR"));
    await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalSemanticHighlighting === "alternate");
    assert.equal(await host.getAttribute("data-terminal-semantic-decoration-count"), "0");
    const renderer = await host.getAttribute("data-terminal-renderer");
    if (process.env.PORTMATE_UI_FORCE_WEBGL === "1") assert.equal(renderer, "webgl");
    console.log(`Verified semantic, selected, ANSI and streaming colors: ${theme}, ${renderer}`);
  }
  // Use the actual session form. Changing a draft must repaint the preview,
  // not the active connection, and the preview must never send the sample.
  await page.evaluate(() => { window.__invokeCalls.length = 0; });
  const dialog = page.locator(".session-settings-dialog");
  const preview = dialog.getByRole("figure", { name: "配色预览" });
  async function openTerminalSettings() {
    await page.locator(".menu-trigger", { hasText: "会话" }).click();
    await page.locator(".menu-popover button", { hasText: "会话设置" }).click();
    await dialog.getByRole("treeitem", { name: "终端外观", exact: true }).click();
    await preview.waitFor();
  }
  async function assertPreviewFits() {
    assert(await preview.evaluate(el => {
      const box = el.getBoundingClientRect();
      const sample = el.querySelector(".terminal-color-sample").getBoundingClientRect();
      return sample.bottom <= box.bottom + 1 && (!el.nextElementSibling || el.nextElementSibling.getBoundingClientRect().top >= box.bottom - 1)
        && box.width <= el.parentElement.clientWidth + 1;
    }), "the entire sample must fit its own row without clipping or covering the next setting");
  }
  await openTerminalSettings();
  const select = dialog.locator(".dialog-field", { hasText: "主题:" }).locator("select");
  for (const [theme, ink] of [["graphite", "rgb(174, 212, 188)"], ["solarized-dark", "rgb(179, 198, 120)"]]) {
    await select.selectOption(theme);
    await page.waitForFunction(expected => [...document.querySelectorAll(".terminal-color-sample span")]
      .some(token => token.textContent === "tail" && getComputedStyle(token).color === expected), ink, { timeout: 3000 });
    assert.equal(await preview.locator(".terminal-color-swatch-strip i").count(), 16);
  }
  await dialog.locator(".dialog-field", { hasText: "字号:" }).locator("input").fill("15");
  await page.waitForFunction(() => getComputedStyle(document.querySelector(".terminal-color-sample")).fontSize === "15px");
  await assertPreviewFits();
  if (process.env.PORTMATE_COLOR_SCREENSHOT_PREFIX) {
    await preview.screenshot({ path: `${process.env.PORTMATE_COLOR_SCREENSHOT_PREFIX}-settings.png` });
    await dialog.locator(".session-form").evaluate(el => { el.scrollTop = 0; });
    await dialog.screenshot({ path: `${process.env.PORTMATE_COLOR_SCREENSHOT_PREFIX}-dialog.png` });
  }
  await dialog.getByRole("slider").press("Home");
  await page.waitForFunction(() => getComputedStyle(document.querySelector(".terminal-color-sample")).backgroundColor === "rgba(0, 43, 54, 0.2)");
  await page.setViewportSize({ width: 520, height: 640 });
  await assertPreviewFits();
  assert.equal(await host.getAttribute("data-terminal-theme"), "portmate-light");
  assert.equal(await page.evaluate(() => window.__invokeCalls.some(call => ["send_text", "run_command", "send_bytes", "connect_session", "save_session_profile"].includes(call.command))), false);

  await page.evaluate(() => {
    const prefs = JSON.parse(localStorage.getItem("portmate.terminalPrefs"));
    localStorage.setItem("portmate.terminalPrefs", JSON.stringify({ ...prefs, semanticHighlightingEnabled: false }));
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.reload();
  await openTerminalSettings();
  assert.match(await preview.innerText(), /HTTP\/2 503/);
  assert.equal(await preview.locator(".terminal-color-sample span").count(), 0, "respect the global plain-output preference");
  assert.equal(await preview.locator(".terminal-color-swatch-strip i").count(), 16, "ANSI swatches are independent of local coloring");
  console.log("Verified theme/font/opacity previews, form layout and plain-output preference");
}
