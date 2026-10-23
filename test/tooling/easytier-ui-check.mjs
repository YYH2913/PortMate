import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { chromium } from "playwright-core";
import { createServer } from "vite";

const server = await createServer({ server: { host: "127.0.0.1", port: 0 } });
let browser;
try {
  await server.listen();
  const url = `http://127.0.0.1:${server.httpServer.address().port}/test/fixtures/easytier/ui.html`;
  browser = await chromium.launch({ executablePath: process.env.PORTMATE_CHROME || "/usr/bin/google-chrome", headless: true, args: ["--no-sandbox"] });
  async function pageFor(mode = "desktop", viewport = { width: 1200, height: 850 }, locale = "en") {
    const page = await browser.newPage({ viewport });
    await page.addInitScript(mode => {
      if (mode === "browser") return;
      const config = { networkName: "PortMate", hostname: "", addressMode: "dhcp", virtualIpv4: "", peers: [], listeners: ["tcp://0.0.0.0:11010", "udp://0.0.0.0:11010"], latencyFirst: false, mtu: 1360 };
      window.calls = [];
      window.network = { config, componentReady: true, componentVersion: "2.6.4", status: { phase: "idle", pid: null, virtualIpv4: "", hostname: "", peers: [], logs: [], error: null } };
      window.__TAURI_INTERNALS__ = {
        transformCallback: () => 1, unregisterCallback() {},
        async invoke(command, args) {
          window.calls.push(command);
          if (command === "easy_tier_status") return structuredClone(window.network);
          if (command === "save_easy_tier_config") {
            if ("networkSecret" in args.config) throw Error("secret persisted in configuration");
            window.network.config = args.config;
            return structuredClone(window.network);
          }
          if (command === "start_easy_tier") {
            window.lastStart = args;
            if (window.failStart) throw { code: "component-missing", detail: "missing" };
            await new Promise(resolve => { window.startFinished = resolve; });
            window.network.config = args.config;
            window.network.status.phase = "starting";
            return structuredClone(window.network);
          }
          if (command === "stop_easy_tier") {
            window.network.status = { ...window.network.status, phase: "idle", pid: null, peers: [], virtualIpv4: "" };
            return structuredClone(window.network);
          }
          throw Error(`Unexpected command ${command}`);
        },
      };
    }, mode);
    await page.goto(`${url}?lang=${locale}`);
    return page;
  }
  const page = await pageFor();
  const dialog = page.getByRole("dialog", { name: "EasyTier networking" });
  const connect = dialog.getByRole("button", { name: "Connect network", exact: true });
  await dialog.getByText("Disconnected", { exact: true }).waitFor();
  assert(await connect.isDisabled());
  await dialog.getByRole("button", { name: "Public server", exact: true }).click();
  assert.equal(await dialog.getByLabel("Peer addresses", { exact: true }).inputValue(), "tcp://public.easytier.top:11010");
  await dialog.getByRole("button", { name: "Manual connection", exact: true }).click();
  await dialog.getByLabel("Network name", { exact: true }).fill("Private lab");
  await dialog.getByLabel("Network secret", { exact: true }).fill("test-only-password");
  await dialog.getByRole("button", { name: "Show secret", exact: true }).click();
  assert.equal(await dialog.getByLabel("Network secret", { exact: true }).getAttribute("type"), "text");
  await dialog.getByRole("button", { name: "Hide secret", exact: true }).click();
  await dialog.getByLabel("Peer addresses 1", { exact: true }).fill("udp://192.168.1.20:11010");
  await dialog.getByRole("button", { name: "Add address", exact: true }).click();
  await dialog.getByLabel("Peer addresses 2", { exact: true }).fill("tcp://192.168.1.21:11010");
  await dialog.getByRole("button", { name: "Remove address 2", exact: true }).click();
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await page.waitForFunction(() => window.network.config.networkName === "Private lab");
  assert.equal(await page.evaluate(() => JSON.stringify(window.network.config).includes("test-only-password")), false);
  await connect.dblclick();
  await page.waitForFunction(() => window.lastStart);
  assert.equal(await page.evaluate(() => window.calls.filter(command => command === "start_easy_tier").length), 1);
  assert.equal(await page.evaluate(() => window.lastStart.networkSecret), "test-only-password");
  await page.keyboard.press("Escape");
  assert.equal(await page.evaluate(() => document.body.dataset.closed), undefined, "busy start must not be dismissed");
  await page.evaluate(() => window.startFinished());
  await dialog.getByRole("button", { name: "Disconnect network" }).waitFor();
  assert.equal(await dialog.getByRole("tab", { name: "2 Running status" }).getAttribute("aria-selected"), "true");
  await dialog.getByRole("tab", { name: "1 Configure network" }).click();
  assert.equal(await dialog.getByLabel("Network secret", { exact: true }).inputValue(), "");
  assert(await dialog.getByLabel("Network name", { exact: true }).isDisabled());
  await dialog.getByRole("tab", { name: "2 Running status" }).click();
  await page.evaluate(() => {
    window.network.status = { ...window.network.status, phase: "running", virtualIpv4: "10.10.10.2", hostname: "local-device", peers: [{ id: "12", hostname: "Router", ipv4: "10.10.10.3", route: "p2p", latency: "2.10", received: "24.1 KiB", sent: "8.2 KiB", protocol: "udp" }], logs: ["Peer connected", "[network credentials omitted]"] };
  });
  await dialog.getByRole("button", { name: "Refresh", exact: true }).click();
  await dialog.getByText("Connected · 1 peer(s)", { exact: true }).waitFor();
  await dialog.getByText("Router", { exact: true }).waitFor();
  await mkdir(resolve("tmp/easytier-review"), { recursive: true });
  await page.screenshot({ path: resolve("tmp/easytier-review/network-connected.png") });
  await dialog.getByRole("button", { name: "Disconnect network" }).click();
  await dialog.getByText("Disconnected", { exact: true }).waitFor();
  await dialog.getByRole("button", { name: "Settings", exact: true }).click();
  await dialog.getByText("EasyTier 2.6.4 is built into PortMate.", { exact: true }).waitFor();
  assert.equal(await dialog.getByRole("button", { name: /download|install component/i }).count(), 0);
  assert.equal(await dialog.getByRole("textbox").count(), 0, "built-in networking must not require an external program directory");
  await page.evaluate(() => { window.failStart = true; });
  await dialog.getByRole("tab", { name: "1 Configure network" }).click();
  await dialog.getByLabel("Network secret", { exact: true }).fill("retry-secret");
  await connect.click();
  await dialog.getByRole("alert").getByText(/bundled EasyTier core is unavailable/).waitFor();
  await dialog.getByRole("tab", { name: "2 Running status" }).click();
  await dialog.getByRole("button", { name: "Refresh", exact: true }).click();
  assert(await dialog.getByRole("alert").isVisible(), "a periodic refresh must not erase action errors");
  await dialog.getByRole("tab", { name: "1 Configure network" }).click();
  await dialog.locator("summary").click();
  await dialog.getByLabel("Address mode", { exact: true }).selectOption("static");
  await dialog.getByLabel("Virtual IPv4", { exact: true }).fill("127.0.0.1");
  assert(await connect.isDisabled());
  await dialog.getByLabel("Address mode", { exact: true }).selectOption("relay");
  await dialog.getByText(/Local applications cannot connect through a virtual IP/).waitFor();
  await page.close();

  for (const viewport of [{ width: 560, height: 620 }, { width: 1100, height: 540 }]) {
    const compact = await pageFor("desktop", viewport);
    await compact.getByRole("dialog").waitFor();
    assert(await compact.getByRole("dialog").evaluate(element => {
      const rect = element.getBoundingClientRect();
      return rect.top >= 0 && rect.bottom <= innerHeight && element.scrollWidth <= element.clientWidth;
    }), "the dialog must fit and scroll within compact viewports");
    assert(await compact.locator(".easytier-actions").evaluate(element => {
      const rect = element.getBoundingClientRect();
      return rect.top >= 0 && rect.bottom <= innerHeight;
    }), "network actions must remain visible without scrolling");
    await compact.screenshot({ path: resolve(`tmp/easytier-review/network-${viewport.width}.png`) });
    await compact.close();
  }
  const chinese = await pageFor("desktop", { width: 1200, height: 850 }, "zh");
  await chinese.getByRole("button", { name: "手动连接", exact: true }).click();
  await chinese.getByLabel("网络名称", { exact: true }).fill("PortMate Lab");
  await chinese.getByLabel("对等节点地址 1", { exact: true }).fill("tcp://192.168.1.20:11010");
  await chinese.screenshot({ path: resolve("tmp/easytier-review/network-config-zh.png") });
  await chinese.evaluate(() => {
    window.network.config.networkName = "PortMate Lab";
    window.network.componentReady = true;
    window.network.status = { phase: "running", virtualIpv4: "10.144.144.2", hostname: "PortMate-PC", peers: [{ id: "12", hostname: "Dragino", ipv4: "10.144.144.3", route: "p2p", latency: "1.42", received: "24.1 KiB", sent: "8.2 KiB", protocol: "udp" }], logs: [] };
  });
  await chinese.getByRole("tab", { name: "2 运行状态" }).click();
  await chinese.getByRole("button", { name: "刷新", exact: true }).click();
  await chinese.getByText("Dragino", { exact: true }).waitFor();
  await chinese.screenshot({ path: resolve("tmp/easytier-review/network-running-zh.png") });
  await chinese.getByRole("tab", { name: "2 运行状态" }).focus();
  await chinese.keyboard.press("Home");
  assert.equal(await chinese.getByRole("tab", { name: "1 配置网络" }).getAttribute("aria-selected"), "true");
  await chinese.getByRole("button", { name: "设置", exact: true }).click();
  await chinese.screenshot({ path: resolve("tmp/easytier-review/network-settings-zh.png") });
  await chinese.close();
  const preview = await pageFor("browser");
  await preview.getByText("EasyTier networking is available in the desktop application.", { exact: true }).waitFor();
  assert(await preview.getByRole("button", { name: "Connect network", exact: true }).isDisabled());
  await preview.close();
  console.log("EasyTier UI passed: built-in core with no download/directory setup, save without secrets, duplicate start guard, secret clearing, peer presentation, disconnect, persistent errors, address validation, compact layouts and browser fallback.");
} finally {
  await browser?.close();
  await server.close();
}
