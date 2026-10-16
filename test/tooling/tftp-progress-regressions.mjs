import assert from "node:assert/strict";

export async function checkTftpProgress(context, appUrl) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.goto(appUrl);
    await page.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
    await page.waitForFunction(() => window.__tauriEventListeners.get("portmate-transfer-task")?.length > 0);
    await page.getByRole("button", { name: "工具", exact: true }).click();
    await page.getByRole("button", { name: "传输任务", exact: true }).click();
    const dialog = page.locator(".transfer-dialog");
    await dialog.waitFor();
    const source = "/tftp-progress-regression.bin";
    const total = 1468 * 64 + 11;
    const row = dialog.locator(".transfer-row", { hasText: source });
    let priorText = "";
    for (const done of [1468, 1468 * 22, 1468 * 44, total]) {
      await page.evaluate(({ source, done, total }) => {
        const task = {
          id: "tftp-live-progress", sessionId: "edge-router", protocol: "tftp",
          source, destination: "load:tftpboot", bytesTotal: total, bytesDone: done,
          status: done === total ? "completed" : "running",
          message: done === total ? "completed" : "running",
          startedAt: new Date(Date.now() - 2000).toISOString(),
          finishedAt: done === total ? new Date().toISOString() : null,
        };
        window.__transfers = [task];
        window.__emitTauriEvent("portmate-transfer-task", task);
      }, { source, done, total });
      await row.waitFor();
      await page.waitForFunction(({ done, total }) => {
        const width = document.querySelector('.transfer-dialog .transfer-row .transfer-progress span')?.style.width;
        return width && Math.abs(Number.parseFloat(width) - done / total * 100) < 0.001;
      }, { done, total });
      const currentText = await row.locator("small").last().textContent();
      assert.notEqual(currentText, priorText, "live byte counts must render before the transfer finishes");
      priorText = currentText;
      assert.equal(await row.getByRole("button", { name: "取消", exact: true }).count(), done === total ? 0 : 1);
    }
    assert.equal(await row.locator(".transfer-progress span").evaluate(element => element.style.width), "100%");
    assert.deepEqual(errors, []);
  } finally { await page.close(); }
}
