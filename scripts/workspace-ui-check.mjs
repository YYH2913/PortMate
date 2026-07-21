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
        sftp: kind === "ssh" || kind === "tmux",
        scp: kind === "ssh" || kind === "tmux",
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

const mcpGrants = [
  {
    clientId: "ops-console",
    name: "Operations Console",
    scopes: ["read-sessions", "read-logs", "write-input"],
    allowedSessions: ["edge-router"],
    confirmWrites: true,
    expiresAt: null,
    revokedAt: null,
  },
  {
    clientId: "audit-reader",
    name: "Audit Reader",
    scopes: ["read-logs"],
    allowedSessions: [],
    confirmWrites: false,
    expiresAt: null,
    revokedAt: null,
  },
];

const mcpAudit = [
  {
    id: "audit-write",
    ts: new Date(recordedAt - 1_000).toISOString(),
    actor: "mcp:ops-console",
    action: "send_text",
    sessionId: "edge-router",
    decision: "succeeded",
    details: { scope: "write-input", bytes: "18" },
  },
  {
    id: "audit-denied",
    ts: new Date(recordedAt - 2_000).toISOString(),
    actor: "mcp:blocked-client",
    action: "create_tunnel",
    sessionId: null,
    decision: "denied",
    details: { scope: "tunnel", reason: "grant missing" },
  },
  {
    id: "audit-read",
    ts: new Date(recordedAt - 3_000).toISOString(),
    actor: "mcp:audit-reader",
    action: "read_logs",
    sessionId: "bench-uart",
    decision: "authorized",
    details: { scope: "read-logs", source: "serial capture" },
  },
];

const mcpHttpConfig = {
  endpoint: "http://127.0.0.1:8787/mcp",
  tokenRef: "keychain:mcp-http-token",
  tokenAvailable: true,
  defaultOrigin: "http://127.0.0.1:8787",
  executable: "/usr/bin/portmate-mcp",
  storePath: "/home/operator/.local/share/dev.portmate.desktop/portmate-store.sqlite3",
  startCommand: "PORTMATE_STORE_PATH='/home/operator/.local/share/dev.portmate.desktop/portmate-store.sqlite3' PORTMATE_MCP_HTTP=1 /usr/bin/portmate-mcp --http",
};

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
  await context.addInitScript(({ initialSessions, initialEvents, initialWorkspace, initialMcpGrants, initialMcpAudit, initialMcpHttpConfig, historyTimestamp }) => {
    if (!sessionStorage.getItem("portmate.workspaceUiCheck.initialized")) {
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
      sessionStorage.setItem("portmate.workspaceUiCheck.initialized", "true");
    }
    window.__invokeCalls = [];
    window.__sessions = structuredClone(initialSessions);
    window.__mcpGrants = structuredClone(initialMcpGrants);
    window.__clipboardText = "";
    window.__closeSessionError = false;
    window.__tauriCallbacks = new Map();
    window.__tauriEventListeners = new Map();
    window.__tauriCallbackId = 0;
    window.__emitTauriEvent = (event, payload) => {
      const listeners = window.__tauriEventListeners.get(event) || [];
      for (const id of listeners) window.__tauriCallbacks.get(id)?.({ event, id, payload });
    };
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        readText: async () => window.__clipboardText,
        writeText: async (value) => { window.__clipboardText = String(value); },
      },
    });
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (event, id) => {
        const listeners = window.__tauriEventListeners.get(event) || [];
        window.__tauriEventListeners.set(event, listeners.filter((listenerId) => listenerId !== id));
        window.__tauriCallbacks.delete(id);
      },
    };
    window.__TAURI_INTERNALS__ = {
      invoke: async (command, args = {}) => {
        window.__invokeCalls.push({ command, args });
        if (command === "plugin:event|listen") {
          const listeners = window.__tauriEventListeners.get(args.event) || [];
          window.__tauriEventListeners.set(args.event, [...listeners, args.handler]);
          return args.handler;
        }
        if (command === "plugin:event|unlisten") {
          window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener(args.event, args.eventId);
          return null;
        }
        if (command === "list_sessions") return window.__sessions;
        if (command === "save_session_profile") {
          const index = window.__sessions.findIndex((session) => session.profile.id === args.profile.id);
          if (index < 0) throw new Error(`unknown test session: ${args.profile.id}`);
          const saved = {
            ...window.__sessions[index],
            profile: structuredClone(args.profile),
          };
          window.__sessions[index] = saved;
          return saved;
        }
        if (command === "tail_log") return initialEvents.filter((event) => event.sessionId === args.sessionId);
        if (command === "list_mcp_grants") return window.__mcpGrants;
        if (command === "list_mcp_audit") return initialMcpAudit;
        if (command === "mcp_http_config") return initialMcpHttpConfig;
        if (command === "list_mcp_approvals") return [];
        if (command === "respond_mcp_approval") return null;
        if (command === "save_mcp_grant") return initialMcpGrants;
        if (command === "revoke_mcp_grant") return initialMcpGrants.filter((grant) => grant.clientId !== args.clientId);
        if (command === "rotate_mcp_http_token") return { config: initialMcpHttpConfig, token: "portmate-test-token" };
        if (command === "export_mcp_audit") {
          return {
            path: "/tmp/portmate-mcp-audit.jsonl",
            checksumPath: "/tmp/portmate-mcp-audit.jsonl.sha256",
            sha256: "a".repeat(64),
            size: 384,
            records: args.request.recordIds.length,
          };
        }
        if (command === "close_session") {
          if (window.__closeSessionError) throw new Error("simulated close failure");
          const session = initialSessions.find((item) => item.profile.id === args.sessionId);
          return session ? { ...session, runtime: { ...session.runtime, status: "disconnected" } } : null;
        }
        if (command === "delete_session_profile") {
          const deletedProfileId = args.sessionId;
          window.__sessions = window.__sessions.filter((session) => session.profile.id !== deletedProfileId);
          window.__mcpGrants = window.__mcpGrants.map((grant) => {
            if (!grant.allowedSessions.length || !grant.allowedSessions.includes(deletedProfileId)) return grant;
            const allowedSessions = grant.allowedSessions.filter((sessionId) => sessionId !== deletedProfileId);
            return {
              ...grant,
              allowedSessions,
              revokedAt: allowedSessions.length ? grant.revokedAt : grant.revokedAt ?? new Date().toISOString(),
            };
          });
          return {
            deletedProfileId,
            sessions: window.__sessions,
            oneKeys: [],
            hostKeys: { keys: [] },
            grants: window.__mcpGrants,
          };
        }
        if (command === "list_host_keys") return { keys: [] };
        if ([
          "list_files",
          "list_transfers",
          "list_serial_ports",
          "list_one_keys",
        ].includes(command)) return [];
        if (command.startsWith("plugin:event|")) return null;
        return null;
      },
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      transformCallback: (callback) => {
        window.__tauriCallbackId += 1;
        window.__tauriCallbacks.set(window.__tauriCallbackId, callback);
        return window.__tauriCallbackId;
      },
      unregisterCallback: () => {},
      convertFileSrc: (path) => path,
    };
  }, {
    initialSessions: sessions,
    initialEvents: events,
    initialWorkspace: workspace,
    initialMcpGrants: mcpGrants,
    initialMcpAudit: mcpAudit,
    initialMcpHttpConfig: mcpHttpConfig,
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
    const leftDock = document.querySelector('.workspace-dock[data-dock="left"]')?.getBoundingClientRect();
    return {
      snapshotVersion: panels.version,
      viewportWidth: innerWidth,
      documentWidth: document.documentElement.scrollWidth,
      leftDockWidth: leftDock?.width ?? 0,
      activePanel: document.querySelector('.workspace-dock[data-dock="left"]')?.getAttribute("data-active-panel") ?? "",
      dockCount: document.querySelectorAll(".workspace-dock").length,
      globalTabs: document.querySelectorAll(".tab-line").length,
      fileManager: document.querySelector('.workspace-dock-content[data-panel="fileManager"]') !== null,
      connectionSummaryRows: document.querySelectorAll(".crumb-line").length,
      connectionControls: document.querySelectorAll(".connection-toggle").length,
      paneHeaderActions: [...document.querySelectorAll(".terminal-pane > header > button")]
        .map((button) => button.getAttribute("aria-label")),
      topToolsText: document.querySelector(".menu-tools")?.textContent ?? "",
      brand: document.querySelector(".menu-brand")?.textContent?.trim() ?? "",
      menuLabels: [...document.querySelectorAll(".menu-trigger")].map((button) => button.textContent),
      status: document.querySelector(".status-bar")?.textContent ?? "",
      panels: panels.panels,
      docks: panels.docks,
      sizes: panels.sizes,
    };
  });
  assert(initial.documentWidth <= initial.viewportWidth, `initial workspace overflow: ${JSON.stringify(initial)}`);
  assert(initial.globalTabs === 0, "redundant global session tabs are visible");
  assert(!initial.fileManager && initial.dockCount === 1 && initial.activePanel === "explorer",
    `legacy all-visible default did not migrate: ${JSON.stringify(initial)}`);
  assert(initial.leftDockWidth >= 252 && initial.leftDockWidth <= 260,
    `resource dock is not using the compact width: ${initial.leftDockWidth}`);
  assert(initial.docks.active.left === "explorer"
    && JSON.stringify(initial.docks.left) === JSON.stringify(["explorer", "fileManager"])
    && JSON.stringify(initial.docks.right) === JSON.stringify(["history"])
    && JSON.stringify(initial.docks.bottom) === JSON.stringify(["sender"]),
  `default dock layout is wrong: ${JSON.stringify(initial.docks)}`);
  assert(initial.snapshotVersion === 6
    && initial.sizes.left === null && initial.sizes.right === null && initial.sizes.bottom === null,
  `legacy panel snapshot did not migrate to bounded v6 sizes: ${JSON.stringify(initial)}`);
  assert(initial.connectionSummaryRows === 0 && initial.connectionControls === 1,
    `connection context is still duplicated: ${JSON.stringify(initial)}`);
  assert(JSON.stringify(initial.paneHeaderActions) === JSON.stringify(["断开 Edge Router"]),
    `low-frequency pane actions are still permanently visible: ${JSON.stringify(initial.paneHeaderActions)}`);
  assert(initial.brand === "PortMate", `workspace brand is missing: ${JSON.stringify(initial.brand)}`);
  assert(!initial.topToolsText.includes("隧道"),
    `duplicate tunnel shortcut survived: ${initial.topToolsText}`);
  assert(JSON.stringify(initial.menuLabels) === JSON.stringify([
    "会话", "终端", "工作区", "工具",
  ]), `top menu categories are still redundant: ${JSON.stringify(initial.menuLabels)}`);
  assert(!initial.status.includes("窗口 -1×-1") && !initial.status.includes("PortMate Issues"),
    `placeholder status text survived: ${initial.status}`);
  assert(initial.panels.explorer && initial.panels.statusBar
    && !initial.panels.fileManager && !Object.hasOwn(initial.panels, "sessions")
    && !initial.panels.history && !initial.panels.sender,
  `migrated v2 panel snapshot is wrong: ${JSON.stringify(initial.panels)}`);

  await page.locator('.workspace-pane-tab[data-view-id="view-edge"]').click({ button: "right" });
  const workspaceViewMenu = page.locator(".workspace-view-context-menu");
  await workspaceViewMenu.waitFor();
  assert(await workspaceViewMenu.getByRole("button", { name: "移到新窗口", exact: true }).isDisabled()
    && await workspaceViewMenu.getByRole("button", { name: "关闭窗格", exact: true }).isDisabled()
    && await workspaceViewMenu.getByRole("button", { name: "移到新分组", exact: true }).isDisabled()
    && await workspaceViewMenu.getByRole("button", { name: "合并当前分组", exact: true }).isDisabled()
    && await workspaceViewMenu.getByRole("button", { name: "交换窗格", exact: true }).isDisabled()
    && await workspaceViewMenu.getByRole("button", { name: "切换窗格缩放", exact: true }).isDisabled(),
  "relocated single-pane actions do not preserve their disabled state");
  assert(await workspaceViewMenu.getByRole("button", { name: "复制视图", exact: true }).isEnabled(),
    "relocated duplicate-view action is unavailable");
  await page.locator(".center-workspace").click({ position: { x: 400, y: 160 } });
  await workspaceViewMenu.waitFor({ state: "detached" });

  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  const sessionMenuState = await page.locator(".menu-popover button").evaluateAll((buttons) => Object.fromEntries(
    buttons.map((button) => [button.textContent?.trim(), button.disabled]),
  ));
  assert(sessionMenuState["启动会话"] && !sessionMenuState["关闭会话"] && !sessionMenuState["会话设置"],
    `connected session menu capabilities are wrong: ${JSON.stringify(sessionMenuState)}`);
  await page.locator(".menu-trigger", { hasText: "会话" }).click();

  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  const toolMenuState = await page.locator(".menu-popover button").evaluateAll((buttons) => Object.fromEntries(
    buttons.map((button) => [button.textContent?.trim(), button.disabled]),
  ));
  assert(!toolMenuState["传输任务"] && !toolMenuState["端口转发"] && !toolMenuState.Tmux
    && toolMenuState["串口分析器"],
  `SSH tool capabilities are wrong: ${JSON.stringify(toolMenuState)}`);
  await page.locator(".menu-popover button", { hasText: "传输任务" }).click();
  await page.locator(".transfer-dialog").waitFor();
  const sshTransferOptions = await page.locator(".transfer-dialog select").locator("option")
    .evaluateAll((options) => options.map((option) => ({ value: option.value, label: option.textContent })));
  assert(JSON.stringify(sshTransferOptions.map((option) => option.value)) === JSON.stringify(["sftp", "scp", "xmodem", "ymodem", "zmodem"]),
    `SSH transfer capabilities are wrong: ${JSON.stringify(sshTransferOptions)}`);
  assert(await page.locator(".transfer-dialog select").inputValue() === "sftp",
    "SSH transfer dialog did not select its first enabled protocol");
  await page.screenshot({ path: `${screenshotPrefix}-transfer.png`, fullPage: true });
  await page.locator(".transfer-dialog .utility-actions button", { hasText: "取消" }).click();
  await page.locator(".transfer-dialog").waitFor({ state: "detached" });

  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" }).click();
  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  const serialToolState = await page.locator(".menu-popover button").evaluateAll((buttons) => Object.fromEntries(
    buttons.map((button) => [button.textContent?.trim(), button.disabled]),
  ));
  assert(!serialToolState["传输任务"] && serialToolState["端口转发"] && serialToolState.Tmux
    && !serialToolState["串口分析器"],
  `serial tool capabilities are wrong: ${JSON.stringify(serialToolState)}`);
  await page.locator(".menu-popover button", { hasText: "传输任务" }).click();
  await page.locator(".transfer-dialog").waitFor();
  const serialTransferOptions = await page.locator(".transfer-dialog select").locator("option")
    .evaluateAll((options) => options.map((option) => option.value));
  assert(JSON.stringify(serialTransferOptions) === JSON.stringify(["xmodem", "ymodem", "zmodem"])
    && await page.locator(".transfer-dialog select").inputValue() === "xmodem",
  `Serial transfer dialog exposes unsupported protocols: ${JSON.stringify(serialTransferOptions)}`);
  await page.locator(".transfer-dialog .utility-actions button", { hasText: "取消" }).click();
  await page.locator(".transfer-dialog").waitFor({ state: "detached" });
  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Edge Router" }).click();

  await page.evaluate(() => { window.__closeSessionError = true; });
  await page.getByRole("button", { name: "断开 Edge Router", exact: true }).click();
  const closeFailure = page.locator(".notice-dialog", { hasText: "断开会话失败" });
  await closeFailure.waitFor();
  assert((await closeFailure.textContent()).includes("simulated close failure"),
    "close-session backend failure did not surface its diagnostic");
  assert(await page.locator('.workspace-pane-tab.status-connected[data-view-id="view-edge"]').count() === 1,
    "failed close changed the connected workspace tab state");
  assert(await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "close_session").length) === 1,
    "close-session control did not reach the backend exactly once");
  await closeFailure.getByRole("button", { name: "确定", exact: true }).click();
  await page.evaluate(() => { window.__closeSessionError = false; });

  const explorerFilter = page.getByRole("textbox", { name: "筛选资源管理器会话", exact: true });
  await explorerFilter.fill("production");
  assert(await page.locator(".workspace-dock-content.panel-explorer .tree-session").count() === 1,
    "resource tag filter did not remove unrelated sessions");
  assert(await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Edge Router" }).count() === 1,
    "resource tag filter missed Edge Router");
  await explorerFilter.fill("10.0.0.1");
  assert(await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Edge Router" }).count() === 1,
    "resource endpoint filter missed Edge Router");
  await page.getByRole("button", { name: "清除筛选资源管理器会话", exact: true }).click();
  assert(await page.locator(".workspace-dock-content.panel-explorer .tree-session").count() === sessions.length,
    "clearing the resource filter did not restore all sessions");
  assert(await page.locator(".workspace-dock-content.panel-explorer .tree-folder svg").count() === 3,
    "resource group headings contain non-semantic controls");

  const edge = page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Edge Router" });
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
    await page.locator(".menu-trigger", { hasText: "工作区" }).click();
    await page.locator(".menu-popover button", { hasText: label }).click();
  }

  await togglePanel("文件管理器");
  const leftDock = page.locator('.workspace-dock[data-dock="left"]');
  await leftDock.locator('.workspace-dock-content[data-panel="fileManager"]').waitFor();
  const fileDockLayout = await leftDock.evaluate((dock) => ({
    width: dock.getBoundingClientRect().width,
    active: dock.getAttribute("data-active-panel"),
    tabs: [...dock.querySelectorAll(".workspace-dock-tab")].map((tab) => tab.getAttribute("data-panel")),
    panels: [...dock.querySelectorAll(".workspace-dock-content")].map((panel) => panel.getAttribute("data-panel")),
    panes: [...dock.querySelectorAll(".file-browser-pane")].map((pane) => {
      const rect = pane.getBoundingClientRect();
      return { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right };
    }),
    actions: [...dock.querySelectorAll(".file-actions")].map((actions) => ({
      clientWidth: actions.clientWidth,
      scrollWidth: actions.scrollWidth,
      labels: [...actions.querySelectorAll("button")].map((button) => button.getAttribute("aria-label")),
      icons: actions.querySelectorAll("button svg").length,
    })),
  }));
  assert(fileDockLayout.width >= 350 && fileDockLayout.width <= 366
    && fileDockLayout.active === "fileManager"
    && JSON.stringify(fileDockLayout.tabs) === JSON.stringify(["explorer", "fileManager"])
    && JSON.stringify(fileDockLayout.panels) === JSON.stringify(["explorer", "fileManager"])
    && fileDockLayout.panes.length === 2
    && fileDockLayout.panes[1].top >= fileDockLayout.panes[0].bottom - 1
    && fileDockLayout.actions.length === 2
    && fileDockLayout.actions.every((actions) => (
      actions.scrollWidth <= actions.clientWidth + 1
      && actions.labels.length === 7
      && actions.labels.every(Boolean)
      && actions.icons === 7
    )),
  `file manager and explorer are not simultaneously visible in the left dock: ${JSON.stringify(fileDockLayout)}`);
  await page.screenshot({ path: `${screenshotPrefix}-file-manager.png`, fullPage: true });

  const fileTitle = leftDock.locator('.workspace-dock-tab[data-panel="fileManager"]');
  const explorerTitle = leftDock.locator('.workspace-dock-tab[data-panel="explorer"]');
  const explorerTitleBox = await explorerTitle.boundingBox();
  assert(explorerTitleBox, "explorer title geometry is unavailable for same-dock reorder");
  const reorderTransfer = await page.evaluateHandle(() => new DataTransfer());
  await fileTitle.dispatchEvent("dragstart", { dataTransfer: reorderTransfer });
  await explorerTitle.dispatchEvent("dragover", {
    dataTransfer: reorderTransfer,
    clientY: explorerTitleBox.y + 2,
  });
  await explorerTitle.dispatchEvent("drop", {
    dataTransfer: reorderTransfer,
    clientY: explorerTitleBox.y + 2,
  });
  await page.waitForFunction(() => {
    const snapshot = JSON.parse(localStorage.getItem("portmate.workspacePanels.v2") || "null");
    return JSON.stringify(snapshot?.docks?.left) === JSON.stringify(["fileManager", "explorer"]);
  });
  assert(JSON.stringify(await leftDock.locator(".workspace-panel-window").evaluateAll(
    (panels) => panels.map((panel) => panel.getAttribute("data-panel")),
  )) === JSON.stringify(["fileManager", "explorer"]), "same-dock title drag did not reorder visible windows");
  await reorderTransfer.dispose();

  await togglePanel("历史命令");
  assert(await page.locator('.workspace-dock[data-dock="right"][data-active-panel="history"]').count() === 1
    && await page.locator(".workspace-dock").count() === 2
    && await leftDock.getAttribute("data-active-panel") === "fileManager",
  "history did not open independently in the right dock");

  const historyFilter = page.getByRole("textbox", { name: "筛选历史命令", exact: true });
  await historyFilter.fill("COMPOSE UP");
  assert(await page.locator(".history-list button").count() === 1,
    "history filter did not normalize case and multiline whitespace");
  await page.locator(".history-list button").click();

  await togglePanel("发送");
  const bottomDock = page.locator('.workspace-dock[data-dock="bottom"][data-active-panel="sender"]');
  await bottomDock.waitFor();
  const multiDockLayout = await page.evaluate(() => {
    const center = document.querySelector(".center-workspace")?.getBoundingClientRect();
    return {
      docks: [...document.querySelectorAll(".workspace-dock")].map((dock) => ({
        dock: dock.getAttribute("data-dock"),
        panel: dock.getAttribute("data-active-panel"),
      })),
      center: center ? { width: center.width, height: center.height } : null,
    };
  });
  assert(JSON.stringify(multiDockLayout.docks) === JSON.stringify([
    { dock: "left", panel: "fileManager" },
    { dock: "right", panel: "history" },
    { dock: "bottom", panel: "sender" },
  ]) && multiDockLayout.center?.width > 700 && multiDockLayout.center?.height > 550,
  `left, right and bottom docks are not simultaneously usable: ${JSON.stringify(multiDockLayout)}`);

  const leftResizer = page.getByRole("separator", { name: "调整左侧停靠区宽度", exact: true });
  const rightResizer = page.getByRole("separator", { name: "调整右侧停靠区宽度", exact: true });
  const bottomResizer = page.getByRole("separator", { name: "调整底部停靠区高度", exact: true });
  assert(await page.getByRole("separator", { name: /调整.*停靠区/ }).count() === 3,
    "visible docks do not expose one resize separator each");
  const layoutBox = await page.locator(".wind-layout").boundingBox();
  const leftResizerBox = await leftResizer.boundingBox();
  assert(layoutBox && leftResizerBox, "left dock resize geometry is unavailable");
  await page.mouse.move(leftResizerBox.x + leftResizerBox.width / 2, leftResizerBox.y + 80);
  await page.mouse.down();
  await page.mouse.move(layoutBox.x + 420, leftResizerBox.y + 80, { steps: 4 });
  await page.mouse.up();
  await rightResizer.focus();
  await rightResizer.press("ArrowLeft");
  await bottomResizer.focus();
  await bottomResizer.press("ArrowUp");
  await page.waitForFunction(() => {
    const snapshot = JSON.parse(localStorage.getItem("portmate.workspacePanels.v2") || "null");
    return snapshot?.version === 6
      && snapshot.sizes?.left === 420
      && snapshot.sizes?.right === 296
      && snapshot.sizes?.bottom === 226;
  });
  const resizedDockLayout = await page.evaluate(() => {
    const rect = (dock) => document.querySelector(`.workspace-dock[data-dock="${dock}"]`)?.getBoundingClientRect();
    const snapshot = JSON.parse(localStorage.getItem("portmate.workspacePanels.v2"));
    return {
      left: rect("left")?.width ?? 0,
      right: rect("right")?.width ?? 0,
      bottom: rect("bottom")?.height ?? 0,
      sizes: snapshot.sizes,
    };
  });
  assert(resizedDockLayout.left >= 419 && resizedDockLayout.left <= 421
    && resizedDockLayout.right >= 295 && resizedDockLayout.right <= 297
    && resizedDockLayout.bottom >= 225 && resizedDockLayout.bottom <= 227,
  `dock resize did not update all three grid tracks: ${JSON.stringify(resizedDockLayout)}`);
  assert(await page.locator(".send-textarea").inputValue() === "docker compose\nup -d",
    "history selection changed the stored command before insertion");

  const sender = bottomDock;
  assert(!(await sender.textContent()).includes("Shell"), "unused Shell sender tab is visible");
  assert(await sender.locator(".send-toolbar > button").count() === 1
    && await sender.locator(".send-toolbar > svg").count() === 0,
  "sender toolbar contains decorative controls");
  await page.screenshot({ path: `${screenshotPrefix}-sender.png`, fullPage: true });

  await leftDock.locator('.workspace-dock-tab[data-panel="explorer"] .workspace-dock-tab-label').click();
  await leftDock.locator('.workspace-dock-content[data-panel="explorer"]').waitFor();
  assert(await leftDock.getAttribute("data-active-panel") === "explorer"
    && await leftDock.locator('.workspace-dock-content[data-panel="fileManager"]').count() === 1
    && await leftDock.locator('.workspace-dock-content[data-panel="explorer"]').count() === 1,
  "focusing one dock window hid another visible dock window");
  const uart = page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" });
  await uart.click();
  await page.waitForFunction(() => document.querySelector(".workspace-dock-content.panel-explorer .tree-session.active")?.textContent?.includes("Bench UART"));
  assert(await page.locator(".pane-serial-tools").count() === 1,
    "serial line controls were lost while consolidating the workspace toolbar");

  await page.getByRole("button", { name: "搜索会话", exact: true }).click();
  await page.locator(".search-dialog").waitFor();
  assert(await page.locator(".search-dialog .dialog-title", { hasText: "会话搜索" }).count() === 1,
    "top search command did not open session search");
  const searchInput = page.getByRole("combobox", { name: "搜索会话和日志", exact: true });
  await searchInput.fill("hardware");
  assert(await page.getByRole("option", { name: /Bench UART/ }).count() === 1,
    "global session search did not match a profile tag");
  await page.screenshot({ path: `${screenshotPrefix}-search.png`, fullPage: true });
  await searchInput.fill("");
  await searchInput.press("ArrowDown");
  assert(await page.getByRole("option", { name: /Bench UART/ }).getAttribute("aria-selected") === "true",
    "session search ArrowDown did not select the next result");
  await searchInput.press("Enter");
  await page.locator(".search-dialog").waitFor({ state: "detached" });
  assert(await page.locator(".workspace-pane-tab.active", { hasText: "Bench UART" }).count() === 1,
    "session search Enter did not activate the selected workspace view");

  await page.getByRole("button", { name: "搜索会话", exact: true }).click();
  await page.getByRole("tab", { name: "日志", exact: true }).click();
  const logSearchInput = page.getByRole("combobox", { name: "搜索会话和日志", exact: true });
  await logSearchInput.fill("production");
  assert(await page.getByRole("option", { name: /Edge Router/ }).count() === 1,
    "global log search did not match its session context");
  await logSearchInput.press("Escape");
  await page.locator(".search-dialog").waitFor({ state: "detached" });

  await page.getByRole("button", { name: "搜索会话", exact: true }).click();
  const restoreSearchInput = page.getByRole("combobox", { name: "搜索会话和日志", exact: true });
  await restoreSearchInput.fill("gateway");
  await restoreSearchInput.press("Enter");
  await page.locator(".search-dialog").waitFor({ state: "detached" });
  assert(await page.locator(".workspace-pane-tab.active", { hasText: /^Edge$/ }).count() === 1,
    "tag search did not restore the exact Edge workspace view");

  const viewDragData = await page.evaluateHandle(() => new DataTransfer());
  const benchViewTab = page.locator(".workspace-pane-tab", { hasText: "Bench UART" });
  const edgePane = page.locator(".terminal-pane", { has: page.locator(".workspace-pane-tab", { hasText: /^Edge$/ }) });
  const edgePaneBounds = await edgePane.boundingBox();
  assert(edgePaneBounds, "edge pane geometry is unavailable for view drag");
  await benchViewTab.dispatchEvent("dragstart", { dataTransfer: viewDragData });
  await edgePane.dispatchEvent("dragover", {
    dataTransfer: viewDragData,
    clientX: edgePaneBounds.x + edgePaneBounds.width - 2,
    clientY: edgePaneBounds.y + edgePaneBounds.height / 2,
  });
  assert(await edgePane.getAttribute("data-view-drop-zone") === "right",
    "dragging a view over the pane edge did not expose the right split drop zone");
  await edgePane.dispatchEvent("drop", {
    dataTransfer: viewDragData,
    clientX: edgePaneBounds.x + edgePaneBounds.width - 2,
    clientY: edgePaneBounds.y + edgePaneBounds.height / 2,
  });
  await page.waitForFunction(() => document.querySelectorAll(".terminal-pane").length === 2);
  assert(await page.locator(".terminal-pane .workspace-pane-tab", { hasText: "Edge" }).count() === 1
    && await page.locator(".terminal-pane .workspace-pane-tab", { hasText: "Bench UART" }).count() === 1,
  "view edge drop did not create one pane per terminal view");
  await page.screenshot({ path: `${screenshotPrefix}-view-split-drop.png`, fullPage: true });

  const benchPane = page.locator(".terminal-pane", { has: page.locator(".workspace-pane-tab", { hasText: "Bench UART" }) });
  const edgePaneAfterSplit = page.locator(".terminal-pane", { has: page.locator(".workspace-pane-tab", { hasText: /^Edge$/ }) });
  const edgeCenterBounds = await edgePaneAfterSplit.boundingBox();
  assert(edgeCenterBounds, "edge pane geometry is unavailable after split drop");
  await benchPane.locator(".workspace-pane-tab", { hasText: "Bench UART" }).dispatchEvent("dragstart", { dataTransfer: viewDragData });
  await edgePaneAfterSplit.dispatchEvent("dragover", {
    dataTransfer: viewDragData,
    clientX: edgeCenterBounds.x + edgeCenterBounds.width / 2,
    clientY: edgeCenterBounds.y + edgeCenterBounds.height / 2,
  });
  assert(await edgePaneAfterSplit.getAttribute("data-view-drop-zone") === "center",
    "dragging a view over the pane center did not expose the group drop zone");
  await edgePaneAfterSplit.dispatchEvent("drop", {
    dataTransfer: viewDragData,
    clientX: edgeCenterBounds.x + edgeCenterBounds.width / 2,
    clientY: edgeCenterBounds.y + edgeCenterBounds.height / 2,
  });
  await page.waitForFunction(() => document.querySelectorAll(".terminal-pane").length === 1);
  assert(await page.locator(".workspace-pane-tab", { hasText: "Edge" }).count() === 1
    && await page.locator(".workspace-pane-tab", { hasText: "Bench UART" }).count() === 1,
  "center drop did not merge the moved view back into the target group");
  await viewDragData.dispose();

  async function openTerminalSettings() {
    await page.locator(".menu-trigger", { hasText: "工具" }).click();
    await page.locator(".menu-popover button", { hasText: "终端设置" }).click();
    await page.locator(".terminal-settings-dialog").waitFor();
  }

  async function openNewSessionSettings() {
    await page.locator(".menu-trigger", { hasText: "会话" }).click();
    await page.locator(".menu-popover button", { hasText: "新建会话" }).click();
    await page.locator(".session-settings-dialog").waitFor();
  }

  await openTerminalSettings();
  const settingsPages = await page.locator(".terminal-settings-dialog .settings-tabs > button")
    .evaluateAll((buttons) => buttons.map((button) => button.textContent?.trim()));
  assert(JSON.stringify(settingsPages) === JSON.stringify([
    "应用",
    "安全",
    "快捷键",
    "自动补全",
    "命令历史",
    "鼠标",
    "同步输入",
  ]), `terminal settings navigation is still redundant: ${JSON.stringify(settingsPages)}`);
  const settingsBounds = await page.locator(".terminal-settings-dialog").evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return { width: rect.width, height: rect.height, scrollWidth: dialog.scrollWidth, scrollHeight: dialog.scrollHeight };
  });
  assert(settingsBounds.width <= 720 && settingsBounds.height <= 620
    && settingsBounds.scrollWidth <= settingsBounds.width
    && settingsBounds.scrollHeight <= settingsBounds.height,
  `terminal settings dialog is not compact: ${JSON.stringify(settingsBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-settings.png`, fullPage: true });
  const startupSelect = (index) => page.locator(".terminal-settings-dialog .setting-row", {
    hasText: `会话 ${index}:`,
  }).locator("select");
  const firstStartup = startupSelect(1);
  const secondStartup = startupSelect(2);
  assert(await page.getByRole("combobox", { name: "会话 1:", exact: true }).count() === 1
    && await page.getByRole("combobox", { name: "会话 2:", exact: true }).count() === 1,
  "startup session selectors do not expose stable accessible names");
  assert(await firstStartup.isDisabled() && await secondStartup.isDisabled(),
    "specific startup selectors are enabled outside specific mode");
  const settingsText = await page.locator(".terminal-settings-dialog").textContent();
  assert(!settingsText.includes("语言:")
    && !settingsText.includes("窗口不透明度")
    && !settingsText.includes("显示关闭标签页确认")
    && !settingsText.includes("外观")
    && !settingsText.includes("小部件")
    && !settingsText.includes("X Server"),
  `unimplemented application preferences are still visible: ${settingsText}`);
  const startupOptions = await firstStartup.locator("option").evaluateAll((options) => options.map((option) => ({
    value: option.value,
    label: option.textContent,
  })));
  assert(JSON.stringify(startupOptions.map((option) => option.value)) === JSON.stringify([
    "",
    "edge-router",
    "bench-uart",
    "local-shell",
  ]), `startup selectors do not use real profile IDs: ${JSON.stringify(startupOptions)}`);
  assert(!startupOptions.some((option) => ["最近使用", "活动工作区", "Serial 默认组", "SSH 默认组"].includes(option.value)),
    `placeholder startup values survived: ${JSON.stringify(startupOptions)}`);
  await page.locator(".terminal-settings-dialog .setting-radio", {
    hasText: "指定一个会话或一组会话(S)",
  }).locator("input").check();
  assert(!await firstStartup.isDisabled() && !await secondStartup.isDisabled(),
    "specific startup selectors did not become enabled");
  await firstStartup.selectOption("bench-uart");
  await secondStartup.selectOption("local-shell");
  await page.locator(".terminal-settings-dialog .dialog-actions button", { hasText: "保存" }).click();
  await page.locator(".terminal-settings-dialog").waitFor({ state: "detached" });
  const startupSettings = await page.evaluate(() => {
    const prefs = JSON.parse(localStorage.getItem("portmate.terminalPrefs"));
    const legacyKeys = [
      "accent",
      "proxyEnabled",
      "tabPosition",
      "terminalScrollback",
      "binaryView",
      "fontFamily1",
      "fileManagerEnabled",
      "quickBarEnabled",
      "xServerEnabled",
      "extensionsEnabled",
    ].filter((key) => Object.hasOwn(prefs, key));
    return { mode: prefs.startupMode, sessions: prefs.startupSessions, legacyKeys };
  });
  assert(startupSettings.mode === "specific"
    && JSON.stringify(startupSettings.sessions) === JSON.stringify(["bench-uart", "local-shell", "", ""])
    && startupSettings.legacyKeys.length === 0,
  `startup settings did not persist exact profile IDs: ${JSON.stringify(startupSettings)}`);
  await openTerminalSettings();
  assert(await startupSelect(1).inputValue() === "bench-uart"
    && await startupSelect(2).inputValue() === "local-shell",
  "saved startup profiles did not restore in the settings dialog");
  await page.locator(".terminal-settings-dialog .dialog-actions button", { hasText: "取消" }).click();
  await page.locator(".terminal-settings-dialog").waitFor({ state: "detached" });

  const expectedSessionPages = {
    Shell: ["会话", "终端", "日志", "触发器", "传输", "Shell"],
    SSH: ["会话", "终端", "日志", "触发器", "传输", "SSH", "代理", "验证", "代理人", "密码", "公钥"],
    Tmux: ["会话", "终端", "日志", "触发器", "传输", "Tmux", "代理", "验证", "代理人", "密码", "公钥"],
    Telnet: ["会话", "终端", "日志", "触发器", "传输", "Telnet", "代理"],
    Tcp: ["会话", "终端", "日志", "触发器", "传输", "Tcp", "代理"],
    Serial: ["会话", "终端", "日志", "触发器", "传输", "串口"],
  };
  const protocolPageLabels = {
    Shell: "Shell",
    SSH: "SSH",
    Tmux: "Tmux",
    Telnet: "Telnet",
    Tcp: "Tcp",
    Serial: "串口",
  };
  await page.evaluate(() => {
    for (const key of Object.keys(localStorage)) {
      if (key.startsWith("portmate.sessionPrefs.")) localStorage.removeItem(key);
    }
  });
  await openNewSessionSettings();
  const protocolSelect = page.getByRole("combobox", { name: "会话类型", exact: true });
  const sectionSelect = page.getByRole("combobox", { name: "会话配置项", exact: true });
  const profileNameInput = page.locator(".session-settings-dialog .dialog-field", { hasText: "名称:" }).locator("input");
  const profileGroupInput = page.locator(".session-settings-dialog .dialog-field", { hasText: "分组:" }).locator("input");
  const profileTagsInput = page.locator(".session-settings-dialog .dialog-field", { hasText: "标签:" }).locator("input");
  await profileNameInput.fill("😀".repeat(129));
  await profileGroupInput.fill("g".repeat(257));
  await profileTagsInput.fill("alpha");
  await profileTagsInput.press("End");
  await profileTagsInput.pressSequentially(", beta");
  assert(Array.from(await profileNameInput.inputValue()).length === 128
    && Array.from(await profileGroupInput.inputValue()).length === 256
    && await profileTagsInput.inputValue() === "alpha, beta",
  "session metadata bounds or incremental comma-separated tag editing failed");
  assert(await page.locator(".session-settings-dialog .protocol-tabs").count() === 0
    && await page.locator(".session-settings-dialog .settings-tree").count() === 0,
  "redundant session settings navigation is still rendered");
  for (const [protocol, expectedPages] of Object.entries(expectedSessionPages)) {
    await protocolSelect.selectOption(protocol);
    const actualPages = await sectionSelect.locator("option")
      .evaluateAll((options) => options.map((option) => option.textContent?.trim()));
    assert(JSON.stringify(actualPages) === JSON.stringify(expectedPages),
      `${protocol} session settings are redundant: ${JSON.stringify(actualPages)}`);
    await sectionSelect.selectOption({ label: protocolPageLabels[protocol] });
    assert(await page.locator(".session-settings-dialog .session-form > *").count() > 0,
      `${protocol} has no real protocol settings`);
  }
  await protocolSelect.selectOption("SSH");
  await sectionSelect.selectOption("传输");
  assert(await page.locator(".session-settings-dialog .dialog-field", { hasText: "SFTP:" }).count() === 1
    && await page.locator(".session-settings-dialog .dialog-field", { hasText: "SCP:" }).count() === 1,
  "SSH transfer capabilities are missing from the consolidated page");
  await protocolSelect.selectOption("Serial");
  await sectionSelect.selectOption("传输");
  await page.waitForTimeout(180);
  assert(await page.locator(".session-settings-dialog .dialog-field", { hasText: "SFTP:" }).count() === 0
    && await page.locator(".session-settings-dialog .dialog-field", { hasText: "XModem:" }).count() === 1,
  "Serial transfer page exposes capabilities from another protocol");
  const sessionSettingsText = await page.locator(".session-settings-dialog").textContent();
  assert(!sessionSettingsText.includes("保存为默认设置")
    && !sessionSettingsText.includes("密钥交换")
    && !sessionSettingsText.includes("MAC 哈希")
    && !sessionSettingsText.includes("X11"),
  `non-runtime session settings are still visible: ${sessionSettingsText}`);
  const sessionSettingsBounds = await page.locator(".session-settings-dialog").evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return { width: rect.width, height: rect.height, scrollWidth: dialog.scrollWidth, scrollHeight: dialog.scrollHeight };
  });
  assert(sessionSettingsBounds.width <= 760 && sessionSettingsBounds.height <= 640
    && sessionSettingsBounds.scrollWidth <= sessionSettingsBounds.width
    && sessionSettingsBounds.scrollHeight <= sessionSettingsBounds.height,
  `session settings dialog is not compact: ${JSON.stringify(sessionSettingsBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-session-settings.png`, fullPage: true });
  await page.locator(".session-settings-dialog .dialog-actions button", { hasText: "取消" }).click();
  await page.locator(".session-settings-dialog").waitFor({ state: "detached" });

  const sessionPreferenceKeys = await page.evaluate(() => (
    Object.keys(localStorage).filter((key) => key.startsWith("portmate.sessionPrefs."))
  ));
  assert(sessionPreferenceKeys.length === 0,
    `closing session settings persisted non-runtime preferences: ${JSON.stringify(sessionPreferenceKeys)}`);

  async function setActiveSessionTheme(theme) {
    await page.locator(".menu-trigger", { hasText: "会话" }).click();
    await page.locator(".menu-popover button", { hasText: "会话设置" }).click();
    const dialog = page.locator(".session-settings-dialog");
    await dialog.waitFor();
    await dialog.getByRole("combobox", { name: "会话配置项", exact: true }).selectOption("终端");
    const terminalBounds = await dialog.locator(".dialog-field").evaluateAll((fields) => Object.fromEntries(fields.flatMap((field) => {
      const label = field.querySelector(":scope > span")?.textContent?.trim() ?? "";
      const input = field.querySelector("input");
      return input ? [[label, {
        min: input.getAttribute("min"),
        max: input.getAttribute("max"),
        maxLength: input.getAttribute("maxlength"),
      }]] : [];
    })));
    assert(JSON.stringify(terminalBounds) === JSON.stringify({
      "终端:(T)": { min: null, max: null, maxLength: "64" },
      "行:(R)": { min: "1", max: "512", maxLength: null },
      "列:(C)": { min: "1", max: "1024", maxLength: null },
      "滚屏:(S)": { min: "0", max: "10000000", maxLength: null },
      "字体:(F)": { min: null, max: null, maxLength: "256" },
      "字号:(Z)": { min: "6", max: "72", maxLength: null },
    }), `terminal input bounds are missing or inconsistent: ${JSON.stringify(terminalBounds)}`);
    const themeSelect = dialog.locator(".dialog-field", { hasText: "主题:" }).locator("select");
    const options = await themeSelect.locator("option").evaluateAll((items) => items.map((item) => item.value));
    assert(JSON.stringify(options) === JSON.stringify([
      "portmate-dark",
      "graphite",
      "solarized-dark",
      "portmate-light",
    ]), `terminal theme choices are incomplete: ${JSON.stringify(options)}`);
    await themeSelect.selectOption(theme);
    if (theme === "portmate-light") {
      await page.screenshot({ path: `${screenshotPrefix}-terminal-theme-settings.png`, fullPage: true });
    }
    await dialog.getByRole("button", { name: "保存", exact: true }).click();
    await dialog.waitFor({ state: "detached" });
  }

  const localShell = page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Local Shell" });
  await localShell.click();
  await page.waitForFunction(() => document.querySelector(".workspace-dock-content.panel-explorer .tree-session.active")?.textContent?.includes("Local Shell"));
  const activeXterm = page.locator(".terminal-pane.active .xterm");
  await activeXterm.evaluate((element) => { element.dataset.themeTestIdentity = "retained"; });
  await setActiveSessionTheme("portmate-light");
  await page.locator('.terminal-pane.active .terminal-host[data-terminal-theme="portmate-light"]').waitFor();
  const lightThemeState = await page.locator(".terminal-pane.active .terminal-canvas").evaluate((canvas) => ({
    background: getComputedStyle(canvas).backgroundColor,
    retained: canvas.querySelector(".xterm")?.dataset.themeTestIdentity ?? "",
  }));
  assert(lightThemeState.background === "rgb(247, 248, 250)" && lightThemeState.retained === "retained",
    `terminal theme did not update in place: ${JSON.stringify(lightThemeState)}`);
  await page.screenshot({ path: `${screenshotPrefix}-terminal-light-theme.png`, fullPage: true });
  await setActiveSessionTheme("portmate-dark");
  await page.locator('.terminal-pane.active .terminal-host[data-terminal-theme="portmate-dark"]').waitFor();
  assert(await activeXterm.getAttribute("data-theme-test-identity") === "retained",
    "restoring the terminal theme replaced the live XTerm instance");

  const detachedUrl = new URL(appUrl);
  detachedUrl.searchParams.set("detachedPane", "1");
  detachedUrl.searchParams.set("windowId", "theme-check-window");
  detachedUrl.searchParams.set("paneId", "theme-check-pane");
  detachedUrl.searchParams.set("viewId", "theme-check-view");
  detachedUrl.searchParams.set("sessionId", "local-shell");
  detachedUrl.searchParams.set("title", "Local Shell");
  detachedUrl.searchParams.set("color", "");
  detachedUrl.searchParams.set("keyMode", "remote");
  const detachedPage = await context.newPage();
  const detachedPageErrors = [];
  detachedPage.on("pageerror", (error) => detachedPageErrors.push(error.message));
  await detachedPage.goto(detachedUrl.toString());
  const detachedXterm = detachedPage.locator(".detached-pane-terminal .xterm");
  await detachedPage.locator('.detached-pane-terminal .terminal-host[data-terminal-theme="portmate-dark"]').waitFor();
  await detachedXterm.evaluate((element) => { element.dataset.profileUpdateIdentity = "retained"; });
  await detachedPage.evaluate(() => {
    const index = window.__sessions.findIndex((session) => session.profile.id === "local-shell");
    const updated = structuredClone(window.__sessions[index]);
    updated.profile.terminal.theme = "portmate-light";
    window.__sessions[index] = updated;
    window.__emitTauriEvent("portmate-session-profile-updated", updated);
  });
  await detachedPage.locator('.detached-pane-terminal .terminal-host[data-terminal-theme="portmate-light"]').waitFor();
  const detachedThemeState = await detachedPage.locator(".detached-pane-terminal .terminal-canvas").evaluate((canvas) => ({
    background: getComputedStyle(canvas).backgroundColor,
    retained: canvas.querySelector(".xterm")?.dataset.profileUpdateIdentity ?? "",
  }));
  assert(detachedThemeState.background === "rgb(247, 248, 250)"
    && detachedThemeState.retained === "retained"
    && detachedPageErrors.length === 0,
  `detached profile update did not apply in place: ${JSON.stringify({ detachedThemeState, detachedPageErrors })}`);
  await detachedPage.screenshot({ path: `${screenshotPrefix}-detached-theme.png`, fullPage: true });
  await detachedPage.close();
  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Edge Router" }).click();

  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.locator(".menu-popover button", { hasText: "MCP Bridge" }).click();
  const mcpDialog = page.locator(".mcp-dialog");
  await mcpDialog.waitFor();
  const mcpTabs = await mcpDialog.getByRole("tab").evaluateAll((tabs) => tabs.map((tab) => tab.textContent?.trim()));
  assert(JSON.stringify(mcpTabs) === JSON.stringify(["授权", "HTTP", "审计"]),
    `MCP task navigation is redundant: ${JSON.stringify(mcpTabs)}`);
  assert(await mcpDialog.locator(".mcp-content").count() === 1
    && await mcpDialog.locator(".mcp-http-view").count() === 0
    && await mcpDialog.locator(".mcp-audit-view").count() === 0,
  "MCP grant page renders inactive task content");
  assert(await mcpDialog.locator(".mcp-grants > button").count() === mcpGrants.length + 1,
    "MCP grants did not load into the compact grant workspace");
  assert(await mcpDialog.getByRole("checkbox", { name: "写操作每次确认", exact: true }).isChecked(),
    "MCP write confirmation setting did not load for the selected grant");
  await page.screenshot({ path: `${screenshotPrefix}-mcp-grants.png`, fullPage: true });

  await mcpDialog.getByRole("tab", { name: "HTTP", exact: true }).click();
  await mcpDialog.locator(".mcp-http-view").waitFor();
  assert(await mcpDialog.locator(".mcp-content").count() === 0
    && await mcpDialog.locator(".mcp-audit-view").count() === 0,
  "MCP HTTP page renders inactive task content");
  const mcpHttpText = await mcpDialog.locator(".mcp-http-panel").textContent();
  assert(mcpHttpText.includes(mcpHttpConfig.endpoint)
    && mcpHttpText.includes(mcpHttpConfig.executable)
    && mcpHttpText.includes(mcpHttpConfig.storePath)
    && await mcpDialog.getByRole("textbox", { name: "MCP HTTP 启动命令", exact: true }).inputValue() === mcpHttpConfig.startCommand
    && !mcpHttpText.includes("cargo run"),
  "MCP HTTP packaged executable/store configuration did not load");

  await mcpDialog.getByRole("tab", { name: "审计", exact: true }).click();
  const auditView = mcpDialog.locator(".mcp-audit-view");
  await auditView.waitFor();
  assert(await mcpDialog.locator(".mcp-content").count() === 0
    && await mcpDialog.locator(".mcp-http-view").count() === 0,
  "MCP audit page renders inactive task content");
  assert(await auditView.locator(".mcp-audit-list > button").count() === mcpAudit.length,
    "MCP audit list did not load all records");

  const auditSearch = page.getByRole("textbox", { name: "筛选 MCP 审计", exact: true });
  const auditDecision = page.getByRole("combobox", { name: "筛选审计决策", exact: true });
  const auditSession = page.getByRole("combobox", { name: "筛选审计会话", exact: true });
  const auditScope = page.getByRole("combobox", { name: "筛选审计权限", exact: true });
  await auditSearch.fill("grant missing");
  assert(await auditView.locator(".mcp-audit-list > button").count() === 1
    && (await auditView.locator(".mcp-audit-inspector").textContent()).includes("grant missing"),
  "MCP audit query did not search record details");
  await auditSearch.fill("");
  await auditDecision.selectOption("authorized");
  assert(await auditView.locator(".mcp-audit-list > button").count() === 1,
    "MCP audit decision filter is not applied");
  await auditDecision.selectOption("");
  await auditSession.selectOption("edge-router");
  assert(await auditView.locator(".mcp-audit-list > button").count() === 1,
    "MCP audit session filter is not applied");
  await auditSession.selectOption("");
  await auditScope.selectOption("read-logs");
  assert(await auditView.locator(".mcp-audit-list > button").count() === 1
    && (await auditView.locator(".mcp-audit-inspector").textContent()).includes("audit-read"),
  "MCP audit scope filter or inspector selection is wrong");
  await page.getByRole("button", { name: "导出 MCP 审计", exact: true }).click();
  const exportCall = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "export_mcp_audit").at(-1));
  assert(JSON.stringify(exportCall?.args?.request?.recordIds) === JSON.stringify(["audit-read"]),
    `MCP audit export ignored the active filters: ${JSON.stringify(exportCall)}`);
  assert((await auditView.locator(".mcp-audit-export").textContent()).includes("已导出 1 条"),
    "MCP audit export result is not visible");
  await page.screenshot({ path: `${screenshotPrefix}-mcp-audit.png`, fullPage: true });
  await mcpDialog.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
  await mcpDialog.waitFor({ state: "detached" });

  await page.waitForFunction(() => (window.__tauriEventListeners.get("portmate-mcp-approval") || []).length > 0);
  const approvalIds = await page.evaluate(() => {
    const now = Date.now();
    const approval = (id, clientId, action, sessionId, scope, offset = 0) => ({
      id,
      clientId,
      action,
      sessionId,
      scope,
      createdAt: new Date(now + offset).toISOString(),
      expiresAt: new Date(now + offset + 60_000).toISOString(),
    });
    const first = approval("11111111-1111-4111-8111-111111111111", "ops-console", "run_command", "edge-router", "write-input");
    const second = approval("22222222-2222-4222-8222-222222222222", "session-manager", "close_session", "bench-uart", "manage-sessions", 1);
    window.__emitTauriEvent("portmate-mcp-approval", { ...first, scope: "tunnel" });
    window.__emitTauriEvent("portmate-mcp-approval", first);
    window.__emitTauriEvent("portmate-mcp-approval", first);
    window.__emitTauriEvent("portmate-mcp-approval", second);
    return [first.id, second.id];
  });
  const approvalDialog = page.getByRole("alertdialog", { name: "MCP 写操作审批", exact: true });
  await approvalDialog.waitFor();
  await page.waitForTimeout(180);
  assert((await approvalDialog.textContent()).includes("待处理 2 项")
    && (await approvalDialog.textContent()).includes("Operations Console") === false
    && (await approvalDialog.textContent()).includes("ops-console"),
  "MCP approval queue did not deduplicate or expose the exact client ID");
  await page.waitForFunction(() => document.activeElement?.textContent?.includes("拒绝"));
  const desktopApprovalBounds = await approvalDialog.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
  });
  assert(desktopApprovalBounds.left >= 0 && desktopApprovalBounds.right <= 1440
    && desktopApprovalBounds.top >= 0 && desktopApprovalBounds.bottom <= 900,
  `desktop MCP approval exceeds the viewport: ${JSON.stringify(desktopApprovalBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-mcp-approval.png`, fullPage: true });
  await approvalDialog.getByRole("button", { name: "本次允许", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".mcp-approval-dialog")?.textContent?.includes("断开会话"));
  await page.waitForFunction(() => document.activeElement?.textContent?.includes("拒绝"));
  await page.keyboard.press("Escape");
  await approvalDialog.waitFor({ state: "detached" });
  const approvalCalls = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "respond_mcp_approval")
    .map((call) => call.args));
  assert(JSON.stringify(approvalCalls) === JSON.stringify([
    { approvalId: approvalIds[0], approved: true },
    { approvalId: approvalIds[1], approved: false },
  ]), `MCP approval responses are wrong: ${JSON.stringify(approvalCalls)}`);

  const senderTab = page.locator('.workspace-dock[data-dock="bottom"] .workspace-dock-tab[data-panel="sender"]');
  const dataTransfer = await page.evaluateHandle(() => new DataTransfer());
  await senderTab.dispatchEvent("dragstart", { dataTransfer });
  const rightDropTarget = page.locator('[data-dock-target="right"]');
  await rightDropTarget.waitFor();
  await rightDropTarget.dispatchEvent("dragover", { dataTransfer });
  await rightDropTarget.dispatchEvent("drop", { dataTransfer });
  await page.waitForFunction(() => document.querySelector('.workspace-dock[data-dock="right"]')?.getAttribute("data-active-panel") === "sender");
  await page.waitForFunction(() => {
    const snapshot = JSON.parse(localStorage.getItem("portmate.workspacePanels.v2") || "null");
    return snapshot?.version === 6
      && JSON.stringify(snapshot.docks?.right) === JSON.stringify(["history", "sender"])
      && snapshot.docks?.bottom?.length === 0;
  });
  await dataTransfer.dispose();
  const movedDockLayout = await page.evaluate(() => {
    const snapshot = JSON.parse(localStorage.getItem("portmate.workspacePanels.v2"));
    return {
      dockCount: document.querySelectorAll(".workspace-dock").length,
      rightTabs: [...document.querySelectorAll('.workspace-dock[data-dock="right"] .workspace-dock-tab')]
        .map((tab) => tab.getAttribute("data-panel")),
      rightPanels: [...document.querySelectorAll('.workspace-dock[data-dock="right"] .workspace-dock-content')]
        .map((panel) => panel.getAttribute("data-panel")),
      bottomDock: document.querySelector('.workspace-dock[data-dock="bottom"]') !== null,
      docks: snapshot.docks,
    };
  });
  assert(movedDockLayout.dockCount === 2
    && JSON.stringify(movedDockLayout.rightTabs) === JSON.stringify(["history", "sender"])
    && JSON.stringify(movedDockLayout.rightPanels) === JSON.stringify(["history", "sender"])
    && !movedDockLayout.bottomDock
    && movedDockLayout.docks.active.right === "sender",
  `cross-dock drag did not keep both right-side windows visible: ${JSON.stringify(movedDockLayout)}`);

  await page.reload();
  await page.waitForSelector('.terminal-host[data-terminal-size] .xterm-screen');
  await page.locator('.workspace-dock[data-dock="right"][data-active-panel="sender"]').waitFor();
  const restoredLayout = await page.evaluate(() => ({
    rightTabs: [...document.querySelectorAll('.workspace-dock[data-dock="right"] .workspace-dock-tab')]
      .map((tab) => tab.getAttribute("data-panel")),
    bottomDock: document.querySelector('.workspace-dock[data-dock="bottom"]') !== null,
    leftWidth: document.querySelector('.workspace-dock[data-dock="left"]')?.getBoundingClientRect().width ?? 0,
    rightWidth: document.querySelector('.workspace-dock[data-dock="right"]')?.getBoundingClientRect().width ?? 0,
    sizes: JSON.parse(localStorage.getItem("portmate.workspacePanels.v2")).sizes,
  }));
  assert(JSON.stringify(restoredLayout.rightTabs) === JSON.stringify(["history", "sender"])
    && !restoredLayout.bottomDock
    && restoredLayout.leftWidth >= 419 && restoredLayout.leftWidth <= 421
    && restoredLayout.rightWidth >= 295 && restoredLayout.rightWidth <= 297
    && JSON.stringify(restoredLayout.sizes) === JSON.stringify({ left: 420, right: 296, bottom: 226 }),
  `dragged dock layout did not survive reload: ${JSON.stringify(restoredLayout)}`);

  await page.getByRole("separator", { name: "调整左侧停靠区宽度", exact: true }).dblclick();
  await page.waitForFunction(() => JSON.parse(
    localStorage.getItem("portmate.workspacePanels.v2") || "null",
  )?.sizes?.left === null);
  const resetDockLayout = await page.evaluate(() => {
    const layout = document.querySelector(".wind-layout");
    const dock = document.querySelector('.workspace-dock[data-dock="left"]');
    return {
      active: dock?.getAttribute("data-active-panel"),
      inlineSize: layout?.style.getPropertyValue("--workspace-left-size"),
      width: dock?.getBoundingClientRect().width ?? 0,
    };
  });
  assert(resetDockLayout.active === "explorer"
    && resetDockLayout.inlineSize === ""
    && resetDockLayout.width >= 359 && resetDockLayout.width <= 361,
  `double-click did not restore the default left dock size: ${JSON.stringify(resetDockLayout)}`);

  await togglePanel("资源管理器");
  await togglePanel("文件管理器");
  await togglePanel("历史命令");
  await togglePanel("发送");
  await page.waitForFunction(() => document.querySelectorAll(".workspace-dock").length === 0);
  const desktop = await page.evaluate(() => ({
    viewportWidth: innerWidth,
    documentWidth: document.documentElement.scrollWidth,
    viewportHeight: innerHeight,
    documentHeight: document.documentElement.scrollHeight,
    terminalWidth: document.querySelector(".center-workspace")?.getBoundingClientRect().width ?? 0,
    docks: document.querySelectorAll(".workspace-dock").length,
  }));
  assert(desktop.documentWidth <= desktop.viewportWidth && desktop.documentHeight <= desktop.viewportHeight,
    `desktop workspace overflow: ${JSON.stringify(desktop)}`);
  assert(desktop.terminalWidth > 1400 && desktop.docks === 0,
    `desktop did not return optional space to the terminal: ${JSON.stringify(desktop)}`);
  await page.screenshot({ path: `${screenshotPrefix}-desktop.png`, fullPage: true });

  await togglePanel("历史命令");
  await page.setViewportSize({ width: 390, height: 844 });
  const mobile = await page.evaluate(() => {
    const center = document.querySelector(".center-workspace")?.getBoundingClientRect();
    return {
      viewportWidth: innerWidth,
      documentWidth: document.documentElement.scrollWidth,
      viewportHeight: innerHeight,
      documentHeight: document.documentElement.scrollHeight,
      dockDisplay: getComputedStyle(document.querySelector(".workspace-dock")).display,
      sysmonSummaryDisplay: getComputedStyle(document.querySelector(".sysmon-applet-summary")).display,
      center: center ? { left: center.left, right: center.right, top: center.top, bottom: center.bottom } : null,
    };
  });
  assert(mobile.documentWidth <= mobile.viewportWidth && mobile.documentHeight <= mobile.viewportHeight,
    `mobile workspace overflow: ${JSON.stringify(mobile)}`);
  assert(mobile.dockDisplay === "none",
    `optional panels crowd the mobile terminal: ${JSON.stringify(mobile)}`);
  assert(mobile.sysmonSummaryDisplay === "none",
    `mobile Sysmon summary still consumes status-bar width: ${JSON.stringify(mobile)}`);
  assert(mobile.center?.left === 0 && mobile.center?.right === mobile.viewportWidth,
    `terminal does not fill the mobile viewport: ${JSON.stringify(mobile.center)}`);
  await page.screenshot({ path: `${screenshotPrefix}-mobile.png`, fullPage: true });

  await page.evaluate(() => {
    const now = Date.now();
    window.__emitTauriEvent("portmate-mcp-approval", {
      id: "33333333-3333-4333-8333-333333333333",
      clientId: "mobile-ops",
      action: "create_tunnel",
      sessionId: "edge-router",
      scope: "tunnel",
      createdAt: new Date(now).toISOString(),
      expiresAt: new Date(now + 60_000).toISOString(),
    });
  });
  const mobileApproval = page.getByRole("alertdialog", { name: "MCP 写操作审批", exact: true });
  await mobileApproval.waitFor();
  await page.waitForTimeout(180);
  const mobileApprovalBounds = await mobileApproval.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return {
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      scrollWidth: dialog.scrollWidth,
      width: rect.width,
    };
  });
  assert(mobileApprovalBounds.left >= 0 && mobileApprovalBounds.right <= mobile.viewportWidth
    && mobileApprovalBounds.top >= 0 && mobileApprovalBounds.bottom <= mobile.viewportHeight
    && mobileApprovalBounds.scrollWidth <= mobileApprovalBounds.width,
  `mobile MCP approval exceeds the viewport: ${JSON.stringify(mobileApprovalBounds)}`);
  assert(await mobileApproval.getByRole("button", { name: "拒绝", exact: true }).isVisible()
    && await mobileApproval.getByRole("button", { name: "本次允许", exact: true }).isVisible(),
  "mobile MCP approval actions are unreachable");
  await page.screenshot({ path: `${screenshotPrefix}-mcp-approval-mobile.png`, fullPage: true });
  await mobileApproval.getByRole("button", { name: "拒绝", exact: true }).click();
  await mobileApproval.waitFor({ state: "detached" });

  await page.getByRole("button", { name: "搜索会话", exact: true }).click();
  const mobileSearch = page.locator(".search-dialog");
  await mobileSearch.waitFor();
  const mobileSearchBounds = await mobileSearch.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
  });
  assert(mobileSearchBounds.left >= 0 && mobileSearchBounds.right <= mobile.viewportWidth
    && mobileSearchBounds.top >= 0 && mobileSearchBounds.bottom <= mobile.viewportHeight,
  `mobile search dialog exceeds the viewport: ${JSON.stringify(mobileSearchBounds)}`);
  assert(await page.getByRole("combobox", { name: "搜索会话和日志", exact: true }).isVisible(),
    "mobile session search has no reachable input");
  await page.screenshot({ path: `${screenshotPrefix}-search-mobile.png`, fullPage: true });
  await page.getByRole("combobox", { name: "搜索会话和日志", exact: true }).press("Escape");
  await mobileSearch.waitFor({ state: "detached" });

  await openNewSessionSettings();
  const mobileSessionSettingsBounds = await page.locator(".session-settings-dialog").evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom, scrollWidth: dialog.scrollWidth, width: rect.width };
  });
  assert(mobileSessionSettingsBounds.left >= 0 && mobileSessionSettingsBounds.right <= mobile.viewportWidth
    && mobileSessionSettingsBounds.top >= 0 && mobileSessionSettingsBounds.bottom <= mobile.viewportHeight
    && mobileSessionSettingsBounds.scrollWidth <= mobileSessionSettingsBounds.width,
  `mobile session settings exceed the viewport: ${JSON.stringify(mobileSessionSettingsBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-session-settings-mobile.png`, fullPage: true });
  await page.locator(".session-settings-dialog .dialog-actions button", { hasText: "取消" }).click();
  await page.locator(".session-settings-dialog").waitFor({ state: "detached" });

  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.locator(".menu-popover button", { hasText: "MCP Bridge" }).click();
  await page.locator(".mcp-dialog").waitFor();
  await page.getByRole("tab", { name: "审计", exact: true }).click();
  const mobileMcpBounds = await page.locator(".mcp-dialog").evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return {
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      scrollWidth: dialog.scrollWidth,
      width: rect.width,
      scrollHeight: dialog.scrollHeight,
      height: rect.height,
    };
  });
  assert(mobileMcpBounds.left >= 0 && mobileMcpBounds.right <= mobile.viewportWidth
    && mobileMcpBounds.top >= 0 && mobileMcpBounds.bottom <= mobile.viewportHeight
    && mobileMcpBounds.scrollWidth <= mobileMcpBounds.width
    && mobileMcpBounds.scrollHeight <= mobileMcpBounds.height,
  `mobile MCP audit workspace exceeds the viewport: ${JSON.stringify(mobileMcpBounds)}`);
  assert(await page.locator(".mcp-audit-list").isVisible()
    && await page.locator(".mcp-audit-inspector").isVisible(),
  "mobile MCP audit list or inspector is unreachable");
  await page.screenshot({ path: `${screenshotPrefix}-mcp-audit-mobile.png`, fullPage: true });
  await page.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
  await page.locator(".mcp-dialog").waitFor({ state: "detached" });

  await page.setViewportSize({ width: 1440, height: 900 });
  await togglePanel("资源管理器");
  const deleteTarget = page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" });
  await deleteTarget.click();
  await page.locator(".workspace-pane-tab", { hasText: "Bench UART" }).waitFor();
  await deleteTarget.click({ button: "right" });
  const deleteAction = page.locator(".context-menu-row", { hasText: "删除会话 Profile" });
  await deleteAction.waitFor();
  await page.screenshot({ path: `${screenshotPrefix}-profile-delete.png`, fullPage: true });
  let deletionPrompt = "";
  page.once("dialog", async (dialog) => {
    deletionPrompt = dialog.message();
    await dialog.accept();
  });
  await deleteAction.click();
  const deletionNotice = page.locator(".notice-dialog", { hasText: "会话已删除" });
  await deletionNotice.waitFor();
  assert(deletionPrompt.includes("Bench UART") && deletionPrompt.includes("磁盘日志分片与安全审计保留"),
    `profile deletion confirmation omitted its target or retention boundary: ${deletionPrompt}`);
  assert(await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" }).count() === 0,
    "deleted profile remained in the resource explorer");
  assert(await page.locator(".workspace-pane-tab", { hasText: "Bench UART" }).count() === 0,
    "deleted profile retained a workspace view");
  await page.waitForFunction(() => !localStorage.getItem("portmate.workspace.v1")?.includes("bench-uart"));
  const deleteCalls = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "delete_session_profile"));
  assert(deleteCalls.length === 1 && deleteCalls[0].args.sessionId === "bench-uart",
    `profile deletion did not reach the backend exactly once: ${JSON.stringify(deleteCalls)}`);
  await deletionNotice.getByRole("button", { name: "确定", exact: true }).click();

  const terminalWrites = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "send_text" || call.command === "send_bytes" || call.command === "run_command"
  )));
  assert(terminalWrites.length === 0,
    `non-input workspace actions wrote to the terminal: ${JSON.stringify(terminalWrites)}`);
  assert(pageErrors.length === 0, `browser exceptions: ${JSON.stringify(pageErrors)}`);

  console.log(JSON.stringify({
    migratedPanels: initial.panels,
    filters: ["resource tag/endpoint", "normalized history"],
    contextMenu: "single synchronized-input, split, move, merge, swap, zoom, detach, close-pane, and profile-delete actions",
    transferProtocols: {
      ssh: sshTransferOptions.map((option) => option.value),
      serial: serialTransferOptions,
    },
    startupSettings,
    sessionSettings: {
      pages: expectedSessionPages,
      preferenceKeys: sessionPreferenceKeys,
      desktop: sessionSettingsBounds,
      mobile: mobileSessionSettingsBounds,
    },
    terminalTheme: lightThemeState,
    detachedTheme: detachedThemeState,
    mcp: {
      tabs: mcpTabs,
      exportedRecordIds: exportCall.args.request.recordIds,
      approvalResponses: approvalCalls,
      approvalDesktop: desktopApprovalBounds,
      approvalMobile: mobileApprovalBounds,
      mobile: mobileMcpBounds,
    },
    dockResize: {
      resized: resizedDockLayout,
      restored: restoredLayout,
      reset: resetDockLayout,
    },
    terminalWrites,
    desktop,
    mobile,
    screenshots: [
      `${screenshotPrefix}-search.png`,
      `${screenshotPrefix}-search-mobile.png`,
      `${screenshotPrefix}-view-split-drop.png`,
      `${screenshotPrefix}-settings.png`,
      `${screenshotPrefix}-transfer.png`,
      `${screenshotPrefix}-file-manager.png`,
      `${screenshotPrefix}-sender.png`,
      `${screenshotPrefix}-session-settings.png`,
      `${screenshotPrefix}-session-settings-mobile.png`,
      `${screenshotPrefix}-terminal-theme-settings.png`,
      `${screenshotPrefix}-terminal-light-theme.png`,
      `${screenshotPrefix}-detached-theme.png`,
      `${screenshotPrefix}-mcp-grants.png`,
      `${screenshotPrefix}-mcp-audit.png`,
      `${screenshotPrefix}-mcp-audit-mobile.png`,
      `${screenshotPrefix}-mcp-approval.png`,
      `${screenshotPrefix}-mcp-approval-mobile.png`,
      `${screenshotPrefix}-profile-delete.png`,
      `${screenshotPrefix}-desktop.png`,
      `${screenshotPrefix}-mobile.png`,
    ],
  }, null, 2));
  await context.close();
} finally {
  await browser?.close().catch(() => {});
  vite.kill("SIGTERM");
  await new Promise((resolve) => {
    if (vite.exitCode !== null) resolve();
    else {
      let settled = false;
      const finish = () => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        resolve();
      };
      vite.once("exit", finish);
      const timer = setTimeout(finish, 2000);
    }
  });
}
