import assert from "node:assert/strict";

export async function checkMcpHttpAddressSelection(context, appUrl) {
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const dialog = page.locator('.mcp-dialog');
  const selector = dialog.getByLabel('MCP HTTP 接入地址选择', { exact: true });
  const save = dialog.getByRole('button', { name: '保存配置', exact: true });
  const selectedOption = () => selector.locator('option:checked').textContent();
  const open = async (editable = true) => {
    await page.getByRole('button', { name: '工具', exact: true }).click();
    await page.getByRole('button', { name: 'MCP Bridge', exact: true }).click();
    await dialog.getByRole('tab', { name: 'HTTP', exact: true }).click();
    if (editable) await page.waitForFunction(() => !document.querySelector('[aria-label="MCP HTTP 接入地址选择"]').disabled);
    else await selector.waitFor();
  };
  const close = () => dialog.getByRole('button', { name: '关闭 MCP Bridge', exact: true }).click();
  const readConfigHost = () => page.evaluate(() => window.__mcpHttpConfig.clientHost);
  const waitSaved = () => page.waitForFunction(() => document.querySelector('[aria-label="MCP HTTP 接入地址选择"]')
    && !document.querySelector('[aria-label="MCP HTTP 接入地址选择"]').disabled
    && [...document.querySelectorAll('.mcp-http-network button')].find(button => button.textContent.includes('保存配置'))?.disabled);
  try {
    await page.goto(appUrl);
    await page.locator('.tree-session', { hasText: 'Edge Router' }).waitFor();
    await page.evaluate(() => { localStorage.removeItem('portmate.mcpHttpAddressSelection.v1'); window.confirm = () => true; });
    await open();
    assert((await selectedOption()).includes('Loopback'), 'default saved IP must use a real local interface');
    assert.equal(await dialog.getByLabel('MCP HTTP 客户端地址', { exact: true }).count(), 0, 'manual textbox is Custom-only');
    const chooseAddress = (id, family, address) => selector.selectOption(JSON.stringify([id, family, address]));
    await chooseAddress('wifi-guid', 'ipv4', '192.0.2.99');
    assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('portmate.mcpHttpAddressSelection.v1')).interfaceId), 'wifi-guid');
    assert.equal(await dialog.getByLabel('MCP HTTP 监听 IP', { exact: true }).inputValue(), '127.0.0.1', 'access selection must not change listener consent');
    await save.click(); await waitSaved();
    assert.equal(await readConfigHost(), '192.0.2.99');
    await close(); await open();
    assert((await selectedOption()).includes('Wi-Fi'), 'reopen must remember the adapter');

    // DHCP renumbering follows the adapter ID, not the old literal IP.
    await close();
    await page.evaluate(() => { window.__localNetworkAddresses = [
      { interfaceId: 'loopback', interfaceName: 'Loopback', address: '127.0.0.1', family: 'ipv4', loopback: true },
      { interfaceId: 'wifi-guid', interfaceName: 'Wi-Fi', address: '192.0.2.101', family: 'ipv4', loopback: false },
      { interfaceId: 'ethernet-guid', interfaceName: 'Ethernet', address: '2001:db8::42', family: 'ipv6', loopback: false },
    ]; });
    await open();
    assert((await selectedOption()).includes('192.0.2.101'));
    await page.waitForFunction(() => ![...document.querySelectorAll('.mcp-http-network button')].find(button => button.textContent.includes('保存配置')).disabled);
    assert.equal(await readConfigHost(), '192.0.2.99', 'adapter refresh only edits a draft');
    await save.click(); await waitSaved();
    assert.equal(await readConfigHost(), '192.0.2.101');

    // Refreshing a DHCP address while editing another field keeps the displayed
    // interface address and the saved draft in sync without losing that edit.
    await dialog.getByLabel('MCP HTTP 端口', { exact: true }).fill('8788');
    await page.evaluate(() => { window.__localNetworkAddresses = window.__localNetworkAddresses.map(entry =>
      entry.interfaceId === 'wifi-guid' ? { ...entry, address: '192.0.2.102' } : entry); });
    await dialog.getByRole('button', { name: '刷新本机网卡地址', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[aria-label="MCP HTTP 接入地址选择"]')?.selectedOptions[0]?.textContent.includes('192.0.2.102'));
    await save.click(); await waitSaved();
    assert.equal(await readConfigHost(), '192.0.2.102', 'dirty DHCP refresh must save the address actually shown');
    assert.equal(await dialog.getByLabel('MCP HTTP 端口', { exact: true }).inputValue(), '8788', 'other draft edits must survive address refresh');
    await dialog.getByLabel('MCP HTTP 端口', { exact: true }).fill('8787');
    await save.click(); await waitSaved();

    // Explicit discard stays discarded instead of immediately restoring preference.
    await selector.selectOption('custom');
    await dialog.getByLabel('MCP HTTP 客户端地址', { exact: true }).fill('discarded.example.test');
    await chooseAddress('ethernet-guid', 'ipv6', '2001:db8::42');
    await dialog.getByRole('button', { name: '放弃更改', exact: true }).click();
    await waitSaved();
    assert((await selectedOption()).includes('Wi-Fi'));
    await selector.selectOption('custom');
    assert.equal(await dialog.getByLabel('MCP HTTP 客户端地址', { exact: true }).inputValue(), '192.0.2.102', 'discard must also clear the unsaved Custom address');
    await dialog.getByRole('button', { name: '放弃更改', exact: true }).click();
    await waitSaved();
    await close();
    await page.evaluate(() => { window.__localNetworkAddresses = window.__localNetworkAddresses.filter(entry => entry.interfaceId !== 'wifi-guid'); });
    await open();
    assert((await selectedOption()).includes('不可用'), 'missing remembered adapter must not silently pick Ethernet');
    assert.equal(await selector.inputValue(), 'unavailable');
    assert.equal(await save.isDisabled(), true);

    await selector.selectOption('custom');
    const custom = dialog.getByLabel('MCP HTTP 客户端地址', { exact: true });
    await custom.fill('mcp.example.test'); await save.click(); await waitSaved();
    assert.equal(await readConfigHost(), 'mcp.example.test');
    await close(); await open();
    assert.equal(await selector.inputValue(), 'custom');
    assert.equal(await custom.inputValue(), 'mcp.example.test');

    await chooseAddress('ethernet-guid', 'ipv6', '2001:db8::42');
    await save.click(); await waitSaved();
    assert.equal(await readConfigHost(), '2001:db8::42');
    assert((await dialog.locator('.mcp-http-row').filter({ hasText: '接入 URL' }).textContent()).includes('http://[2001:db8::42]:8787/mcp'));

    // A refresh can discover a service configured/started by another window.
    // Its canonical address must supersede this already-initialized selection.
    await page.evaluate(() => {
      window.__mcpHttpConfig.clientHost = '127.0.0.1';
      window.__mcpHttpConfig.clientEndpoint = 'http://127.0.0.1:8787/mcp';
      window.__mcpHttpRuntime = { phase: 'running', endpoint: window.__mcpHttpConfig.endpoint, pid: 99, startedAt: new Date().toISOString(), message: null };
    });
    await dialog.getByRole('button', { name: '刷新 HTTP 状态', exact: true }).click();
    await dialog.locator('.mcp-http-runtime.running').waitFor();
    await page.waitForFunction(() => document.querySelector('[aria-label="MCP HTTP 接入地址选择"]')?.selectedOptions[0]?.textContent.includes('127.0.0.1'));
    assert.equal(await selector.isDisabled(), true);
    assert.equal(await readConfigHost(), '127.0.0.1');
    await close();
    await page.evaluate(() => {
      window.__mcpHttpConfig.clientHost = '2001:db8::42';
      window.__mcpHttpConfig.clientEndpoint = 'http://[2001:db8::42]:8787/mcp';
      localStorage.setItem('portmate.mcpHttpAddressSelection.v1', JSON.stringify({ mode: 'custom', address: 'remembered.example.test' }));
      window.__mcpHttpRuntime = { phase: 'running', endpoint: window.__mcpHttpConfig.endpoint, pid: 99, startedAt: new Date().toISOString(), message: null };
    });
    await open(false);
    await dialog.locator('.mcp-http-runtime.running').waitFor();
    await page.waitForFunction(() => [...document.querySelector('[aria-label="MCP HTTP 接入地址选择"]').options].some(option => option.textContent.includes('2001:db8::42')));
    assert((await selectedOption()).includes('2001:db8::42'), 'stored preference must not replace a running service address');
    assert.equal(await selector.isDisabled(), true);
    assert.equal(await readConfigHost(), '2001:db8::42');
    assert.deepEqual(errors, []);
  } finally { await page.close(); }
}
