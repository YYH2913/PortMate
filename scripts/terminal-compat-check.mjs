import { spawn } from "node:child_process";
import { createServer } from "node:net";
import process from "node:process";
import { chromium } from "playwright-core";

const chromeExecutable = process.env.PORTMATE_CHROME ?? "/usr/bin/google-chrome";
const screenshotPrefix = process.env.PORTMATE_TERMINAL_SCREENSHOT_PREFIX
  ?? "/tmp/portmate-terminal-compat";

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

async function waitForServer(url, processOutput) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // Vite is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`timed out waiting for ${url}\n${processOutput()}`);
}

function createSession(id, name) {
  const now = "2026-07-15T00:00:00.000Z";
  return {
    profile: {
      id,
      name,
      kind: "tcp",
      group: "Compatibility",
      tags: ["terminal-baseline"],
      connection: {
        kind: "tcp",
        host: "192.0.2.10",
        port: id === "session-a" ? 2201 : 2202,
        reconnect: false,
        reconnectDelayMs: 1000,
        keepaliveEnabled: true,
        keepaliveIdleSeconds: 30,
        keepaliveIntervalSeconds: 10,
        keepaliveRetries: 3,
        proxy: { enabled: false, kind: "socks5", host: "", port: 1080, username: "" },
        telnetBinary: true,
        telnetNaws: true,
      },
      terminal: {
        term: "xterm-256color",
        rows: 32,
        cols: 120,
        scrollback: 200000,
        fontFamily: "JetBrains Mono, monospace",
        fontSize: 13,
        theme: "portmate-dark",
      },
      logging: {
        enabled: false,
        raw: false,
        text: true,
        jsonl: true,
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
      connectedSince: now,
      lastActivity: now,
      lastDisconnect: null,
      lastDisconnectReason: null,
      activeTransport: "tcp",
    },
    logLines: id === "session-a" ? 5 : 1,
    lastLine: id === "session-a" ? "PORTMATE VTT BASELINE" : "secondary$ ",
  };
}

function createEvent(id, sessionId, text) {
  return {
    id,
    sessionId,
    paneId: `${sessionId}:main`,
    ts: "2026-07-15T00:00:00.000Z",
    direction: "inbound",
    stream: "stdout",
    bytesRef: null,
    text,
    annotations: {},
  };
}

const sessions = [
  createSession("session-a", "Primary VTT"),
  createSession("session-b", "Secondary shell"),
];
const eventsBySession = {
  "session-a": [
    createEvent("a-normal", "session-a", "NORMAL-PROMPT $ "),
    createEvent("a-alt-enter", "session-a", "\x1b[?1049h\x1b[2J\x1b[H"),
    createEvent("a-title", "session-a", "\x1b[1;38;2;94;234;212mPORTMATE VTT BASELINE\x1b[0m"),
    createEvent("a-color", "session-a", "\x1b[4;8HTRUECOLOR \x1b[38;2;255;120;80mRGB-OK\x1b[0m"),
    createEvent("a-wide-mouse", "session-a", "\x1b[6;4H宽字符界面\x1b[?1000h\x1b[?1006h"),
  ],
  "session-b": [createEvent("b-prompt", "session-b", "secondary$ ")],
};
const workspace = {
  version: 4,
  root: {
    kind: "split",
    id: "split-main",
    direction: "vertical",
    ratio: 0.68,
    first: {
      kind: "pane",
      id: "pane-a",
      activeViewId: "view-a",
      views: [
        { id: "view-a", sessionId: "session-a", title: "Primary", color: "#008B8B", keyMode: "remote" },
        { id: "view-b", sessionId: "session-b", title: "Secondary", color: "#DAA520", keyMode: "remote" },
      ],
    },
    second: {
      kind: "pane",
      id: "pane-b",
      activeViewId: "view-a-copy",
      views: [
        { id: "view-a-copy", sessionId: "session-a", title: "Primary mirror", color: "#228B22", keyMode: "remote" },
      ],
    },
  },
  activePaneId: "pane-a",
  activeId: "session-a",
  tabColors: {},
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
  await context.route("https://www.google.com/**", (route) => route.fulfill({
    contentType: "text/html",
    body: "<!doctype html><title>PortMate online search test</title>",
  }));
  await context.addInitScript(({ initialWorkspace, initialSessions, initialEvents }) => {
    if (localStorage.getItem("portmate.compat.initialized") !== "1") {
      localStorage.clear();
      localStorage.setItem("portmate.compat.initialized", "1");
      localStorage.setItem("portmate.workspace.v1", JSON.stringify(initialWorkspace));
      localStorage.setItem("portmate.terminalPrefs", JSON.stringify({
        startupMode: "none",
        startupSessions: [],
        lockOnIdle: false,
        requireMasterPassword: false,
        oneKeyCompletionEnabled: false,
        mouseReporting: true,
        mouseCopyOnSelect: true,
      }));
    }
    window.__invokeCalls = [];
    window.__clipboardWrites = [];
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        readText: async () => "",
        writeText: async (text) => { window.__clipboardWrites.push(String(text)); },
      },
    });
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = {
      invoke: async (command, args = {}) => {
        window.__invokeCalls.push({ command, args });
        if (command === "list_sessions") return initialSessions;
        if (command === "tail_log") return initialEvents[args.sessionId] ?? [];
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
  }, { initialWorkspace: workspace, initialSessions: sessions, initialEvents: eventsBySession });

  const page = await context.newPage();
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(appUrl);
  await page.waitForFunction(() => {
    const hosts = [...document.querySelectorAll(".terminal-host")];
    return hosts.length === 2
      && hosts.every((host) => /^\d+x\d+$/.test(host.dataset.terminalSize ?? ""))
      && document.querySelectorAll('[data-terminal-resize-owner="active"]').length === 1;
  });

  const terminalState = () => page.evaluate(() => [...document.querySelectorAll("[data-pane-id]")].map((pane) => {
    const host = pane.querySelector(".terminal-host");
    return {
      paneId: pane.getAttribute("data-pane-id"),
      active: pane.classList.contains("active"),
      owner: host?.dataset.terminalResizeOwner,
      size: host?.dataset.terminalSize,
      restored: host?.dataset.terminalRestored,
      mouse: host?.dataset.terminalMouseReporting,
    };
  }));
  const resizeCalls = () => page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "resize_session")
    .map((call) => call.args));
  const clearCalls = () => page.evaluate(() => { window.__invokeCalls = []; });
  const expectedResize = (size) => {
    const [cols, rows] = size.split("x").map(Number);
    return { cols, rows };
  };
  const assertLatestResize = async (paneId, label) => {
    await page.waitForFunction(() => window.__invokeCalls.some((call) => call.command === "resize_session"));
    const panes = await terminalState();
    const pane = panes.find((item) => item.paneId === paneId);
    const calls = await resizeCalls();
    const expected = expectedResize(pane.size);
    const latest = calls.at(-1);
    assert(latest?.cols === expected.cols && latest?.rows === expected.rows,
      `${label} did not report ${pane.size}: ${JSON.stringify(calls)}`);
    return { panes, calls };
  };

  const initial = await terminalState();
  assert(initial.find((pane) => pane.paneId === "pane-a")?.owner === "active", "pane A must own the initial PTY size");
  assert(initial.find((pane) => pane.paneId === "pane-b")?.owner === "inactive", "pane B must not own the initial PTY size");
  assert(initial[0].size !== initial[1].size, `test panes need distinct dimensions: ${JSON.stringify(initial)}`);

  await clearCalls();
  await page.setViewportSize({ width: 1320, height: 820 });
  const resized = await assertLatestResize("pane-a", "active pane resize");
  const activeSize = expectedResize(resized.panes.find((pane) => pane.paneId === "pane-a").size);
  assert(resized.calls.every((call) => call.cols === activeSize.cols && call.rows === activeSize.rows),
    `inactive pane overwrote a viewport resize: ${JSON.stringify(resized.calls)}`);

  await clearCalls();
  await page.locator('[data-pane-id="pane-b"] header').click({ position: { x: 4, y: 14 } });
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-b"] .terminal-host')?.dataset.terminalResizeOwner === "active");
  const paneBActivation = await assertLatestResize("pane-b", "pane B activation");

  await clearCalls();
  await page.locator('[data-pane-id="pane-a"] header').click({ position: { x: 4, y: 14 } });
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalResizeOwner === "active");
  const paneAReactivation = await assertLatestResize("pane-a", "pane A reactivation");

  const openAndAssertSearch = async (query) => {
    await page.evaluate(() => window.dispatchEvent(new Event("portmate-terminal-search")));
    const input = page.locator('[data-pane-id="pane-a"] .terminal-search-bar input');
    await input.fill(query);
    const statusLocator = page.locator('[data-pane-id="pane-a"] .terminal-search-status');
    let status = "0/0";
    for (let attempt = 0; attempt < 100 && status === "0/0"; attempt += 1) {
      await input.press("Enter");
      await page.waitForTimeout(50);
      status = await statusLocator.textContent() ?? "0/0";
    }
    if (status === "0/0") {
      const diagnostics = await page.locator('[data-pane-id="pane-a"] .terminal-host').evaluate((host) => ({
        dataset: { ...host.dataset },
        rows: host.querySelector(".xterm-rows")?.textContent ?? "",
        canvases: host.querySelectorAll("canvas").length,
      }));
      throw new Error(`terminal buffer is missing ${query}: ${JSON.stringify(diagnostics)}`);
    }
    await input.fill("");
    await page.locator('[data-pane-id="pane-a"] [aria-label="关闭查找"]').click();
    return status;
  };
  const ansiSearch = await openAndAssertSearch("PORTMATE VTT BASELINE");
  const trueColorSearch = await openAndAssertSearch("TRUECOLOR RGB-OK");
  const wideSearch = await openAndAssertSearch("宽字符界面");

  await page.locator('[data-pane-id="pane-a"] [data-view-id="view-b"] [role="tab"]').click();
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] [data-view-id="view-b"] [role="tab"]')?.getAttribute("aria-selected") === "true");
  await page.locator('[data-pane-id="pane-a"] [data-view-id="view-a"] [role="tab"]').click();
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalRestored === "true");
  const restoredSearch = await openAndAssertSearch("PORTMATE VTT BASELINE");

  const activeScreen = page.locator('[data-pane-id="pane-a"] .xterm-screen');
  const selectTerminalText = async () => {
    const box = await activeScreen.boundingBox();
    assert(box && box.width > 100 && box.height > 40, `invalid terminal screen box: ${JSON.stringify(box)}`);
    await page.keyboard.down("Shift");
    await page.mouse.move(box.x + 8, box.y + 9);
    await page.mouse.down();
    await page.mouse.move(box.x + Math.min(220, box.width - 8), box.y + 9, { steps: 8 });
    await page.mouse.up();
    await page.keyboard.up("Shift");
    await page.evaluate(() => window.dispatchEvent(new Event("portmate-terminal-search")));
    const input = page.locator('[data-pane-id="pane-a"] .terminal-search-bar input');
    await page.waitForFunction(() => (
      document.querySelector('[data-pane-id="pane-a"] .terminal-search-bar input')?.value.length > 0
    ));
    const selected = await input.inputValue();
    await input.fill("");
    await page.locator('[data-pane-id="pane-a"] [aria-label="关闭查找"]').click();
    return selected;
  };
  await page.evaluate(() => { window.__clipboardWrites = []; });
  const selectedWithPreference = await selectTerminalText();
  await page.waitForFunction(() => window.__clipboardWrites.length > 0);
  const copiedWithPreference = await page.evaluate(() => window.__clipboardWrites.at(-1));
  assert(copiedWithPreference, "copy-on-select did not copy a terminal selection");

  await clearCalls();
  await activeScreen.dispatchEvent("contextmenu", {
    bubbles: true,
    button: 2,
    cancelable: true,
    clientX: 420,
    clientY: 180,
  });
  await page.locator(".terminal-context-menu").waitFor();
  const selectionPopupPromise = page.waitForEvent("popup");
  await page.locator(".terminal-context-menu .context-menu-row", { hasText: "在线搜索" }).click();
  const selectionPopup = await selectionPopupPromise;
  await selectionPopup.waitForLoadState("domcontentloaded");
  const onlineSelectionSearch = {
    url: selectionPopup.url(),
    hasOpener: await selectionPopup.evaluate(() => window.opener !== null),
    referrer: await selectionPopup.evaluate(() => document.referrer),
  };
  assert(new URL(onlineSelectionSearch.url).searchParams.get("q") === copiedWithPreference.trim(),
    `online search did not use the exact XTerm selection: ${JSON.stringify(onlineSelectionSearch)}`);
  assert(!onlineSelectionSearch.hasOpener && !onlineSelectionSearch.referrer,
    `online search retained opener/referrer access: ${JSON.stringify(onlineSelectionSearch)}`);
  await selectionPopup.close();
  const onlineSelectionWrites = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "send_text" || call.command === "send_bytes" || call.command === "run_command"
  )));
  assert(onlineSelectionWrites.length === 0,
    `selection online search wrote terminal input: ${JSON.stringify(onlineSelectionWrites)}`);

  await activeScreen.dispatchEvent("contextmenu", {
    bubbles: true,
    button: 2,
    cancelable: true,
    clientX: 420,
    clientY: 180,
  });
  await page.locator(".terminal-context-menu .context-menu-row", { hasText: "清除选择" }).click();
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalHasSelection === "false");
  await clearCalls();
  await page.locator(".menu-trigger", { hasText: "搜索" }).click();
  const fallbackPopupPromise = page.waitForEvent("popup");
  await page.locator(".menu-popover button", { hasText: "在线搜索" }).click();
  const fallbackPopup = await fallbackPopupPromise;
  await fallbackPopup.waitForLoadState("domcontentloaded");
  const onlineFallbackSearch = {
    url: fallbackPopup.url(),
    hasOpener: await fallbackPopup.evaluate(() => window.opener !== null),
    referrer: await fallbackPopup.evaluate(() => document.referrer),
  };
  assert(new URL(onlineFallbackSearch.url).searchParams.get("q") === sessions[0].lastLine,
    `online search did not fall back to the target session line: ${JSON.stringify(onlineFallbackSearch)}`);
  assert(!onlineFallbackSearch.hasOpener && !onlineFallbackSearch.referrer,
    `fallback online search retained opener/referrer access: ${JSON.stringify(onlineFallbackSearch)}`);
  await fallbackPopup.close();
  const onlineFallbackWrites = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "send_text" || call.command === "send_bytes" || call.command === "run_command"
  )));
  assert(onlineFallbackWrites.length === 0,
    `fallback online search wrote terminal input: ${JSON.stringify(onlineFallbackWrites)}`);

  await clearCalls();
  await activeScreen.click({ position: { x: 120, y: 80 } });
  await page.waitForFunction(() => window.__invokeCalls.some((call) => (
    call.command === "send_text" && /^\x1b\[<0;\d+;\d+[Mm]$/.test(call.args.text)
  )));
  const mouseTexts = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "send_text" && typeof call.args.text === "string")
    .map((call) => call.args.text)
    .filter((text) => text.startsWith("\x1b[<")));

  await page.screenshot({ path: `${screenshotPrefix}-desktop.png`, fullPage: true });
  const desktopLayout = await page.evaluate(() => ({
    innerWidth,
    documentWidth: document.documentElement.scrollWidth,
    ownerCount: document.querySelectorAll('[data-terminal-resize-owner="active"]').length,
    panes: [...document.querySelectorAll("[data-pane-id]")].map((pane) => {
      const rect = pane.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom, width: rect.width, height: rect.height };
    }),
  }));
  assert(desktopLayout.documentWidth <= desktopLayout.innerWidth, `desktop overflow: ${JSON.stringify(desktopLayout)}`);
  assert(desktopLayout.ownerCount === 1, `desktop resize owner count is ${desktopLayout.ownerCount}`);

  await page.evaluate(() => {
    const prefs = JSON.parse(localStorage.getItem("portmate.terminalPrefs"));
    prefs.mouseReporting = false;
    prefs.mouseCopyOnSelect = false;
    localStorage.setItem("portmate.terminalPrefs", JSON.stringify(prefs));
  });
  await page.reload();
  await page.waitForFunction(() => (
    document.querySelectorAll('.terminal-host[data-terminal-mouse-reporting="disabled"]').length === 2
    && document.querySelectorAll('[data-terminal-resize-owner="active"]').length === 1
  ));
  await page.evaluate(() => { window.__invokeCalls = []; window.__clipboardWrites = []; });
  const disabledScreen = page.locator('[data-pane-id="pane-a"] .xterm-screen');
  await disabledScreen.click({ position: { x: 120, y: 80 } });
  await page.waitForTimeout(150);
  const leakedMouseTexts = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "send_text" && typeof call.args.text === "string")
    .map((call) => call.args.text)
    .filter((text) => text.startsWith("\x1b[<")));
  assert(leakedMouseTexts.length === 0, `disabled mouse reporting leaked input: ${JSON.stringify(leakedMouseTexts)}`);
  await page.locator('[data-pane-id="pane-a"] .xterm-helper-textarea').focus();
  await page.keyboard.type("k");
  await page.waitForFunction(() => window.__invokeCalls.some((call) => call.command === "send_text" && call.args.text === "k"));

  await page.evaluate(() => { window.__clipboardWrites = []; });
  const selectedWithoutPreference = await selectTerminalText();
  await page.waitForTimeout(100);
  const disabledClipboardWrites = await page.evaluate(() => window.__clipboardWrites);
  assert(disabledClipboardWrites.length === 0,
    `disabled copy-on-select wrote to the clipboard: ${JSON.stringify(disabledClipboardWrites)}`);

  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForFunction(() => [...document.querySelectorAll(".terminal-host")]
    .every((host) => /^\d+x\d+$/.test(host.dataset.terminalSize ?? "")));
  await page.screenshot({ path: `${screenshotPrefix}-mobile.png`, fullPage: true });
  const mobileLayout = await page.evaluate(() => ({
    innerWidth,
    documentWidth: document.documentElement.scrollWidth,
    ownerCount: document.querySelectorAll('[data-terminal-resize-owner="active"]').length,
    panes: [...document.querySelectorAll("[data-pane-id]")].map((pane) => {
      const rect = pane.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom, width: rect.width, height: rect.height };
    }),
  }));
  assert(mobileLayout.documentWidth <= mobileLayout.innerWidth, `mobile overflow: ${JSON.stringify(mobileLayout)}`);
  assert(mobileLayout.ownerCount === 1, `mobile resize owner count is ${mobileLayout.ownerCount}`);
  assert(mobileLayout.panes.every((pane) => (
    pane.left >= 0 && pane.right <= mobileLayout.innerWidth && pane.width > 0 && pane.height > 0
  )), `mobile pane bounds are invalid: ${JSON.stringify(mobileLayout)}`);
  assert(pageErrors.length === 0, `browser exceptions: ${JSON.stringify(pageErrors)}`);

  console.log(JSON.stringify({
    initial,
    viewportResizeCalls: resized.calls,
    paneBActivationCalls: paneBActivation.calls,
    paneAReactivationCalls: paneAReactivation.calls,
    searches: { ansiSearch, trueColorSearch, wideSearch, restoredSearch },
    selections: { selectedWithPreference, selectedWithoutPreference },
    copiedWithPreference,
    onlineSearches: { selection: onlineSelectionSearch, fallback: onlineFallbackSearch },
    mouseTexts,
    leakedMouseTexts,
    disabledClipboardWrites,
    desktopLayout,
    mobileLayout,
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
