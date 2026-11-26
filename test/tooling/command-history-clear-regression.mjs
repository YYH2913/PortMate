import assert from "node:assert/strict";
import { respondAppConfirm } from "./app-dialog-helpers.mjs";

export async function checkHistoryClearRace(context, appUrl) {
  const page = await context.newPage();
  const url = new URL(appUrl);
  url.searchParams.set("portmateTestHistory", "clear-race");
  await page.goto(url.href);
  await page.waitForFunction(() => typeof window.__rejectHistoryStartupRead === "function");
  await page.waitForSelector('.terminal-canvas[data-terminal-view-id="view-edge"] .xterm-screen');
  const rows = page.locator(".history-list .history-entry-pick");
  const clear = page.locator(".history-clear-button");
  const historyDock = page.locator('.workspace-dock[data-active-panel="history"]');
  if (!(await historyDock.isVisible())) {
    await page.locator(".menu-trigger", { hasText: "工作区" }).click();
    await page.locator(".menu-popover button", { hasText: "历史命令" }).click();
  }
  await rows.first().waitFor();
  const wrongTargetInput = page.locator(".terminal-free-input textarea");
  await page.evaluate(async () => {
    const { requestTerminalFreeInput } = await import("/src/terminal-free-input.ts");
    requestTerminalFreeInput(window, "Wrong view", { sessionId: "edge-router", viewId: "retired-view" });
  });
  assert.equal(await wrongTargetInput.count(), 0, "a targeted recall must not fill another view of the same session");

  await clear.click();
  await respondAppConfirm(page);
  await rows.first().waitFor({ state: "detached" });
  await page.evaluate(() => {
    window.__emitTauriEvent("portmate-command-history-updated", structuredClone(window.__commandHistory));
    window.__failHistoryClearOnce = true;
    window.__rejectHistoryStartupRead(new Error("fixture startup read failure"));
  });
  const notice = page.locator(".notice-dialog", { hasText: "fixture history clear failure" });
  await notice.waitFor();
  assert.equal(await rows.count(), 0, "a delayed snapshot and startup fallback must respect a pending clear");
  assert.equal(await page.evaluate(() => localStorage.getItem("portmate.commandHistory")), null);
  await notice.getByRole("button").last().click();
  await clear.click();
  await respondAppConfirm(page);
  await page.waitForFunction(() => window.__commandHistory.entries.length === 0);
  assert.equal(await rows.count(), 0, "the retry should clear the backend without restoring old history");
  await page.close();
}
