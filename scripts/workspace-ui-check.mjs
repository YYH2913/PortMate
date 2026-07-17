import { spawn } from "node:child_process";
import { createServer } from "node:net";
import process from "node:process";
import { chromium } from "playwright-core";

const chromeExecutable = process.env.PORTMATE_CHROME ?? "/usr/bin/google-chrome";
const screenshotPrefix = process.env.PORTMATE_WORKSPACE_UI_SCREENSHOT_PREFIX
  ?? "/tmp/portmate-workspace-ui";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function reservePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : 0;
  await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  if (!port) throw new Error("failed to reserve a Vite port");
  return port;
}

async function waitForServer(url, output) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // Vite is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`timed out waiting for ${url}\n${output()}`);
}

const recordedAt = Date.now();
const isoNow = new Date(recordedAt).toISOString();

function createSession(id, name, kind, group, tags, connection) {
  return {
    profile: {
      id,
      name,
      kind,
      group,
      tags,
      connection,
      terminal: {
        term: "xterm-256color",
        rows: 28,
        cols: 100,
        scrollback: 200000,
        fontFamily: "JetBrains Mono, monospace",
        fontSize: 13,
        theme: "portmate-dark",
      },
      logging: {
        enabled: false,
        raw: false,
        text: false,
        jsonl: false,
        redactSecrets: true,
        pathTemplate: "{profile}/{date}/{session}.jsonl",
        retentionDays: 0,
      },
      triggers: [],
      transfer: {
        sftp: false,
        scp: false,
        xmodem: true,
        ymodem: true,
        zmodem: true,
        rateLimitBytesPerSecond: null,
        defaultLocalDir: null,
      },
    },
    runtime: {
      sessionId: id,
      paneId: `${id}:main`,
      status: "connected",
      title: name,
      cwd: null,
      connectedSince: isoNow,
      lastActivity: isoNow,
      lastDisconnect: null,
      lastDisconnectReason: null,
      activeTransport: kind,
    },
    logLines: 1,
    lastLine: `${name} ready`,
  };
}

const sessions = [
  createSession("edge-router", "Edge Router", "ssh", "Network", ["production", "gateway"], {
    kind: "ssh",
    endpoint: { host: "10.0.0.1", port: 2222 },
    username: "admin",
  }),
  createSession("bench-uart", "Bench UART", "serial", "Lab", ["hardware"], {
    kind: "serial",
    port: "/dev/ttyUSB0",
    baudRate: 115200,
  }),
  createSession("local-shell", "Local Shell", "shell", "Local", ["development"], {
    kind: "shell",
    program: "/bin/zsh",
    args: ["-l"],
    cwd: "/workspace",
    env: {},
  }),
];

const events = sessions.map((session) => ({
  id: `event-${session.profile.id}`,
  sessionId: session.profile.id,
  paneId: `${session.profile.id}:main`,
  ts: isoNow,
  direction: "inbound",
  stream: "stdout",
  bytesRef: null,
  text: `${session.profile.name}\r\n$ `,
  annotations: {},
}));

const workspace = {
  version: 4,
  root: {
    kind: "pane",
    id: "pane-a",
    activeViewId: "view-edge",
    views: [{ id: "view-edge", sessionId: "edge-router", title: "Edge", color: "", keyMode: "remote" }],
  },
  activePaneId: "pane-a",
  activeId: "edge-router",
  tabColors: { "edge-router": "#008080", "bench-uart": "#DAA520" },
};

const port = await reservePort();
const appUrl = `http://127.0.0.1:${port}/`;
let viteOutput = "";
const vite = spawn(process.execPath, [
  "node_modules/vite/bin/vite.js",
  "--host", "127.0.0.1",
  "--port", String(port),
  "--strictPort",
], { cwd: process.cwd(), stdio: ["ignore", "pipe", "pipe"] });
vite.stdout.on("data", (chunk) => { viteOutput += chunk.toString(); });
vite.stderr.on("data", (chunk) => { viteOutput += chunk.toString(); });

let browser;
try {
  await waitForServer(appUrl, () => viteOutput);
  browser = await chromium.launch({
    executablePath: chromeExecutable,
    headless: true,
    args: ["--no-sandbox", "--enable-unsafe-swiftshader"],
  });
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  await context.addInitScript(({ initialSessions, initialEvents, initialWorkspace, historyTimestamp }) => {
    localStorage.clear();
    localStorage.setItem("portmate.workspace.v1", JSON.stringify(initialWorkspace));
    localStorage.setItem("portmate.workspacePanels.v1", JSON.stringify({
      version: 1,
      panels: {
        explorer: true,
        fileManager: true,
        sessions: true,
        history: true,
        sender: true,
        statusBar: true,
      },
    }));
    localStorage.setItem("portmate.commandHistory", JSON.stringify({
      version: 2,
      entries: [
        { command: "git status --short", recordedAt: historyTimestamp },
        { command: "docker compose\nup -d", recordedAt: historyTimestamp - 1 },
      ],
    }));
    localStorage.setItem("portmate.terminalPrefs", JSON.stringify({
      historyEnabled: true,
      historyLimit: "100",
      historyRetentionDays: "30",
      startupMode: "none",
      startupSessions: [],
      lockOnIdle: false,
      requireMasterPassword: false,
      oneKeyCompletionEnabled: false,
    }));
    window.__invokeCalls = [];
    window.__clipboardText = "";
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        readText: async () => window.__clipboardText,
        writeText: async (value) => { window.__clipboardText = String(value); },
      },
    });
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = {
      invoke: async (command, args = {}) => {
        window.__invokeCalls.push({ command, args });
        if (command === "list_sessions") return initialSessions;
        if (command === "tail_log") return initialEvents.filter((event) => event.sessionId === args.sessionId);
        if (command === "list_host_keys") return { keys: [] };
        if ([
          "list_files",
          "list_transfers",
          "list_mcp_audit",
          "list_mcp_grants",
          "list_serial_ports",
          "list_one_keys",
        ].includes(command)) return [];
        if (command.startsWith("plugin:event|")) return 1;
        return null;
      },
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      transformCallback: () => 1,
      unregisterCallback: () => {},
      convertFileSrc: (path) => path,
    };
  }, {
    initialSessions: sessions,
    initialEvents: events,
    initialWorkspace: workspace,
    historyTimestamp: recordedAt,
  });

  const page = await context.newPage();
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(appUrl);
  await page.waitForSelector('.terminal-host[data-terminal-size] .xterm-screen');
  await page.waitForFunction(() => localStorage.getItem("portmate.workspacePanels.v2") !== null);
  await page.getByRole("textbox", { name: "筛选资源管理器会话", exact: true }).waitFor();

  const initial = await page.evaluate(() => {
    const panels = JSON.parse(localStorage.getItem("portmate.workspacePanels.v2"));
    const left = document.querySelector(".left-stack")?.getBoundingClientRect();
    return {
      viewportWidth: innerWidth,
      documentWidth: document.documentElement.scrollWidth,
      leftWidth: left?.width ?? 0,
      globalTabs: document.querySelectorAll(".tab-line").length,
      rightDock: document.querySelectorAll(".right-stack").length,
      sender: document.querySelectorAll(".send-panel").length,
      fileManager: [...document.querySelectorAll(".dock-panel strong")]
        .some((node) => node.textContent === "文件管理器"),
      status: document.querySelector(".status-bar")?.textContent ?? "",
      panels: panels.panels,
    };
  });
  assert(initial.documentWidth <= initial.viewportWidth, `initial workspace overflow: ${JSON.stringify(initial)}`);
  assert(initial.globalTabs === 0, "redundant global session tabs are visible");
  assert(!initial.fileManager && initial.rightDock === 0 && initial.sender === 0,
    `legacy all-visible default did not migrate: ${JSON.stringify(initial)}`);
  assert(initial.leftWidth >= 275 && initial.leftWidth <= 285,
    `resource tree is not using the compact width: ${initial.leftWidth}`);
  assert(!initial.status.includes("窗口 -1×-1") && !initial.status.includes("PortMate Issues"),
    `placeholder status text survived: ${initial.status}`);
  assert(initial.panels.explorer && initial.panels.statusBar
    && !initial.panels.fileManager && !initial.panels.sessions
    && !initial.panels.history && !initial.panels.sender,
  `migrated v2 panel snapshot is wrong: ${JSON.stringify(initial.panels)}`);

  const explorerFilter = page.getByRole("textbox", { name: "筛选资源管理器会话", exact: true });
  await explorerFilter.fill("production");
  assert(await page.locator(".left-stack .tree-session").count() === 1,
    "resource tag filter did not remove unrelated sessions");
  assert(await page.locator(".left-stack .tree-session", { hasText: "Edge Router" }).count() === 1,
    "resource tag filter missed Edge Router");
  await explorerFilter.fill("10.0.0.1");
  assert(await page.locator(".left-stack .tree-session", { hasText: "Edge Router" }).count() === 1,
    "resource endpoint filter missed Edge Router");
  await page.getByRole("button", { name: "清除筛选资源管理器会话", exact: true }).click();
  assert(await page.locator(".left-stack .tree-session").count() === sessions.length,
    "clearing the resource filter did not restore all sessions");
  assert(await page.locator(".left-stack .tree-folder svg").count() === 3,
    "resource group headings contain non-semantic controls");

  const edge = page.locator(".left-stack .tree-session", { hasText: "Edge Router" });
  await edge.click({ button: "right" });
  const firstMenuText = await page.locator(".portmate-context-menu").textContent();
  assert(!firstMenuText.includes("复制SSH通道")
    && !firstMenuText.includes("拆分为")
    && !firstMenuText.includes("选择分组"),
  `redundant context actions survived: ${firstMenuText}`);
  assert(await page.locator(".portmate-context-menu .context-label", { hasText: /^水平拆分视图\(H\)$/ }).count() === 1,
    "horizontal split appears more than once");
  assert(await page.locator(".portmate-context-menu .context-label", { hasText: /^垂直拆分视图\(V\)$/ }).count() === 1,
    "vertical split appears more than once");
  await page.locator(".context-menu-row", { hasText: "开启同步输入(S)" }).click();
  await page.locator(".sync-status.active").waitFor();
  await edge.click({ button: "right" });
  assert(await page.locator(".context-menu-row", { hasText: "关闭同步输入(S)" }).count() === 1,
    "enabled synchronized input did not expose the inverse action");
  await page.locator(".context-menu-row", { hasText: "关闭同步输入(S)" }).click();
  await page.waitForFunction(() => !document.querySelector(".sync-status")?.classList.contains("active"));
  await edge.click({ button: "right" });
  await page.locator(".context-menu-row", { hasText: "复制会话名称(N)" }).click();
  assert(await page.evaluate(() => window.__clipboardText) === "Edge Router",
    "resource context menu targeted a different session");

  async function togglePanel(label) {
    await page.locator(".menu-trigger", { hasText: "查看" }).click();
    await page.locator(".menu-popover button", { hasText: label }).click();
  }

  await togglePanel("会话");
  await togglePanel("历史命令");
  await togglePanel("发送");
  const sessionFilter = page.getByRole("textbox", { name: "筛选会话列表", exact: true });
  await sessionFilter.fill("ttyUSB0");
  const uart = page.locator(".right-stack .tree-session", { hasText: "Bench UART" });
  assert(await uart.count() === 1, "session endpoint filter missed Bench UART");
  await uart.click();
  await page.waitForFunction(() => document.querySelector(".right-stack .tree-session.active")?.textContent?.includes("Bench UART"));
  await sessionFilter.fill("gateway");
  const filteredEdge = page.locator(".right-stack .tree-session", { hasText: "Edge Router" });
  assert(await filteredEdge.count() === 1, "session tag filter missed Edge Router");
  await filteredEdge.click({ button: "right" });
  await page.locator(".context-menu-row", { hasText: "复制会话名称(N)" }).click();
  assert(await page.evaluate(() => window.__clipboardText) === "Edge Router",
    "filtered session context menu targeted a different session");

  const historyFilter = page.getByRole("textbox", { name: "筛选历史命令", exact: true });
  await historyFilter.fill("COMPOSE UP");
  assert(await page.locator(".history-list button").count() === 1,
    "history filter did not normalize case and multiline whitespace");
  await page.locator(".history-list button").click();
  assert(await page.locator(".send-textarea").inputValue() === "docker compose\nup -d",
    "history selection changed the stored command before insertion");

  const sender = page.locator(".send-panel");
  assert(!(await sender.textContent()).includes("Shell"), "unused Shell sender tab is visible");
  assert(await sender.locator(".send-toolbar > button").count() === 1
    && await sender.locator(".send-toolbar > svg").count() === 0,
  "sender toolbar contains decorative controls");

  await page.getByRole("button", { name: "搜索会话", exact: true }).click();
  await page.locator(".search-dialog").waitFor();
  assert(await page.locator(".search-dialog .dialog-title", { hasText: "会话搜索" }).count() === 1,
    "top search command did not open session search");
  await page.locator(".search-dialog .dialog-title button").click();

  await togglePanel("会话");
  await togglePanel("历史命令");
  await togglePanel("发送");
  const desktop = await page.evaluate(() => ({
    viewportWidth: innerWidth,
    documentWidth: document.documentElement.scrollWidth,
    viewportHeight: innerHeight,
    documentHeight: document.documentElement.scrollHeight,
    terminalWidth: document.querySelector(".center-workspace")?.getBoundingClientRect().width ?? 0,
    rightDock: document.querySelectorAll(".right-stack").length,
    sender: document.querySelectorAll(".send-panel").length,
  }));
  assert(desktop.documentWidth <= desktop.viewportWidth && desktop.documentHeight <= desktop.viewportHeight,
    `desktop workspace overflow: ${JSON.stringify(desktop)}`);
  assert(desktop.terminalWidth > 1100 && desktop.rightDock === 0 && desktop.sender === 0,
    `desktop did not return optional space to the terminal: ${JSON.stringify(desktop)}`);
  await page.screenshot({ path: `${screenshotPrefix}-desktop.png`, fullPage: true });

  await togglePanel("会话");
  await togglePanel("历史命令");
  await togglePanel("发送");
  await page.setViewportSize({ width: 390, height: 844 });
  const mobile = await page.evaluate(() => {
    const center = document.querySelector(".center-workspace")?.getBoundingClientRect();
    return {
      viewportWidth: innerWidth,
      documentWidth: document.documentElement.scrollWidth,
      viewportHeight: innerHeight,
      documentHeight: document.documentElement.scrollHeight,
      leftDisplay: getComputedStyle(document.querySelector(".left-stack")).display,
      rightDisplay: getComputedStyle(document.querySelector(".right-stack")).display,
      senderDisplay: getComputedStyle(document.querySelector(".send-panel")).display,
      center: center ? { left: center.left, right: center.right, top: center.top, bottom: center.bottom } : null,
    };
  });
  assert(mobile.documentWidth <= mobile.viewportWidth && mobile.documentHeight <= mobile.viewportHeight,
    `mobile workspace overflow: ${JSON.stringify(mobile)}`);
  assert(mobile.leftDisplay === "none" && mobile.rightDisplay === "none" && mobile.senderDisplay === "none",
    `optional panels crowd the mobile terminal: ${JSON.stringify(mobile)}`);
  assert(mobile.center?.left === 0 && mobile.center?.right === mobile.viewportWidth,
    `terminal does not fill the mobile viewport: ${JSON.stringify(mobile.center)}`);
  await page.screenshot({ path: `${screenshotPrefix}-mobile.png`, fullPage: true });

  const terminalWrites = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "send_text" || call.command === "send_bytes" || call.command === "run_command"
  )));
  assert(terminalWrites.length === 0,
    `non-input workspace actions wrote to the terminal: ${JSON.stringify(terminalWrites)}`);
  assert(pageErrors.length === 0, `browser exceptions: ${JSON.stringify(pageErrors)}`);

  console.log(JSON.stringify({
    migratedPanels: initial.panels,
    filters: ["resource tag/endpoint", "session tag/endpoint", "normalized history"],
    contextMenu: "single synchronized-input, split, and move actions",
    terminalWrites,
    desktop,
    mobile,
    screenshots: [`${screenshotPrefix}-desktop.png`, `${screenshotPrefix}-mobile.png`],
  }, null, 2));
  await context.close();
} finally {
  await browser?.close().catch(() => {});
  vite.kill("SIGTERM");
  await new Promise((resolve) => {
    if (vite.exitCode !== null) resolve();
    else {
      vite.once("exit", resolve);
      setTimeout(resolve, 2000).unref();
    }
  });
}
