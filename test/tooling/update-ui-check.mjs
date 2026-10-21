import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright-core";

const server = await createServer({ server: { host: "127.0.0.1", port: 0, strictPort: false, watch: { usePolling: false } } });
const releaseNotes = [
  "## New in 0.2.0 / 主要更新", "", "**Faster uploads** and *serial fixes*.", "",
  "- Keep `\\r\\n` endings", "- 中文说明", "",
  "| Platform | Package |", "| --- | --- |", "| Windows | MSI |", "",
  "> Existing sessions are preserved.", "",
  "```sh", `printf '${"wide-code-".repeat(20)}\\r\\n'`, "```", "",
  "[Documentation](https://github.com/YYH2913/PortMate#readme)", "",
  "![Screenshot](https://images.example.test/release.png)", "",
  "[Unsafe](javascript:alert%281%29)", "",
  "<script>window.releaseNotesExecuted = true</script>",
];
let browser;
try {
  await server.listen();
  const address = server.httpServer.address();
  const url = `http://127.0.0.1:${address.port}/test/fixtures/updater/ui.html`;
  browser = await chromium.launch({ executablePath: process.env.PORTMATE_CHROME ?? "/usr/bin/google-chrome", headless: true, args: ["--no-sandbox"] });
  async function pageFor(mode, viewport = { width: 1280, height: 800 }) {
    const page = await browser.newPage({ viewport });
    await page.addInitScript(({ mode, notes }) => {
      window.installs = 0; window.checks = 0;
      window.open = (...args) => { window.openedReleaseLink = args; return null; };
      if (mode === "browser") return;
      window.__TAURI_INTERNALS__ = {
        transformCallback: () => 1, unregisterCallback() {},
        async invoke(command, args) {
          if (command === "check_for_updates") {
            window.checks += 1;
            if (mode === "failure") throw { code: "check-failed", detail: "network failure" };
            return { currentVersion: "0.1.12", version: "0.2.0", available: mode !== "current", candidateId: mode === "manual" ? null : "checked-candidate", manualReason: mode === "manual" ? "unsupported-package" : null, releaseUrl: "https://github.com/YYH2913/PortMate/releases/tag/0.2.0", notes, publishedAt: null };
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
    }, { mode, notes: releaseNotes.join("\n") });
    await page.goto(url);
    return page;
  }

  const page = await pageFor("available");
  const dialog = page.getByRole("dialog", { name: "Check for updates" });
  await dialog.getByText("PortMate 0.2.0 is available.", { exact: true }).waitFor();
  await dialog.getByRole("heading", { name: "New in 0.2.0 / 主要更新", level: 2 }).waitFor();
  assert.equal(await dialog.locator("details").getAttribute("open"), "");
  assert.equal(await dialog.getByRole("cell", { name: "MSI", exact: true }).count(), 1);
  assert.equal(await dialog.locator("strong").filter({ hasText: "Faster uploads" }).count(), 1);
  assert.equal(await dialog.getByRole("link", { name: "Unsafe", exact: true }).count(), 0);
  assert.equal(await dialog.locator("img").count(), 0);
  assert.equal(await page.evaluate(() => window.releaseNotesExecuted), undefined);
  await dialog.getByRole("button", { name: "Close", exact: true }).first().focus();
  await page.keyboard.press("Tab");
  assert.equal(await page.evaluate(() => document.activeElement.tagName), "SUMMARY");
  await page.keyboard.press("Tab");
  assert.equal(await dialog.getByRole("link", { name: "Documentation", exact: true }).evaluate(link => link === document.activeElement), true);
  await page.keyboard.press("Enter");
  assert.deepEqual(await page.evaluate(() => window.openedReleaseLink), ["https://github.com/YYH2913/PortMate#readme", "_blank", "noopener,noreferrer"]);
  await dialog.locator("summary").click();
  assert.equal(await dialog.locator("details").getAttribute("open"), null);
  await dialog.locator("summary").focus();
  await page.keyboard.press("Tab");
  assert.equal(await dialog.getByRole("button", { name: "Releases", exact: true }).evaluate(button => button === document.activeElement), true, "collapsed notes must not trap focus in hidden links");
  await dialog.locator("summary").click();
  const bounds = await dialog.evaluate(element => {
    const rect = element.getBoundingClientRect();
    const content = element.querySelector(".update-content");
    const code = element.querySelector("pre");
    return { top: rect.top, bottom: rect.bottom, height: window.innerHeight, overflow: content.scrollWidth > content.clientWidth, codeScrolls: code.scrollWidth > code.clientWidth };
  });
  assert.ok(bounds.top >= 0 && bounds.bottom <= bounds.height);
  assert.equal(bounds.overflow, false, "long code must not widen the whole dialog");
  assert.equal(bounds.codeScrolls, true);
  await mkdir(resolve("tmp/updater-review"), { recursive: true });
  await dialog.locator(".update-content").evaluate(element => { element.scrollTop = 0; });
  await page.screenshot({ path: resolve("tmp/updater-review/update-markdown.png") });
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
  assert.equal(await dialog.getByRole("link", { name: "Documentation", exact: true }).getAttribute("tabindex"), "-1");
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
  const compact = await pageFor("available", { width: 800, height: 540 });
  await compact.getByRole("heading", { name: "New in 0.2.0 / 主要更新" }).waitFor();
  assert.equal(await compact.getByRole("dialog").evaluate(element => {
    const bounds = element.getBoundingClientRect();
    const content = element.querySelector(".update-content");
    return bounds.top >= 0 && bounds.bottom <= window.innerHeight && content.scrollHeight > content.clientHeight;
  }), true, "long release notes must scroll within a short viewport");
  await compact.screenshot({ path: resolve("tmp/updater-review/update-markdown-compact.png") });
  await compact.close();
  console.log("Updater UI checks passed: Markdown rendering and safe links, keyboard navigation, compact layout, confirmation/cancel, live progress, duplicate/close guards, failure recovery, manual upgrade, current version, browser preview.");
} finally {
  await browser?.close();
  await server.close();
}
