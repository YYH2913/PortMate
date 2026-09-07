import assert from "node:assert/strict";

export async function checkPacedSender(context, appUrl) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.goto(appUrl);
    await page.waitForSelector('.terminal-host[data-terminal-size] .xterm-screen');
    if (!await page.locator('.send-toolbar').count()) {
      await page.locator('.menu-trigger', { hasText: '工作区' }).click();
      await page.locator('.menu-popover').getByRole('button', { name: '发送', exact: true }).click();
    }
    const sender = page.locator('.send-toolbar').locator('..');
    const start = page.getByRole('button', { name: '发送', exact: true });
    const stop = page.getByRole('button', { name: '停止重复发送', exact: true });
    await page.getByRole('button', { name: '高级发送选项', exact: true }).click();
    const count = page.getByRole('spinbutton', { name: '发送次数', exact: true });
    const interval = page.getByRole('spinbutton', { name: '发送间隔（毫秒）', exact: true });
    const text = page.getByRole('textbox', { name: 'send text', exact: true });
    const dismiss = async () => {
      const notice = page.locator('.notice-dialog');
      await notice.waitFor();
      await notice.getByRole('button', { name: '确定', exact: true }).click();
    };

    await sender.getByRole('radio', { name: 'Hex(H)' }).check();
    await text.fill('01 GG 02');
    await start.click();
    await page.locator('.notice-dialog', { hasText: 'Hex 格式无效' }).waitFor();
    assert.equal(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === 'begin_paced_send').length), 0);
    await dismiss();
    await sender.getByRole('radio', { name: '文本(T)' }).check();

    // Stop a first payload that is behind earlier keyboard IPC, before it
    // reaches native send_text. No repeated payload should escape later.
    await text.fill('cancelled-before-ipc');
    await count.fill('3');
    await interval.fill('100');
    await page.evaluate(() => { window.__deferTerminalSends = true; window.__pendingTerminalSends = []; });
    await page.locator('.terminal-pane.active .xterm-helper-textarea').focus();
    await page.keyboard.type('q');
    await page.waitForFunction(() => window.__pendingTerminalSends.length === 1);
    await start.click();
    await stop.click();
    await dismiss();
    await page.evaluate(() => {
      window.__deferTerminalSends = false;
      for (const pending of window.__pendingTerminalSends.splice(0)) pending.resolve(null);
    });
    assert.equal(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === 'send_text' && c.args.text === 'cancelled-before-ipc').length), 0);

    // Delay the first actual write longer than the configured interval.
    // The next write must still wait a full interval after it completes.
    await text.fill('paced-wire');
    await count.fill('2');
    await interval.fill('200');
    await page.evaluate(() => { window.__deferTerminalSends = true; window.__pacedWrites = []; });
    await start.click();
    await page.waitForFunction(() => window.__pendingTerminalSends.length === 1);
    await page.waitForTimeout(300);
    await page.evaluate(() => {
      window.__deferTerminalSends = false;
      window.__pendingTerminalSends.shift().resolve(null);
    });
    await start.waitFor();
    const times = await page.evaluate(() => window.__pacedWrites.filter(w => w.text === 'paced-wire').map(w => w.time));
    assert.equal(times.length, 2);
    assert(times[1] - times[0] >= 195, `write gap collapsed: ${times}`);

    // Synchronized keyboard input must not sit behind interval timers.
    await page.locator('.menu-trigger', { hasText: '终端' }).click();
    await page.locator('.menu-popover').getByRole('button', { name: '同步输入', exact: true }).click();
    await text.fill('repeat-with-sync');
    await count.fill('3');
    await interval.fill('10000');
    await start.click();
    await page.waitForFunction(() => window.__pacedWrites.some(w => w.text === 'repeat-with-sync'));
    const beforeKey = await page.evaluate(() => window.__invokeCalls.length);
    await page.locator('.terminal-pane.active .xterm-helper-textarea').focus();
    await page.keyboard.press('Control+c');
    await page.waitForFunction(start => window.__invokeCalls.slice(start).some(c => c.command === 'send_text' && c.args.text === '\x03'), beforeKey);
    assert.equal(await page.evaluate(() => window.__pacedWrites.filter(w => w.text === 'repeat-with-sync').length), 1);
    await stop.click();
    await dismiss();

    // Both text and Hex jobs must retire immediately on a connection change.
    for (const mode of ['text', 'hex']) {
      await sender.getByRole('radio', { name: mode === 'text' ? '文本(T)' : 'Hex(H)' }).check();
      await text.fill(mode === 'text' ? 'no-replay-after-disconnect' : '01 02 FF');
      await page.evaluate(() => { window.__pacedWrites = []; });
      await start.click();
      await page.waitForFunction(() => window.__pacedWrites.length === 1);
      await page.evaluate(() => {
        const id = window.__pacedWrites[0].sessionId;
        const session = window.__sessions.find(s => s.profile.id === id);
        session.runtime.status = 'disconnected';
        window.__emitTauriEvent('portmate-session-profile-updated', structuredClone(session));
      });
      // Force a summaries refresh through the same window focus recovery path.
      await page.evaluate(() => window.dispatchEvent(new Event('focus')));
      await stop.waitFor({ state: 'detached', timeout: 5000 });
      await dismiss();
      assert.equal(await page.evaluate(() => window.__pacedWrites.length), 1);
      await page.evaluate(() => {
        for (const session of window.__sessions) {
          session.runtime.status = 'connected';
          window.__emitTauriEvent('portmate-session-profile-updated', structuredClone(session));
        }
        window.dispatchEvent(new Event('focus'));
      });
      await page.waitForTimeout(300);
    }
    assert.deepEqual(errors, [], `paced sender browser errors: ${errors}`);
  } finally {
    await page.close();
  }
}
