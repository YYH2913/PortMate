import { spawn } from "node:child_process";
import { createServer } from "node:net";
import process from "node:process";
import { chromium } from "playwright-core";

const chromeExecutable = process.env.PORTMATE_CHROME ?? "/usr/bin/google-chrome";
const screenshotPrefix = process.env.PORTMATE_TMUX_SCREENSHOT_PREFIX ?? "/tmp/portmate-tmux-workflow";

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

const now = "2026-07-15T00:00:00.000Z";
const session = {
  profile: {
    id: "ssh-tmux",
    name: "Tmux Lab",
    kind: "ssh",
    group: "Compatibility",
    tags: ["tmux"],
    connection: {
      kind: "ssh",
      endpoint: { host: "192.0.2.20", port: 22 },
      username: "operator",
      reconnect: false,
      reconnectDelayMs: 1000,
      keepaliveEnabled: true,
      keepaliveIntervalSeconds: 30,
      keepaliveMaxMissed: 3,
      proxy: { enabled: false, kind: "socks5", host: "", port: 1080, username: "" },
      hostKeyPolicy: { mode: "strict", alias: "tmux-lab", trustScope: "profile", allowRotation: false, checkIp: false },
      trustedHostKeys: [],
      identityPolicy: { identitiesOnly: true, authOrder: ["public-key"], recordSuccess: true, lastSuccessful: "public-key" },
      identityRefs: [],
      agentPolicy: { enabled: false, forwarding: false, offerMode: "disabled" },
      jumps: [],
      tunnels: [],
    },
    terminal: { term: "xterm-256color", rows: 32, cols: 120, scrollback: 200000, fontFamily: "JetBrains Mono, monospace", fontSize: 13, theme: "portmate-dark" },
    logging: { enabled: false, raw: false, text: true, jsonl: true, redactSecrets: true, pathTemplate: "{profile}/{date}/{session}.jsonl", retentionDays: 0 },
    triggers: [],
    transfer: { sftp: true, scp: true, xmodem: true, ymodem: true, zmodem: true, rateLimitBytesPerSecond: null, defaultLocalDir: null },
  },
  runtime: {
    sessionId: "ssh-tmux",
    paneId: "ssh-tmux:main",
    status: "connected",
    title: "Tmux Lab",
    cwd: "/home/operator",
    connectedSince: now,
    lastActivity: now,
    lastDisconnect: null,
    lastDisconnectReason: null,
    activeTransport: "ssh",
  },
  logLines: 1,
  lastLine: "operator@lab:~$ ",
};
const workspace = {
  version: 4,
  root: {
    kind: "pane",
    id: "pane-tmux",
    activeViewId: "view-tmux",
    views: [{ id: "view-tmux", sessionId: "ssh-tmux", title: "Tmux Lab", color: "#008B8B", keyMode: "remote" }],
  },
  activePaneId: "pane-tmux",
  activeId: "ssh-tmux",
  tabColors: {},
};
const initialTmuxState = {
  sessions: [
    { name: "lab", windows: 2, attached: 1, created: now },
    { name: "build", windows: 1, attached: 0, created: null },
  ],
  windows: [
    { session: "lab", windowIndex: 0, windowId: "@1", name: "editor", panes: 2, active: true, synchronized: false },
    { session: "lab", windowIndex: 1, windowId: "@2", name: "shell", panes: 1, active: false, synchronized: true },
    { session: "build", windowIndex: 0, windowId: "@3", name: "compile", panes: 1, active: true, synchronized: false },
  ],
  panes: [
    { session: "lab", windowIndex: 0, paneIndex: 1, paneId: "%2", active: false, synchronized: false, command: "tail", title: "logs" },
    { session: "lab", windowIndex: 0, paneIndex: 0, paneId: "%1", active: true, synchronized: false, command: "vim", title: "editor" },
    { session: "lab", windowIndex: 1, paneIndex: 0, paneId: "%3", active: true, synchronized: true, command: "bash", title: "shell" },
    { session: "build", windowIndex: 0, paneIndex: 0, paneId: "%4", active: true, synchronized: false, command: "cargo", title: "compile" },
  ],
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
  await context.addInitScript(({ initialSession, initialWorkspace, tmuxState }) => {
    localStorage.clear();
    localStorage.setItem("portmate.workspace.v1", JSON.stringify(initialWorkspace));
    localStorage.setItem("portmate.terminalPrefs", JSON.stringify({
      startupMode: "none",
      startupSessions: [],
      lockOnIdle: false,
      requireMasterPassword: false,
      oneKeyCompletionEnabled: false,
    }));
    window.__invokeCalls = [];
    window.__tmuxState = structuredClone(tmuxState);
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = {
      invoke: async (command, args = {}) => {
        window.__invokeCalls.push({ command, args });
        if (command === "list_sessions") return [initialSession];
        if (command === "tail_log") return [];
        if (command === "list_tmux_state") return structuredClone(window.__tmuxState);
        if (command === "set_tmux_pane_sync") {
          if (args.target === "lab:1" && args.enabled === false) throw new Error("tmux permission denied");
          window.__tmuxState = {
            ...window.__tmuxState,
            windows: window.__tmuxState.windows.map((item) => (
              `${item.session}:${item.windowIndex}` === args.target
                ? { ...item, synchronized: args.enabled }
                : item
            )),
            panes: window.__tmuxState.panes.map((pane) => (
              `${pane.session}:${pane.windowIndex}` === args.target
                ? { ...pane, synchronized: args.enabled }
                : pane
            )),
          };
          return structuredClone(window.__tmuxState);
        }
        if (command === "mutate_tmux") {
          const request = args.request;
          if (request.action === "rename-session") {
            window.__tmuxState = {
              sessions: window.__tmuxState.sessions.map((item) => item.name === request.target ? { ...item, name: request.name } : item),
              windows: window.__tmuxState.windows.map((item) => item.session === request.target ? { ...item, session: request.name } : item),
              panes: window.__tmuxState.panes.map((item) => item.session === request.target ? { ...item, session: request.name } : item),
            };
          } else if (request.action === "new-window") {
            const indexes = window.__tmuxState.windows.filter((item) => item.session === request.target).map((item) => item.windowIndex);
            const windowIndex = indexes.length ? Math.max(...indexes) + 1 : 0;
            const windowId = `@${window.__tmuxState.windows.length + 1}`;
            const paneId = `%${window.__tmuxState.panes.length + 1}`;
            window.__tmuxState = {
              sessions: window.__tmuxState.sessions.map((item) => item.name === request.target ? { ...item, windows: item.windows + 1 } : item),
              windows: [...window.__tmuxState.windows, {
                session: request.target, windowIndex, windowId, name: request.name || `window-${windowIndex}`,
                panes: 1, active: false, synchronized: false,
              }],
              panes: [...window.__tmuxState.panes, {
                session: request.target, windowIndex, paneIndex: 0, paneId, active: true,
                synchronized: false, command: "bash", title: request.name || `window-${windowIndex}`,
              }],
            };
          } else if (request.action === "rename-window") {
            window.__tmuxState = {
              ...window.__tmuxState,
              windows: window.__tmuxState.windows.map((item) => (
                `${item.session}:${item.windowIndex}` === request.target ? { ...item, name: request.name } : item
              )),
            };
          } else if (request.action === "kill-window") {
            const removed = window.__tmuxState.windows.find((item) => `${item.session}:${item.windowIndex}` === request.target);
            window.__tmuxState = {
              sessions: window.__tmuxState.sessions.map((item) => item.name === removed?.session ? { ...item, windows: Math.max(0, item.windows - 1) } : item),
              windows: window.__tmuxState.windows.filter((item) => `${item.session}:${item.windowIndex}` !== request.target),
              panes: window.__tmuxState.panes.filter((item) => `${item.session}:${item.windowIndex}` !== request.target),
            };
          } else if (request.action === "kill-session") {
            window.__tmuxState = {
              sessions: window.__tmuxState.sessions.filter((item) => item.name !== request.target),
              windows: window.__tmuxState.windows.filter((item) => item.session !== request.target),
              panes: window.__tmuxState.panes.filter((item) => item.session !== request.target),
            };
          } else if (request.action === "kill-pane") {
            const source = window.__tmuxState.panes.find((item) => item.paneId === request.target);
            if (source) {
              window.__tmuxState = {
                ...window.__tmuxState,
                windows: window.__tmuxState.windows.map((item) => (
                  item.session === source.session && item.windowIndex === source.windowIndex
                    ? { ...item, panes: Math.max(0, item.panes - 1) }
                    : item
                )),
                panes: window.__tmuxState.panes.filter((item) => item.paneId !== request.target),
              };
            }
          } else if (request.action === "split-pane-horizontal" || request.action === "split-pane-vertical") {
            const source = window.__tmuxState.panes.find((item) => item.paneId === request.target);
            if (source) {
              const indexes = window.__tmuxState.panes
                .filter((item) => item.session === source.session && item.windowIndex === source.windowIndex)
                .map((item) => item.paneIndex);
              const paneIndex = indexes.length ? Math.max(...indexes) + 1 : 0;
              const paneId = `%${window.__tmuxState.panes.length + 1}`;
              window.__tmuxState = {
                ...window.__tmuxState,
                windows: window.__tmuxState.windows.map((item) => (
                  item.session === source.session && item.windowIndex === source.windowIndex
                    ? { ...item, panes: item.panes + 1 }
                    : item
                )),
                panes: [...window.__tmuxState.panes, {
                  ...source,
                  paneIndex,
                  paneId,
                  active: false,
                  command: "bash",
                  title: request.action === "split-pane-horizontal" ? "horizontal split" : "vertical split",
                }],
              };
            }
          } else if (request.action === "swap-pane-previous" || request.action === "swap-pane-next") {
            const source = window.__tmuxState.panes.find((item) => item.paneId === request.target);
            if (source) {
              const siblings = window.__tmuxState.panes
                .filter((item) => item.session === source.session && item.windowIndex === source.windowIndex)
                .sort((left, right) => left.paneIndex - right.paneIndex);
              const sourceIndex = siblings.findIndex((item) => item.paneId === source.paneId);
              const offset = request.action === "swap-pane-previous" ? -1 : 1;
              const neighbor = siblings[(sourceIndex + offset + siblings.length) % siblings.length];
              if (neighbor && neighbor.paneId !== source.paneId) {
                window.__tmuxState = {
                  ...window.__tmuxState,
                  panes: window.__tmuxState.panes.map((item) => {
                    if (item.paneId === source.paneId) return { ...item, paneIndex: neighbor.paneIndex };
                    if (item.paneId === neighbor.paneId) return { ...item, paneIndex: source.paneIndex };
                    return item;
                  }),
                };
              }
            }
          }
          return structuredClone(window.__tmuxState);
        }
        if (command === "attach_tmux") return null;
        if (command === "list_host_keys") return { keys: [] };
        if (["list_files", "list_transfers", "list_mcp_audit", "list_mcp_grants", "list_serial_ports", "list_one_keys"].includes(command)) return [];
        if (command.startsWith("plugin:event|")) return 1;
        return null;
      },
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      transformCallback: () => 1,
      unregisterCallback: () => {},
      convertFileSrc: (path) => path,
    };
  }, { initialSession: session, initialWorkspace: workspace, tmuxState: initialTmuxState });

  const page = await context.newPage();
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(appUrl);
  await page.getByRole("button", { name: "工具", exact: true }).click();
  await page.getByRole("button", { name: "Tmux", exact: true }).click();
  await page.getByRole("heading", { name: "窗口与窗格" }).waitFor();

  const labZero = page.locator('[data-tmux-target="lab:0"]');
  const labOne = page.locator('[data-tmux-target="lab:1"]');
  const labZeroSwitch = labZero.getByRole("switch");
  const labOneSwitch = labOne.getByRole("switch");
  assert(await labZero.locator(".tmux-window-panes > div").count() === 2, "lab:0 must contain two pane rows");
  assert(await labZero.locator(".tmux-window-panes > div strong").allTextContents().then((values) => values.join(",")) === "lab:0.0,lab:0.1", "pane rows must be sorted by pane index");
  assert(await labZeroSwitch.isChecked() === false, "lab:0 must initially be unsynchronized");
  assert(await labOneSwitch.isChecked() === true, "lab:1 must initially be synchronized");

  await labZeroSwitch.check();
  await page.getByText("lab:0 已开启 pane 同步输入", { exact: true }).waitFor();
  assert(await labZeroSwitch.isChecked(), "lab:0 switch did not reflect the confirmed enabled state");
  const enabledCall = await page.evaluate(() => window.__invokeCalls.findLast((call) => call.command === "set_tmux_pane_sync"));
  assert(enabledCall?.args?.sessionId === "ssh-tmux" && enabledCall.args.target === "lab:0" && enabledCall.args.enabled === true,
    `invalid enable request: ${JSON.stringify(enabledCall)}`);

  await labZeroSwitch.uncheck();
  await page.getByText("lab:0 已关闭 pane 同步输入", { exact: true }).waitFor();
  assert(await labZeroSwitch.isChecked() === false, "lab:0 switch did not reflect the confirmed disabled state");

  await labOneSwitch.click();
  await page.getByText("tmux permission denied", { exact: true }).waitFor();
  assert(await labOneSwitch.isChecked(), "failed remote mutation must preserve the last confirmed state");

  await page.getByRole("button", { name: "刷新", exact: true }).click();
  await page.waitForFunction(() => window.__invokeCalls.filter((call) => call.command === "list_tmux_state").length >= 2);

  await page.getByRole("combobox", { name: "lab:0 window 布局", exact: true }).selectOption("tiled");
  await page.getByText("lab:0 已应用 tiled 布局", { exact: true }).waitFor();

  await page.getByRole("combobox", { name: "lab:0.0 pane 分割", exact: true }).selectOption("split-pane-horizontal");
  await page.getByText("lab:0.0 已左右分割", { exact: true }).waitFor();
  assert(await labZero.locator(".tmux-window-panes > div").count() === 3, "split pane did not refresh the remote snapshot");

  const paneOne = page.locator('[data-tmux-pane="%1"]');
  await paneOne.getByRole("combobox", { name: /pane 交换$/ }).selectOption("swap-pane-next");
  await page.getByText("lab:0.0 已与后一 pane 交换", { exact: true }).waitFor();
  assert((await paneOne.locator("strong").textContent()) === "lab:0.1", "swapped pane index was not refreshed");

  await paneOne.getByRole("combobox", { name: /pane 调整尺寸$/ }).selectOption("resize-pane-right");
  await page.getByText("lab:0.1 已向右调整 5 cells", { exact: true }).waitFor();

  await page.getByRole("button", { name: "关闭 pane lab:0.2", exact: true }).click();
  await page.getByText("关闭 pane lab:0.2？", { exact: true }).waitFor();
  await page.getByRole("button", { name: "确认关闭", exact: true }).click();
  await page.getByText("lab:0.2 已关闭", { exact: true }).waitFor();
  assert(await labZero.locator(".tmux-window-panes > div").count() === 2, "closed pane remains visible");

  await page.getByRole("button", { name: "重命名 session build", exact: true }).click();
  const sessionNameInput = page.getByRole("textbox", { name: "新名称 build" });
  await sessionNameInput.fill("build-renamed");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await page.getByText("build 已重命名为 build-renamed", { exact: true }).waitFor();
  assert(await page.locator('[data-tmux-session="build-renamed"]').count() === 1, "renamed session is missing");

  await page.getByRole("button", { name: "在 lab 新建 window", exact: true }).click();
  await page.getByRole("textbox", { name: "新 window 名称 lab" }).fill("ops");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await page.getByText("lab 已新建 window", { exact: true }).waitFor();
  assert(await page.locator('[data-tmux-target="lab:2"]').count() === 1, "new window is missing");

  await page.getByRole("button", { name: "重命名 window lab:1", exact: true }).click();
  await page.getByRole("textbox", { name: "新名称 lab:1" }).fill("metrics");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await page.getByText("lab:1 已重命名为 metrics", { exact: true }).waitFor();
  assert((await page.locator('[data-tmux-target="lab:1"] > header small').textContent())?.includes("metrics"), "renamed window metadata is missing");

  await page.getByRole("button", { name: "关闭 window lab:1", exact: true }).click();
  await page.getByText("关闭 window lab:1？", { exact: true }).waitFor();
  await page.getByRole("button", { name: "取消", exact: true }).click();
  assert(await page.locator('[data-tmux-target="lab:1"]').count() === 1, "cancelled window deletion changed state");

  await page.getByRole("button", { name: "关闭 window lab:2", exact: true }).click();
  await page.getByText("关闭 window lab:2？", { exact: true }).waitFor();
  await page.getByRole("button", { name: "确认关闭", exact: true }).click();
  await page.getByText("lab:2 已关闭", { exact: true }).waitFor();
  assert(await page.locator('[data-tmux-target="lab:2"]').count() === 0, "closed window remains visible");

  await page.getByRole("button", { name: "关闭 session build-renamed", exact: true }).click();
  await page.getByText("关闭 session build-renamed 及其全部 window？", { exact: true }).waitFor();
  await page.getByRole("button", { name: "确认关闭", exact: true }).click();
  await page.getByText("build-renamed 已关闭", { exact: true }).waitFor();
  assert(await page.locator('[data-tmux-session="build-renamed"]').count() === 0, "closed session remains visible");

  const mutationCalls = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "mutate_tmux"));
  assert(mutationCalls.map((call) => call.args.request.action).join(",") === "select-layout,split-pane-horizontal,swap-pane-next,resize-pane-right,kill-pane,rename-session,new-window,rename-window,kill-window,kill-session",
    `unexpected Tmux mutation sequence: ${JSON.stringify(mutationCalls)}`);
  assert(mutationCalls[0].args.request.target === "lab:0" && mutationCalls[0].args.request.layout === "tiled",
    `invalid layout request: ${JSON.stringify(mutationCalls[0])}`);
  assert(mutationCalls[1].args.request.target === "%1", `invalid split request: ${JSON.stringify(mutationCalls[1])}`);
  assert(mutationCalls[2].args.request.target === "%1", `invalid swap request: ${JSON.stringify(mutationCalls[2])}`);
  assert(mutationCalls[3].args.request.target === "%1" && mutationCalls[3].args.request.amount === 5,
    `invalid resize request: ${JSON.stringify(mutationCalls[3])}`);
  assert(mutationCalls[4].args.request.target === "%5", `invalid pane close request: ${JSON.stringify(mutationCalls[4])}`);
  await page.evaluate(() => {
    document.querySelectorAll(".tmux-list, .tmux-window-list").forEach((element) => { element.scrollTop = 0; });
  });
  await page.screenshot({ path: `${screenshotPrefix}-desktop.png`, fullPage: true });
  const desktop = await page.evaluate(() => {
    const dialog = document.querySelector(".tmux-dialog").getBoundingClientRect();
    return { innerWidth, documentWidth: document.documentElement.scrollWidth, dialog: { left: dialog.left, right: dialog.right, top: dialog.top, bottom: dialog.bottom } };
  });
  assert(desktop.documentWidth <= desktop.innerWidth && desktop.dialog.left >= 0 && desktop.dialog.right <= desktop.innerWidth,
    `invalid desktop tmux layout: ${JSON.stringify(desktop)}`);

  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: `${screenshotPrefix}-mobile.png`, fullPage: true });
  const mobile = await page.evaluate(() => {
    const dialog = document.querySelector(".tmux-dialog").getBoundingClientRect();
    const toolbar = document.querySelector(".tmux-toolbar").getBoundingClientRect();
    return {
      innerWidth,
      innerHeight,
      documentWidth: document.documentElement.scrollWidth,
      dialog: { left: dialog.left, right: dialog.right, top: dialog.top, bottom: dialog.bottom },
      toolbar: { left: toolbar.left, right: toolbar.right, top: toolbar.top, bottom: toolbar.bottom },
    };
  });
  assert(mobile.documentWidth <= mobile.innerWidth
    && mobile.dialog.left >= 0 && mobile.dialog.right <= mobile.innerWidth
    && mobile.dialog.top >= 0 && mobile.dialog.bottom <= mobile.innerHeight
    && mobile.toolbar.left >= mobile.dialog.left && mobile.toolbar.right <= mobile.dialog.right,
  `invalid mobile tmux layout: ${JSON.stringify(mobile)}`);

  const targetInput = page.getByPlaceholder("session name");
  await targetInput.fill("new-lab");
  await page.getByRole("button", { name: "附着/新建", exact: true }).click();
  await page.waitForFunction(() => window.__invokeCalls.some((call) => call.command === "attach_tmux" && call.args.target === "new-lab"));
  const attachCall = await page.evaluate(() => window.__invokeCalls.findLast((call) => call.command === "attach_tmux"));
  assert(attachCall?.args?.sessionId === "ssh-tmux", `invalid attach request: ${JSON.stringify(attachCall)}`);
  assert(pageErrors.length === 0, `browser exceptions: ${JSON.stringify(pageErrors)}`);

  console.log(JSON.stringify({
    enabledCall,
    mutationCalls,
    attachCall,
    desktop,
    mobile,
    screenshots: [`${screenshotPrefix}-desktop.png`, `${screenshotPrefix}-mobile.png`],
  }, null, 2));
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
