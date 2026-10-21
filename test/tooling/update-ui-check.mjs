import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright-core";

const server = await createServer({ server: { host: "127.0.0.1", port: 0, strictPort: false, watch: { usePolling: false } } });
let browser;
try {
  await server.listen();
  const address = server.httpServer.address();
  const url = `http://127.0.0.1:${address.port}/test/fixtures/updater/ui.html`;
  browser = await chromium.launch({ executablePath: process.env.PORTMATE_CHROME ?? "/usr/bin/google-chrome", headless: true, args: ["--no-sandbox"] });
  async function pageFor(mode) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await page.addInitScript((mode) => {
      window.installs = 0; window.checks = 0;
      if (mode === "browser") return;
      window.__TAURI_INTERNALS__ = {
        transformCallback: () => 1, unregisterCallback() {},
        async invoke(command, args) {
          if (command === "check_for_updates") {
            window.checks += 1;
            if (mode === "failure") throw { code: "check-failed", detail: "network failure" };
            return { currentVersion: "0.1.13", version: "0.2.0", available: mode !== "current", candidateId: mode === "manual" ? null : "checked-candidate", manualReason: mode === "manual" ? "unsupported-package" : null, releaseUrl: "https://github.com/YYH2913/PortMate/releases/tag/0.2.0", notes: "English / 中文", publishedAt: null };
          }
          if (command === "install_update") {
            window.installs += 1;
            if (args.candidateId !== "checked-candidate") throw Error("wrong update candidate");
            window.progress = args.onProgress;
            await new Promise((resolve, reject) => { window.finishInstall = resolve; window.failInstall = reject; });
            return;
          }
          throw Error(`unexpected native command: ${command}`);
        },
      };
    }, mode);
    await page.goto(url);
    return page;
  }

  const page = await pageFor("available");
  const dialog = page.getByRole("dialog", { name: "Check for updates" });
  await dialog.getByText("PortMate 0.2.0 is available.", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.installs), 0);
  page.once("dialog", async prompt => { assert.match(prompt.message(), /all PortMate windows will close/); await prompt.dismiss(); });
  await dialog.getByRole("button", { name: "Update and restart", exact: true }).click();
  assert.equal(await page.evaluate(() => window.installs), 0, "declining must not start the update");
  page.once("dialog", prompt => prompt.accept());
  await dialog.getByRole("button", { name: "Update and restart", exact: true }).click();
  await page.waitForFunction(() => window.installs === 1);
  await page.evaluate(() => window.progress.onmessage({ phase: "downloading", downloaded: 256_000, total: 1_024_000 }));
  const progress = dialog.getByRole("progressbar");
  await progress.waitFor();
  assert.equal(await progress.getAttribute("value"), "256000");
  assert.equal(await progress.getAttribute("max"), "1024000");
  assert.ok(await dialog.getByRole("button", { name: "Update and restart", exact: true }).isDisabled());
  await page.keyboard.press("Escape");
  assert.equal(await page.evaluate(() => !!window.closedUpdater), false);
  await page.evaluate(() => window.failInstall({ code: "download-failed", detail: "invalid signature" }));
  await dialog.getByRole("alert").getByText(/Your sessions are still running/).waitFor();
  await page.evaluate(() => window.progress.onmessage({ phase: "preparing", downloaded: 0, total: null }));
  assert.ok(await dialog.getByRole("button", { name: "Check for updates", exact: true }).isEnabled(), "late progress must not override a failed request");
  await mkdir(resolve("tmp/updater-review"), { recursive: true });
  await page.screenshot({ path: resolve("tmp/updater-review/update-dialog.png") });
  await dialog.getByRole("button", { name: "Check for updates", exact: true }).click();
  await dialog.getByText("PortMate 0.2.0 is available.", { exact: true }).waitFor();
  page.once("dialog", prompt => prompt.accept());
  await dialog.getByRole("button", { name: "Update and restart", exact: true }).click();
  await page.waitForFunction(() => window.installs === 2);
  for (const phase of ["verifying", "preparing", "installing"]) {
    await page.evaluate(phase => window.progress.onmessage({ phase, downloaded: 0, total: null }), phase);
    await page.waitForFunction(phase => document.querySelector('[role="status"]').textContent.includes(({ verifying: "Verifying", preparing: "Closing sessions", installing: "Installing" })[phase]), phase);
  }
  await page.evaluate(() => window.finishInstall());
  assert.ok(await dialog.getByRole("button", { name: "Close", exact: true }).last().isDisabled(), "success must stay locked until native restart");
  await page.close();

  for (const [mode, text] of [["current", "You are using the latest stable version."], ["manual", "This package requires a manual upgrade."], ["failure", "Unable to check for updates."], ["browser", "Update checks are available in the desktop application."]]) {
    const other = await pageFor(mode);
    await other.getByText(text, { exact: mode === "current" || mode === "browser" }).waitFor();
    assert.equal(await other.getByRole("button", { name: "Update and restart", exact: true }).count(), 0);
    await other.close();
  }
  console.log("Updater UI checks passed: confirmation/cancel, live progress, duplicate/close guards, failure recovery, manual upgrade, current version, browser preview.");
} finally {
  await browser?.close();
  await server.close();
}
