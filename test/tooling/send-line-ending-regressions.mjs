import assert from "node:assert/strict";

export async function checkSendLineEndings(page, sender, start, text, count, interval, dismiss) {
  const ending = sender.getByRole('combobox', { name: '追加换行符', exact: true });
  const target = sender.getByRole('combobox', { name: '发送目标', exact: true });
  assert.equal(await ending.inputValue(), 'none', 'raw text must remain the default');
  assert.equal(await ending.isEnabled(), true);
  if (process.env.PORTMATE_SEND_LINE_ENDING_SCREENSHOT) {
    await page.screenshot({ path: process.env.PORTMATE_SEND_LINE_ENDING_SCREENSHOT });
  }
  await count.fill('1');
  await interval.fill('0');
  const draft = 'first\n中文😀\n';

  // Every send appends to the original draft, never to a previous payload.
  for (const [value, suffix] of [['none', ''], ['lf', '\n'], ['cr', '\r'], ['crlf', '\r\n']]) {
    await ending.selectOption(value);
    await text.fill(draft);
    await page.evaluate(() => { window.__pacedWrites = []; });
    if (value === 'lf') await text.press('Control+Enter');
    else await start.click();
    await page.waitForFunction(() => window.__pacedWrites.length === 1);
    await start.waitFor();
    assert.equal(await page.evaluate(() => window.__pacedWrites[0].text), draft + suffix);
    assert.equal(await text.inputValue(), draft, 'sending must not change the draft');
  }

  // No payload in None mode; the other choices can submit an empty command.
  await text.fill('');
  await ending.selectOption('none');
  const emptyStart = await page.evaluate(() => window.__invokeCalls.filter(c => c.command === 'begin_paced_send').length);
  await start.click();
  assert.equal(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === 'begin_paced_send').length), emptyStart);
  for (const [value, suffix] of [['lf', '\n'], ['cr', '\r'], ['crlf', '\r\n']]) {
    await ending.selectOption(value);
    await page.evaluate(() => { window.__pacedWrites = []; });
    await start.click();
    await page.waitForFunction(() => window.__pacedWrites.length === 1);
    await start.waitFor();
    assert.equal(await page.evaluate(() => window.__pacedWrites[0].text), suffix);
    assert.equal(await text.inputValue(), '');
  }

  // The bound applies to the final UTF-8 payload, including the suffix.
  await text.fill('a'.repeat(4 * 1024 * 1024 - 1));
  const oversizedStart = await page.evaluate(() => window.__invokeCalls.filter(c => c.command === 'begin_paced_send').length);
  await start.click();
  await page.locator('.notice-dialog', { hasText: '单次发送内容不能超过 4 MiB' }).waitFor();
  assert.equal(await page.evaluate(() => window.__invokeCalls.filter(c => c.command === 'begin_paced_send').length), oversizedStart);
  await dismiss();

  // A selected text suffix must not alter Hex frames.
  await sender.getByRole('radio', { name: 'Hex(H)' }).check();
  assert.equal(await ending.isDisabled(), true);
  await text.fill('41 0D 0A 00 FF');
  await page.evaluate(() => { window.__pacedWrites = []; });
  await start.click();
  await page.waitForFunction(() => window.__pacedWrites.length === 1);
  await start.waitFor();
  assert.deepEqual(await page.evaluate(() => window.__pacedWrites[0].bytes), [65, 13, 10, 0, 255]);
  await sender.getByRole('radio', { name: '文本(T)' }).check();
  assert.equal(await ending.inputValue(), 'crlf', 'Hex mode must retain the text selection');

  const keyboardStart = await page.evaluate(() => window.__invokeCalls.length);
  await page.locator('.terminal-pane.active .xterm-helper-textarea').focus();
  await page.keyboard.press('Control+c');
  await page.waitForFunction(start => window.__invokeCalls.slice(start).some(c => c.command === 'send_text'), keyboardStart);
  assert.deepEqual(await page.evaluate(start => window.__invokeCalls.slice(start)
    .filter(c => c.command === 'send_text').map(c => c.args.text), keyboardStart), ['\x03']);

  // Freeze the draft and ending before waiting for any transport write.
  await text.fill('snapshot');
  await target.selectOption('connected');
  await count.fill('2');
  const connectedIds = await page.evaluate(() => window.__sessions.filter(s => s.runtime.status === 'connected').map(s => s.profile.id));
  await page.evaluate(() => {
    window.__deferTerminalSends = true;
    window.__pendingTerminalSends = [];
    window.__pacedWrites = [];
  });
  await start.click();
  await page.waitForFunction(total => window.__pendingTerminalSends.length === total, connectedIds.length);
  assert.equal(await ending.isDisabled(), true, 'ending selection must be locked during sending');
  await text.fill('next-draft');
  await sender.getByRole('radio', { name: 'Hex(H)' }).check();
  await page.evaluate(() => {
    window.__deferTerminalSends = false;
    for (const pending of window.__pendingTerminalSends.splice(0)) pending.resolve(null);
  });
  await start.waitFor();
  const writes = await page.evaluate(() => window.__pacedWrites.map(w => ({ sessionId: w.sessionId, text: w.text })));
  assert.equal(writes.length, connectedIds.length * 2);
  for (const id of connectedIds) {
    assert.deepEqual(writes.filter(w => w.sessionId === id).map(w => w.text), ['snapshot\r\n', 'snapshot\r\n']);
  }
  assert.equal(await text.inputValue(), 'next-draft');
  await sender.getByRole('radio', { name: '文本(T)' }).check();
  await target.selectOption('active');
  await count.fill('1');

  // A failed write retains the unmodified draft and unlocks the selector.
  await text.fill('retained-on-failure');
  await page.evaluate(() => { window.__deferTerminalSends = true; });
  await start.click();
  await page.waitForFunction(() => window.__pendingTerminalSends.length === 1);
  await page.evaluate(() => {
    window.__deferTerminalSends = false;
    window.__pendingTerminalSends.shift().reject(new Error('line-ending-write-failed'));
  });
  await page.locator('.notice-dialog', { hasText: 'line-ending-write-failed' }).waitFor();
  assert.equal(await text.inputValue(), 'retained-on-failure');
  assert.equal(await ending.isEnabled(), true);
  await dismiss();

  const language = page.locator('.menu-primary .language-selector select');
  const originalLanguage = await language.inputValue();
  const toolbar = sender.locator('.send-toolbar');
  try {
    // A 200px dock has only 180px of inner toolbar space. Long labels and
    // RTL text must wrap instead of hiding the new control outside the dock.
    for (const width of [200, 320]) {
      await toolbar.evaluate((element, width) => { element.style.width = `${width}px`; }, width);
      for (const locale of ['en', 'fr', 'ru', 'es', 'ar', 'zh']) {
        await language.selectOption(locale);
        await page.waitForFunction(locale => document.documentElement.lang === (locale === 'zh' ? 'zh-CN' : locale), locale);
        const geometry = await toolbar.evaluate(element => {
          const bounds = element.getBoundingClientRect();
          return [...element.querySelectorAll('.send-line-ending-control > *, .send-toolbar-primary > *')].map(control => {
            const rect = control.getBoundingClientRect();
            return { inside: rect.left >= bounds.left && rect.right <= bounds.right,
              width: rect.width, height: rect.height };
          });
        });
        assert(geometry.every(control => control.inside && control.width > 0 && control.height > 0),
          `${locale} send controls overflow at ${width}px: ${JSON.stringify(geometry)}`);
        assert.equal(await sender.locator('.send-line-ending-input').inputValue(), 'crlf', 'locale changes must not alter the ending');
      }
    }
  } finally {
    await toolbar.evaluate(element => { element.style.removeProperty('width'); });
    await language.selectOption(originalLanguage);
  }
  await ending.selectOption('none');
  await interval.fill('1000');
}
