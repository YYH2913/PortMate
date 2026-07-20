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
    window.__tmuxControlRuntimes = {};
    window.__tmuxControlSequence = 0;
    window.__tauriCallbacks = new Map();
    window.__tauriEventListeners = new Map();
    window.__tauriCallbackId = 0;
    window.__emitTauriEvent = (event, payload) => {
      const listeners = window.__tauriEventListeners.get(event) || [];
      for (const id of listeners) {
        window.__tauriCallbacks.get(id)?.({ event, id, payload });
      }
    };
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
        if (command === "list_sessions") return [initialSession];
        if (command === "tail_log") return [];
        if (command === "list_tmux_state") return structuredClone(window.__tmuxState);
        if (command === "start_tmux_control") {
          window.__tmuxControlSequence += 1;
          const runtimeId = `control-${window.__tmuxControlSequence}`;
          window.__tmuxControlRuntimes[args.target] = runtimeId;
          return {
            sessionId: args.sessionId,
            target: args.target,
            active: true,
            runtimeId,
          };
        }
        if (command === "stop_tmux_control") {
          const targets = args.target ? [args.target] : Object.keys(window.__tmuxControlRuntimes);
          const target = targets.length === 1 ? targets[0] : "";
          const runtimeId = target ? window.__tmuxControlRuntimes[target] || null : null;
          for (const item of targets) delete window.__tmuxControlRuntimes[item];
          return { sessionId: args.sessionId, target, active: false, runtimeId };
        }
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
          if (request.action === "move-pane-vertical" && request.destination === "build:0") {
            throw new Error("tmux move denied");
          }
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
          } else if (request.action === "select-pane") {
            const source = window.__tmuxState.panes.find((item) => item.paneId === request.target);
            if (source) {
              window.__tmuxState = {
                ...window.__tmuxState,
                panes: window.__tmuxState.panes.map((item) => (
                  item.session === source.session && item.windowIndex === source.windowIndex
                    ? { ...item, active: item.paneId === source.paneId }
                    : item
                )),
              };
            }
          } else if (request.action === "break-pane") {
            const source = window.__tmuxState.panes.find((item) => item.paneId === request.target);
            const sourceWindow = source && window.__tmuxState.windows.find((item) => (
              item.session === source.session && item.windowIndex === source.windowIndex
            ));
            if (source && sourceWindow && sourceWindow.panes > 1) {
              const indexes = window.__tmuxState.windows
                .filter((item) => item.session === source.session)
                .map((item) => item.windowIndex);
              const windowIndex = indexes.length ? Math.max(...indexes) + 1 : 0;
              const windowId = `@${window.__tmuxState.windows.length + 1}`;
              window.__tmuxState = {
                sessions: window.__tmuxState.sessions.map((item) => (
                  item.name === source.session ? { ...item, windows: item.windows + 1 } : item
                )),
                windows: [
                  ...window.__tmuxState.windows.map((item) => item.windowId === sourceWindow.windowId
                    ? { ...item, panes: item.panes - 1 }
                    : item),
                  {
                    session: source.session, windowIndex, windowId, name: `window-${windowIndex}`,
                    panes: 1, active: false, synchronized: source.synchronized,
                  },
                ],
                panes: window.__tmuxState.panes.map((item) => item.paneId === source.paneId
                  ? { ...item, windowIndex, paneIndex: 0, active: true }
                  : item),
              };
            }
          } else if (request.action === "move-pane-horizontal" || request.action === "move-pane-vertical") {
            const source = window.__tmuxState.panes.find((item) => item.paneId === request.target);
            const sourceWindow = source && window.__tmuxState.windows.find((item) => (
              item.session === source.session && item.windowIndex === source.windowIndex
            ));
            const destinationWindow = window.__tmuxState.windows.find((item) => (
              `${item.session}:${item.windowIndex}` === request.destination
            ));
            if (source && sourceWindow && destinationWindow && sourceWindow.windowId !== destinationWindow.windowId) {
              const destinationIndexes = window.__tmuxState.panes
                .filter((item) => item.session === destinationWindow.session && item.windowIndex === destinationWindow.windowIndex)
                .map((item) => item.paneIndex);
              const paneIndex = destinationIndexes.length ? Math.max(...destinationIndexes) + 1 : 0;
              const removesSourceWindow = sourceWindow.panes === 1;
              window.__tmuxState = {
                sessions: window.__tmuxState.sessions.map((item) => (
                  removesSourceWindow && item.name === sourceWindow.session
                    ? { ...item, windows: Math.max(0, item.windows - 1) }
                    : item
                )),
                windows: window.__tmuxState.windows
                  .filter((item) => !removesSourceWindow || item.windowId !== sourceWindow.windowId)
                  .map((item) => {
                    if (item.windowId === destinationWindow.windowId) return { ...item, panes: item.panes + 1 };
                    if (item.windowId === sourceWindow.windowId) return { ...item, panes: item.panes - 1 };
                    return item;
                  }),
                panes: window.__tmuxState.panes.map((item) => item.paneId === source.paneId
                  ? {
                    ...item,
                    session: destinationWindow.session,
                    windowIndex: destinationWindow.windowIndex,
                    paneIndex,
                    active: false,
                  }
                  : item),
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
        if (command.startsWith("plugin:event|")) return null;
        return null;
      },
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      transformCallback: (callback) => {
        window.__tauriCallbackId += 1;
        window.__tauriCallbacks.set(window.__tauriCallbackId, callback);
        return window.__tauriCallbackId;
      },
      unregisterCallback: (id) => window.__tauriCallbacks.delete(id),
      convertFileSrc: (path) => path,
    };
  }, { initialSession: session, initialWorkspace: workspace, tmuxState: initialTmuxState });

  const page = await context.newPage();
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(appUrl);
  const toolsMenu = page.getByRole("button", { name: "工具", exact: true });
  try {
    await toolsMenu.waitFor({ state: "visible" });
  } catch (error) {
    const body = await page.locator("body").innerText().catch(() => "<body unavailable>");
    throw new Error(`tmux workspace did not become ready: ${error.message}\npage errors: ${JSON.stringify(pageErrors)}\nbody: ${body.slice(0, 2_000)}\nvite: ${viteOutput.slice(-4_000)}`);
  }
  await toolsMenu.click();
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

  await page.waitForFunction(() => (window.__tauriEventListeners.get("portmate-tmux-control-event") || []).length > 0);
  await page.getByRole("button", { name: "实时监听 session lab", exact: true }).click();
  await page.getByText("lab 已开启 control-mode 实时监听", { exact: true }).waitFor();
  const activeControlButton = page.getByRole("button", { name: "停止实时监听 session lab", exact: true });
  assert(await activeControlButton.getAttribute("aria-pressed") === "true", "control-mode button did not become active");
  await page.getByRole("button", { name: "实时监听 session build", exact: true }).click();
  await page.getByText("build 已开启 control-mode 实时监听", { exact: true }).waitFor();
  const buildControlButton = page.getByRole("button", { name: "停止实时监听 session build", exact: true });
  assert(await activeControlButton.getAttribute("aria-pressed") === "true"
    && await buildControlButton.getAttribute("aria-pressed") === "true",
  "starting build replaced the active lab control runtime");

  await page.getByRole("button", { name: "重命名 session build", exact: true }).click();
  const draftSessionName = page.getByRole("textbox", { name: "新名称 build" });
  await draftSessionName.fill("build-draft");
  await activeControlButton.click();
  await page.getByText("lab 已停止 control-mode 实时监听", { exact: true }).waitFor();
  await page.getByRole("button", { name: "实时监听 session lab", exact: true }).click();
  await page.getByText("lab 已开启 control-mode 实时监听", { exact: true }).waitFor();
  assert(await draftSessionName.inputValue() === "build-draft",
    "restarting lab control discarded the independent build editor");
  const listCallsBeforeControlEvent = await page.evaluate(() => (
    window.__invokeCalls.filter((call) => call.command === "list_tmux_state").length
  ));
  await page.evaluate(() => {
    window.__tmuxState = {
      ...window.__tmuxState,
      windows: window.__tmuxState.windows.map((item) => (
        item.session === "lab" && item.windowIndex === 0 ? { ...item, name: "external-editor" } : item
      )),
    };
    window.__emitTauriEvent("portmate-tmux-control-event", {
      sessionId: "ssh-tmux",
      target: "lab",
      kind: "state-changed",
      active: true,
      runtimeId: window.__tmuxControlRuntimes.lab,
      protocolEvent: "window-renamed",
      error: null,
    });
  });
  await page.waitForFunction((previous) => (
    window.__invokeCalls.filter((call) => call.command === "list_tmux_state").length > previous
  ), listCallsBeforeControlEvent);
  await page.locator('[data-tmux-target="lab:0"] > header small').getByText(/external-editor/).waitFor();
  const draftCountAfterRefresh = await draftSessionName.count();
  assert(draftCountAfterRefresh === 1, "control-mode refresh discarded the inline editor");
  assert(await draftSessionName.inputValue() === "build-draft", "control-mode refresh changed the inline editor");
  await page.evaluate(() => {
    window.__emitTauriEvent("portmate-tmux-control-event", {
      sessionId: "ssh-tmux",
      target: "lab",
      kind: "stopped",
      active: false,
      runtimeId: "stale-control-runtime",
      protocolEvent: null,
      error: null,
    });
  });
  await page.getByRole("button", { name: "取消", exact: true }).click();
  const activeControlCounts = await page.locator(".tmux-control-active").evaluateAll((buttons) => (
    buttons.map((button) => button.getAttribute("aria-label"))
  ));
  assert(activeControlCounts.includes("停止实时监听 session lab"), "stale watcher stop cleared the active runtime");
  assert(activeControlCounts.includes("停止实时监听 session build"), "stale lab stop cleared the build runtime");
  await activeControlButton.click();
  await page.getByText("lab 已停止 control-mode 实时监听", { exact: true }).waitFor();
  assert(await buildControlButton.getAttribute("aria-pressed") === "true",
    "stopping lab also stopped the independent build runtime");
  await buildControlButton.click();
  await page.getByText("build 已停止 control-mode 实时监听", { exact: true }).waitFor();
  await page.getByRole("button", { name: "实时监听 session lab", exact: true }).click();
  await page.getByText("lab 已开启 control-mode 实时监听", { exact: true }).waitFor();
  await page.getByRole("button", { name: "实时监听 session build", exact: true }).click();
  await page.getByText("build 已开启 control-mode 实时监听", { exact: true }).waitFor();
  await page.getByRole("button", { name: "关闭 Tmux", exact: true }).click();
  await page.locator(".tmux-dialog").waitFor({ state: "detached" });
  await page.waitForFunction(() => window.__invokeCalls.filter((call) => (
    call.command === "start_tmux_control" || call.command === "stop_tmux_control"
  )).length >= 10);
  await page.getByRole("button", { name: "工具", exact: true }).click();
  await page.getByRole("button", { name: "Tmux", exact: true }).click();
  await page.getByRole("heading", { name: "窗口与窗格" }).waitFor();
  const controlCalls = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "start_tmux_control" || call.command === "stop_tmux_control"
  )));
  assert(controlCalls.length === 10
    && controlCalls[0].args.sessionId === "ssh-tmux"
    && controlCalls[0].args.target === "lab"
    && controlCalls[1].args.sessionId === "ssh-tmux"
    && controlCalls[1].args.target === "build"
    && controlCalls[2].args.sessionId === "ssh-tmux"
    && controlCalls[2].args.target === "lab"
    && controlCalls[3].args.sessionId === "ssh-tmux"
    && controlCalls[3].args.target === "lab"
    && controlCalls[4].args.sessionId === "ssh-tmux"
    && controlCalls[4].args.target === "lab"
    && controlCalls[5].args.sessionId === "ssh-tmux"
    && controlCalls[5].args.target === "build"
    && controlCalls[6].args.sessionId === "ssh-tmux"
    && controlCalls[6].args.target === "lab"
    && controlCalls[7].args.sessionId === "ssh-tmux"
    && controlCalls[7].args.target === "build"
    && controlCalls[8].args.sessionId === "ssh-tmux"
    && controlCalls[8].args.target === "lab"
    && controlCalls[9].args.sessionId === "ssh-tmux"
    && controlCalls[9].args.target === "build",
  `invalid control-mode lifecycle: ${JSON.stringify(controlCalls)}`);

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

  const paneTwo = page.locator('[data-tmux-pane="%2"]');
  await paneTwo.getByRole("button", { name: "激活 pane lab:0.1", exact: true }).click();
  await page.getByText("lab:0.1 已激活", { exact: true }).waitFor();
  assert(await paneTwo.evaluate((element) => element.classList.contains("active")), "selected pane did not become active");
  assert(await page.locator('[data-tmux-pane="%1"]').evaluate((element) => !element.classList.contains("active")),
    "previous pane remained active after selection");

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

  const splitPane = page.locator('[data-tmux-pane="%5"]');
  await splitPane.getByRole("combobox", { name: "lab:0.2 pane 移动", exact: true }).selectOption("break");
  await page.getByText("lab:0.2 已拆为新 window", { exact: true }).waitFor();
  assert(await page.locator('[data-tmux-target="lab:2"] [data-tmux-pane="%5"]').count() === 1,
    "broken pane did not move to a new window");

  await splitPane.getByRole("combobox", { name: "lab:2.0 pane 移动", exact: true }).selectOption("vertical:build:0");
  await page.getByText("tmux move denied", { exact: true }).waitFor();
  assert(await page.locator('[data-tmux-target="lab:2"] [data-tmux-pane="%5"]').count() === 1,
    "failed pane move changed the confirmed remote snapshot");

  await splitPane.getByRole("combobox", { name: "lab:2.0 pane 移动", exact: true }).selectOption("horizontal:lab:1");
  await page.getByText("lab:2.0 已移到 lab:1", { exact: true }).waitFor();
  assert(await page.locator('[data-tmux-target="lab:2"]').count() === 0, "empty source window remains after pane move");
  assert(await page.locator('[data-tmux-target="lab:1"] [data-tmux-pane="%5"]').count() === 1,
    "moved pane is missing from destination window");

  await page.getByRole("button", { name: "关闭 pane lab:1.1", exact: true }).click();
  await page.getByText("关闭 pane lab:1.1？", { exact: true }).waitFor();
  await page.getByRole("button", { name: "确认关闭", exact: true }).click();
  await page.getByText("lab:1.1 已关闭", { exact: true }).waitFor();
  assert(await page.locator('[data-tmux-pane="%5"]').count() === 0, "closed pane remains visible");

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
  assert(mutationCalls.map((call) => call.args.request.action).join(",") === "select-pane,select-layout,split-pane-horizontal,swap-pane-next,resize-pane-right,break-pane,move-pane-vertical,move-pane-horizontal,kill-pane,rename-session,new-window,rename-window,kill-window,kill-session",
    `unexpected Tmux mutation sequence: ${JSON.stringify(mutationCalls)}`);
  assert(mutationCalls[0].args.request.target === "%2", `invalid select request: ${JSON.stringify(mutationCalls[0])}`);
  assert(mutationCalls[1].args.request.target === "lab:0" && mutationCalls[1].args.request.layout === "tiled",
    `invalid layout request: ${JSON.stringify(mutationCalls[1])}`);
  assert(mutationCalls[2].args.request.target === "%1", `invalid split request: ${JSON.stringify(mutationCalls[2])}`);
  assert(mutationCalls[3].args.request.target === "%1", `invalid swap request: ${JSON.stringify(mutationCalls[3])}`);
  assert(mutationCalls[4].args.request.target === "%1" && mutationCalls[4].args.request.amount === 5,
    `invalid resize request: ${JSON.stringify(mutationCalls[4])}`);
  assert(mutationCalls[5].args.request.target === "%5", `invalid break request: ${JSON.stringify(mutationCalls[5])}`);
  assert(mutationCalls[6].args.request.target === "%5" && mutationCalls[6].args.request.destination === "build:0",
    `invalid failed move request: ${JSON.stringify(mutationCalls[6])}`);
  assert(mutationCalls[7].args.request.target === "%5" && mutationCalls[7].args.request.destination === "lab:1",
    `invalid move request: ${JSON.stringify(mutationCalls[7])}`);
  assert(mutationCalls[8].args.request.target === "%5", `invalid pane close request: ${JSON.stringify(mutationCalls[8])}`);
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
    controlCalls,
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
