import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import process from "node:process";
import { chromium } from "playwright-core";

const chromeExecutable = process.env.PORTMATE_CHROME ?? "/usr/bin/google-chrome";
const screenshotPrefix = process.env.PORTMATE_TERMINAL_SCREENSHOT_PREFIX
  ?? "/tmp/portmate-terminal-compat";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function inspectOnlineSearchPopup(popup) {
  await popup.waitForURL((url) => url.hostname === "www.google.com" && url.pathname === "/search");
  await popup.waitForLoadState("load");
  return popup.evaluate(() => ({
    url: window.location.href,
    hasOpener: window.opener !== null,
    referrer: document.referrer,
  }));
}

async function inspectIsolatedPopup(popup, hostname) {
  await popup.waitForURL((url) => url.hostname === hostname);
  await popup.waitForLoadState("load");
  return popup.evaluate(() => ({
    url: window.location.href,
    hasOpener: window.opener !== null,
    referrer: document.referrer,
  }));
}

function captureAlternateScreenPty(label, command, input, marker) {
  const result = spawnSync("script", [
    "-qfec",
    command,
    "/dev/null",
  ], {
    cwd: process.cwd(),
    env: { ...process.env, TERM: "xterm-256color", LANG: "C", LC_ALL: "C" },
    input,
    encoding: "utf8",
    timeout: 5_000,
    maxBuffer: 4 * 1024 * 1024,
  });
  assert(!result.error, `failed to capture ${label} PTY output: ${result.error?.message}`);
  assert(result.status === 0, `${label} PTY capture exited with ${result.status}: ${result.stderr}`);

  const output = result.stdout;
  const alternateStart = output.indexOf("\x1b[?1049h");
  const alternateEnd = output.lastIndexOf("\x1b[?1049l");
  assert(alternateStart >= 0, `${label} PTY capture did not enter the alternate screen`);
  assert(alternateEnd > alternateStart, `${label} PTY capture did not leave the alternate screen`);

  const alternateFrame = output.slice(alternateStart, alternateEnd);
  const exitFrame = output.slice(alternateEnd);
  assert(alternateFrame.includes(marker), `${label} PTY capture did not render ${marker}`);
  return {
    alternateFrame,
    exitFrame,
    bytes: Buffer.byteLength(output),
  };
}

async function captureTopPty() {
  const maxBytes = 4 * 1024 * 1024;
  const child = spawn("script", ["-qfec", "top -d 0.1", "/dev/null"], {
    cwd: process.cwd(),
    env: { ...process.env, TERM: "xterm-256color", LANG: "C", LC_ALL: "C" },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const stdout = [];
  const stderr = [];
  let capturedBytes = 0;
  let failure = null;

  const output = await new Promise((resolve, reject) => {
    const inputTimer = setTimeout(() => {
      if (!child.killed) child.stdin.end("q");
    }, 500);
    const timeout = setTimeout(() => {
      failure = new Error("top PTY capture exceeded 5 seconds");
      child.kill("SIGTERM");
    }, 5_000);
    const cleanup = () => {
      clearTimeout(inputTimer);
      clearTimeout(timeout);
    };
    const collect = (chunks, chunk) => {
      capturedBytes += chunk.length;
      if (capturedBytes > maxBytes) {
        failure = new Error("top PTY capture exceeded 4 MiB");
        child.kill("SIGTERM");
        return;
      }
      chunks.push(chunk);
    };
    child.stdin.on("error", () => {});
    child.stdout.on("data", (chunk) => collect(stdout, chunk));
    child.stderr.on("data", (chunk) => collect(stderr, chunk));
    child.once("error", (error) => {
      cleanup();
      reject(error);
    });
    child.once("close", (status, signal) => {
      cleanup();
      if (failure) {
        reject(failure);
        return;
      }
      const stderrText = Buffer.concat(stderr).toString("utf8");
      if (status !== 0) {
        reject(new Error(`top PTY capture exited with ${status ?? signal}: ${stderrText}`));
        return;
      }
      resolve(Buffer.concat(stdout).toString("utf8"));
    });
  });

  assert(output.includes("\x1b[2J"), "top PTY capture did not clear the full screen");
  assert(output.includes("\x1b[?25l"), "top PTY capture did not hide the cursor");
  const cursorRestore = output.lastIndexOf("\x1b[?25h");
  assert(cursorRestore >= 0, "top PTY capture did not restore the cursor");
  const frame = output.slice(0, cursorRestore);
  assert(frame.includes("top -"), "top PTY capture did not render its header");
  assert(frame.includes("Tasks:"), "top PTY capture did not render its process summary");
  return {
    frame,
    exitFrame: output.slice(cursorRestore),
    bytes: Buffer.byteLength(output),
  };
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
  const now = "2026-07-15T00:00:00.000000Z";
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
        scrollback: 4096,
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

function createEvent(id, sessionId, text, ts = "2026-07-15T00:00:00.000000Z") {
  return {
    id,
    sessionId,
    paneId: `${sessionId}:main`,
    ts,
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
    createEvent("a-alt-enter", "session-a", "\x1b[?1049h\x1b[2J\x1b[H", "2026-07-15T00:00:01.111111Z"),
    createEvent("a-title", "session-a", "\x1b[1;38;2;94;234;212mPORTMATE VTT BASELINE\x1b[0m", "2026-07-15T00:00:02.222222Z"),
    createEvent("a-color", "session-a", "\x1b[4;8HTRUECOLOR \x1b[38;2;255;120;80mRGB-OK\x1b[0m", "2026-07-15T00:00:03.333333Z"),
    createEvent("a-wide-mouse", "session-a", "\x1b[6;4H宽字符界面\x1b[?1000h\x1b[?1006h", "2026-07-15T00:00:04.444444Z"),
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
        { id: "view-a", sessionId: "session-a", title: "Primary", color: "#008B8B", keyMode: "command" },
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
const readmePtyMarker = '<h1 align="center">PortMate</h1>';
const vimPty = captureAlternateScreenPty(
  "Vim",
  "vim -Nu NONE -n README.md",
  ":q!\r",
  readmePtyMarker,
);
const lessPty = captureAlternateScreenPty("less", "less -R README.md", "q", readmePtyMarker);
const topPty = await captureTopPty();
const longLogLineCount = 6_000;
const longLogTailMarker = "PORTMATE-LONG-LOG-TAIL-006000";
const longLogText = `${Array.from({ length: longLogLineCount }, (_, index) => (
  `2026-07-15T00:00:00.000000Z INFO compatibility line ${String(index + 1).padStart(6, "0")} ${"x".repeat(48)}`
)).join("\r\n")}\r\n${longLogTailMarker}\r\n`;

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
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    reducedMotion: "reduce",
  });
  await context.route("https://www.google.com/**", (route) => route.fulfill({
    contentType: "text/html",
    body: "<!doctype html><title>PortMate online search test</title>",
  }));
  await context.route("https://terminal.example.test/**", (route) => route.fulfill({
    contentType: "text/html",
    body: "<!doctype html><title>PortMate terminal link test</title><script>document.body.dataset.hasOpener = String(Boolean(window.opener)); document.body.dataset.referrer = document.referrer;</script>",
  }));
  await context.route("https://trigger.example.test/**", (route) => route.fulfill({
    contentType: "text/html",
    body: "<!doctype html><title>PortMate trigger link test</title>",
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
        readText: async () => "",
        writeText: async (text) => { window.__clipboardWrites.push(String(text)); },
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
        if (command === "list_sessions") {
          const terminalTheme = localStorage.getItem("portmate.compat.terminalTheme");
          const terminalOpacity = Number(localStorage.getItem("portmate.compat.terminalOpacity"));
          if (!terminalTheme && !terminalOpacity) return initialSessions;
          return initialSessions.map((session) => ({
            ...session,
            profile: {
              ...session.profile,
              terminal: {
                ...session.profile.terminal,
                ...(terminalTheme ? { theme: terminalTheme } : {}),
                ...(terminalOpacity ? { backgroundOpacity: terminalOpacity } : {}),
              },
            },
          }));
        }
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
  }, { initialWorkspace: workspace, initialSessions: sessions, initialEvents: eventsBySession });

  const page = await context.newPage();
  const pageErrors = [];
  const fontResponses = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("response", (response) => {
    if (/JetBrainsMono-(?:Regular|SemiBold|Bold)\.woff2/.test(response.url())) {
      fontResponses.push({ url: response.url(), status: response.status() });
    }
  });
  await page.goto(appUrl);
  try {
    await page.waitForFunction(() => {
      const hosts = [...document.querySelectorAll(".terminal-host")];
      return hosts.length === 2
        && hosts.every((host) => /^\d+x\d+$/.test(host.dataset.terminalSize ?? ""))
        && document.querySelectorAll('[data-terminal-resize-owner="active"]').length === 1;
    });
  } catch (error) {
    const body = await page.locator("body").innerText().catch(() => "<body unavailable>");
    throw new Error(`terminal workspace did not become ready: ${error.message}\npage errors: ${JSON.stringify(pageErrors)}\nbody: ${body.slice(0, 2_000)}\nvite: ${viteOutput.slice(-4_000)}`);
  }

  const bundledFonts = await page.evaluate(async () => {
    const weights = [400, 600, 700];
    const loaded = await Promise.all(weights.map(async (weight) => ({
      weight,
      faces: (await document.fonts.load(`${weight} 13px "JetBrains Mono"`, "PortMate 0O1l ─│┌┐└┘"))
        .map((face) => ({ family: face.family, status: face.status, weight: face.weight })),
    })));
    return {
      bootstrap: document.documentElement.dataset.bundledTerminalFont,
      loaded,
    };
  });
  assert(bundledFonts.bootstrap === "loaded",
    `bundled terminal font was not ready before XTerm mounted: ${JSON.stringify(bundledFonts)}`);
  assert(bundledFonts.loaded.every((entry) => entry.faces.length > 0
      && entry.faces.every((face) => face.family === "JetBrains Mono" && face.status === "loaded")),
  `bundled terminal font weights did not load: ${JSON.stringify(bundledFonts)}`);
  assert(["Regular", "SemiBold", "Bold"].every((weight) => fontResponses
    .some((response) => response.status === 200 && response.url.includes(`JetBrainsMono-${weight}.woff2`))),
  `bundled terminal font assets were not served successfully: ${JSON.stringify(fontResponses)}`);

  const terminalState = () => page.evaluate(() => [...document.querySelectorAll("[data-pane-id]")].map((pane) => {
    const host = pane.querySelector(".terminal-host");
    return {
      paneId: pane.getAttribute("data-pane-id"),
      active: pane.classList.contains("active"),
      owner: host?.dataset.terminalResizeOwner,
      size: host?.dataset.terminalSize,
      restored: host?.dataset.terminalRestored,
      mouse: host?.dataset.terminalMouseReporting,
      keyMode: host?.dataset.terminalKeyMode,
      cursor: host?.dataset.terminalCursorStyle,
    };
  }));
  const resizeCalls = () => page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "resize_session")
    .map((call) => call.args));
  const clearCalls = () => page.evaluate(() => { window.__invokeCalls = []; });
  const emitSessionEvent = (event) => page.evaluate((payload) => {
    window.__emitTauriEvent("portmate-session-event", payload);
  }, event);
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
  assert(initial.every((pane) => pane.keyMode === "remote" && pane.cursor === "bar"),
    `persisted terminal modes did not restore as Insert/bar cursors: ${JSON.stringify(initial)}`);
  assert(initial[0].size !== initial[1].size, `test panes need distinct dimensions: ${JSON.stringify(initial)}`);
  const activeHost = page.locator('[data-pane-id="pane-a"] .terminal-host');
  const inspectSemanticRendering = async (host, expectedColors) => {
    await page.waitForTimeout(100);
    const rendering = await host.evaluate((element) => {
      const decorationElements = [...element.querySelectorAll(".xterm-decoration")];
      const decorationRects = decorationElements.map((decoration) => decoration.getBoundingClientRect());
      const hostRect = element.getBoundingClientRect();
      return {
        state: element.dataset.terminalSemanticHighlighting,
        renderer: element.dataset.terminalRenderer,
        decorations: Number(element.dataset.terminalSemanticDecorationCount ?? "-1"),
        elements: decorationElements.length,
        hostHeight: hostRect.height,
        top: Math.min(...decorationRects.map((rect) => rect.top)) - hostRect.top,
        bottom: Math.max(...decorationRects.map((rect) => rect.bottom)) - hostRect.top,
      };
    });
    const screenshot = await host.screenshot();
    const pixelColors = await page.evaluate(async ({ base64, colors, hostHeight, top, bottom }) => {
      const expected = colors.map((color) => [
        Number.parseInt(color.slice(1, 3), 16),
        Number.parseInt(color.slice(3, 5), 16),
        Number.parseInt(color.slice(5, 7), 16),
      ]);
      const image = new Image();
      image.src = `data:image/png;base64,${base64}`;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.naturalWidth;
      canvas.height = image.naturalHeight;
      const context = canvas.getContext("2d", { willReadFrequently: true });
      context.drawImage(image, 0, 0);
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
      const matched = new Set();
      const firstY = Math.max(0, Math.floor(top * canvas.height / hostHeight));
      const lastY = Math.min(canvas.height, Math.ceil(bottom * canvas.height / hostHeight));
      for (let y = firstY; y < lastY; y += 1) {
        for (let x = 0; x < canvas.width; x += 1) {
          const offset = (y * canvas.width + x) * 4;
          for (let index = 0; index < expected.length; index += 1) {
            const [red, green, blue] = expected[index];
            if (Math.abs(pixels[offset] - red) <= 4
              && Math.abs(pixels[offset + 1] - green) <= 4
              && Math.abs(pixels[offset + 2] - blue) <= 4
              && pixels[offset + 3] > 0) {
              matched.add(colors[index]);
            }
          }
        }
      }
      return [...matched];
    }, {
      base64: screenshot.toString("base64"),
      colors: expectedColors,
      hostHeight: rendering.hostHeight,
      top: rendering.top,
      bottom: rendering.bottom,
    });
    return { ...rendering, pixelColors };
  };
  const inspectCursorRendering = async (host, expectedStyle) => {
    const renderer = await host.getAttribute("data-terminal-renderer");
    if (renderer === "dom") {
      const rendered = await host.locator(`.xterm-rows .xterm-cursor.xterm-cursor-${expectedStyle}`).count() > 0;
      assert(rendered, `DOM renderer did not draw the ${expectedStyle} cursor`);
      return { renderer, style: expectedStyle, renderedClass: `xterm-cursor-${expectedStyle}` };
    }

    const webglContext = await host.evaluate((element) => {
      for (const canvas of element.querySelectorAll("canvas")) {
        const context = canvas.getContext("webgl2");
        if (!context) continue;
        return {
          width: canvas.width,
          height: canvas.height,
          preserveDrawingBuffer: context.getContextAttributes()?.preserveDrawingBuffer === true,
        };
      }
      return null;
    });
    assert(webglContext?.width > 0 && webglContext.height > 0,
      `WebGL renderer did not expose a drawable canvas: ${JSON.stringify(webglContext)}`);
    assert(webglContext.preserveDrawingBuffer,
      `WebGL automation canvas did not preserve rendered pixels: ${JSON.stringify(webglContext)}`);

    const cursorColor = await host.evaluate((element) => {
      const value = getComputedStyle(element).getPropertyValue("--xterm-cursor-color").trim();
      return value || element.dataset.terminalCursorColor || "#5eead4";
    });
    const inspectScreenshot = async (screenshot) => page.evaluate(async ({ base64, color }) => {
      const expected = [
        Number.parseInt(color.slice(1, 3), 16),
        Number.parseInt(color.slice(3, 5), 16),
        Number.parseInt(color.slice(5, 7), 16),
      ];
      const image = new Image();
      image.src = `data:image/png;base64,${base64}`;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.naturalWidth;
      canvas.height = image.naturalHeight;
      const context = canvas.getContext("2d", { willReadFrequently: true });
      context.drawImage(image, 0, 0);
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
      const matches = new Set();
      for (let y = 0; y < canvas.height; y += 1) {
        for (let x = 0; x < canvas.width; x += 1) {
          const offset = (y * canvas.width + x) * 4;
          if (Math.abs(pixels[offset] - expected[0]) <= 4
            && Math.abs(pixels[offset + 1] - expected[1]) <= 4
            && Math.abs(pixels[offset + 2] - expected[2]) <= 4
            && pixels[offset + 3] > 0) {
            matches.add(y * canvas.width + x);
          }
        }
      }
      const found = [];
      while (matches.size) {
        const start = matches.values().next().value;
        const queue = [start];
        matches.delete(start);
        let minX = start % canvas.width;
        let maxX = minX;
        let minY = Math.floor(start / canvas.width);
        let maxY = minY;
        let area = 0;
        for (let index = 0; index < queue.length; index += 1) {
          const point = queue[index];
          const x = point % canvas.width;
          const y = Math.floor(point / canvas.width);
          area += 1;
          minX = Math.min(minX, x);
          maxX = Math.max(maxX, x);
          minY = Math.min(minY, y);
          maxY = Math.max(maxY, y);
          for (let dy = -1; dy <= 1; dy += 1) {
            for (let dx = -1; dx <= 1; dx += 1) {
              const nextX = x + dx;
              const nextY = y + dy;
              if ((dx === 0 && dy === 0) || nextX < 0 || nextX >= canvas.width
                || nextY < 0 || nextY >= canvas.height) continue;
              const next = nextY * canvas.width + nextX;
              if (!matches.delete(next)) continue;
              queue.push(next);
            }
          }
        }
        if (area >= 4) found.push({
          area,
          width: maxX - minX + 1,
          height: maxY - minY + 1,
          bottom: maxY,
        });
      }
      return found.sort((left, right) => right.bottom - left.bottom || right.area - left.area).slice(0, 8);
    }, { base64: screenshot.toString("base64"), color: cursorColor });
    const samples = [];
    const deadline = Date.now() + 1_600;
    do {
      const components = await inspectScreenshot(await host.screenshot());
      samples.push(components);
      const cursor = components[0];
      const rendered = expectedStyle === "block"
        ? cursor?.width >= 5 && cursor.height >= 8 && cursor.area >= cursor.width * cursor.height * 0.6
        : cursor?.width <= 3 && cursor.height >= 8;
      if (rendered) return { renderer, style: expectedStyle, context: webglContext, pixels: cursor };
      await page.waitForTimeout(80);
    } while (Date.now() < deadline);
    assert(false, `WebGL renderer did not draw the ${expectedStyle} cursor across a blink cycle: ${JSON.stringify(samples)}`);
  };
  await page.waitForFunction(() => (
    document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalSemanticHighlighting === "alternate"
      && document.querySelector('[data-pane-id="pane-a"] .terminal-timestamp-gutter')?.getAttribute("data-buffer-type") === "alternate"
      && document.querySelectorAll('[data-pane-id="pane-a"] .terminal-timestamp-gutter time').length > 0
  ));
  const initialSemanticState = await activeHost.evaluate((host) => ({
    state: host.dataset.terminalSemanticHighlighting,
    decorations: Number(host.dataset.terminalSemanticDecorationCount ?? "-1"),
  }));
  const initialTimestampAlternate = await page.locator('[data-pane-id="pane-a"] .terminal-timestamp-gutter').evaluate((gutter) => ({
    bufferType: gutter.getAttribute("data-buffer-type"),
    count: gutter.querySelectorAll("time").length,
    timestamps: [...gutter.querySelectorAll("time")].map((time) => time.getAttribute("datetime")),
  }));
  const initialAlternateUniqueTimestamps = new Set(initialTimestampAlternate.timestamps);
  assert(initialSemanticState.decorations === 0,
    `semantic highlighting leaked into the initial alternate screen: ${JSON.stringify(initialSemanticState)}`);
  assert(initialTimestampAlternate.bufferType === "alternate"
    && initialTimestampAlternate.count > 0
    && initialAlternateUniqueTimestamps.size >= 2
    && initialAlternateUniqueTimestamps.has("2026-07-15T00:00:01.111111Z")
    && initialAlternateUniqueTimestamps.has("2026-07-15T00:00:04.444444Z"),
  `the initial alternate screen lost per-row event timestamps: ${JSON.stringify(initialTimestampAlternate)}`);

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
  const openAndAssertMissing = async (query) => {
    await page.evaluate(() => window.dispatchEvent(new Event("portmate-terminal-search")));
    const input = page.locator('[data-pane-id="pane-a"] .terminal-search-bar input');
    await input.fill(query);
    await input.press("Enter");
    await page.waitForTimeout(100);
    const status = await page.locator('[data-pane-id="pane-a"] .terminal-search-status').textContent() ?? "0/0";
    if (status !== "0/0") throw new Error(`terminal buffer unexpectedly contains ${query}: ${status}`);
    await input.fill("");
    await page.locator('[data-pane-id="pane-a"] [aria-label="关闭查找"]').click();
    return status;
  };
  const ansiSearch = await openAndAssertSearch("PORTMATE VTT BASELINE");
  const trueColorSearch = await openAndAssertSearch("TRUECOLOR RGB-OK");
  const wideSearch = await openAndAssertSearch("宽字符界面");

  const timestampAlternateBeforeRestore = await page.locator('[data-pane-id="pane-a"] .terminal-timestamp-gutter').evaluate((gutter) => ({
    bufferType: gutter.getAttribute("data-buffer-type"),
    count: gutter.querySelectorAll("time").length,
    timestamps: [...gutter.querySelectorAll("time")].map((time) => time.getAttribute("datetime")),
  }));

  await page.locator('[data-pane-id="pane-a"] [data-view-id="view-b"] [role="tab"]').click();
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] [data-view-id="view-b"] [role="tab"]')?.getAttribute("aria-selected") === "true");
  await page.locator('[data-pane-id="pane-a"] [data-view-id="view-a"] [role="tab"]').click();
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalRestored === "true");
  await page.waitForFunction(() => {
    const gutter = document.querySelector('[data-pane-id="pane-a"] .terminal-timestamp-gutter');
    return gutter?.getAttribute("data-buffer-type") === "alternate"
      && gutter.querySelectorAll("time").length > 0;
  });
  const restoredTimestampAlternate = await page.locator('[data-pane-id="pane-a"] .terminal-timestamp-gutter').evaluate((gutter) => ({
    bufferType: gutter.getAttribute("data-buffer-type"),
    count: gutter.querySelectorAll("time").length,
    timestamps: [...gutter.querySelectorAll("time")].map((time) => time.getAttribute("datetime")),
  }));
  assert(restoredTimestampAlternate.bufferType === "alternate"
    && JSON.stringify(restoredTimestampAlternate.timestamps) === JSON.stringify(timestampAlternateBeforeRestore.timestamps),
  `alternate row timestamps changed after cached tab restoration: ${JSON.stringify({
    beforeRestore: timestampAlternateBeforeRestore,
    restored: restoredTimestampAlternate,
  })}`);
  const restoredSearch = await openAndAssertSearch("PORTMATE VTT BASELINE");

  await page.waitForFunction(() => (
    (window.__tauriEventListeners.get("portmate-session-event") || []).length >= 2
  ));
  const replayProbeEventCount = 4_100;
  const replayProbeWindow = 600;
  const replayProbeMarker = "PORTMATE-DEDUPE-ROLLOVER-REPLAY";
  await page.evaluate(({ eventCount, replayWindow, marker }) => {
    const sessionId = "session-a";
    const emit = (index, text = "") => window.__emitTauriEvent("portmate-session-event", {
      id: `a-rollover-${index}`,
      sessionId,
      paneId: `${sessionId}:main`,
      ts: "2026-07-15T00:00:00.000000Z",
      direction: "inbound",
      stream: "stdout",
      bytesRef: null,
      text,
      annotations: {},
    });
    for (let index = 0; index < eventCount; index += 1) emit(index);
    for (let index = eventCount - replayWindow; index < eventCount; index += 1) {
      emit(index, index === eventCount - replayWindow ? `${marker}\r\n` : "");
    }
  }, { eventCount: replayProbeEventCount, replayWindow: replayProbeWindow, marker: replayProbeMarker });
  await page.waitForTimeout(150);
  const replayProbeSearch = await openAndAssertMissing(replayProbeMarker);

  await emitSessionEvent(createEvent("a-baseline-alt-exit", "session-a", "\x1b[?1049l"));
  await page.waitForFunction(() => {
    const gutter = document.querySelector('[data-pane-id="pane-a"] .terminal-timestamp-gutter');
    return gutter?.getAttribute("data-buffer-type") === "normal"
      && [...gutter.querySelectorAll("time")].some((time) => (
        time.getAttribute("datetime") === "2026-07-15T00:00:00.000000Z"
      ));
  });
  const restoredTimestampNormal = await page.locator('[data-pane-id="pane-a"] .terminal-timestamp-gutter').evaluate((gutter) => ({
    bufferType: gutter.getAttribute("data-buffer-type"),
    count: gutter.querySelectorAll("time").length,
    timestamps: [...gutter.querySelectorAll("time")].map((time) => time.getAttribute("datetime")),
  }));
  const semanticCommand = 'root@OpenWrt:~# grep -n "wireless" /etc/config/wireless 192.168.1.1 42';
  await emitSessionEvent(createEvent("a-semantic-command", "session-a", `\r\n${semanticCommand}`));
  await page.waitForFunction(() => {
    const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
    return host?.dataset.terminalSemanticHighlighting === "active"
      && Number(host.dataset.terminalSemanticDecorationCount ?? "0") >= 6
      && host.querySelectorAll(".xterm-decoration").length >= 6;
  });
  const semanticDark = await inspectSemanticRendering(activeHost, [
    "#86efac", "#93c5fd", "#fde047", "#67e8f9", "#5eead4", "#d8b4fe",
  ]);
  assert(semanticDark.pixelColors.length >= 4,
    `semantic command did not render enough distinct dark-theme colors: ${JSON.stringify(semanticDark)}`);
  const semanticSearch = await openAndAssertSearch(semanticCommand);
  const semanticOutboundCommand = 'grep -n "wireless" /etc/config/wireless';
  await clearCalls();
  await page.locator('[data-pane-id="pane-a"] .xterm-helper-textarea').focus();
  await page.keyboard.type(semanticOutboundCommand);
  await page.waitForFunction(() => window.__invokeCalls.some((call) => call.command === "send_text"));
  await page.waitForTimeout(50);
  const semanticOutboundText = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "send_text")
    .map((call) => call.args.text)
    .join(""));
  assert(semanticOutboundText === semanticOutboundCommand && !semanticOutboundText.includes("\x1b"),
    `semantic highlighting mutated outbound terminal bytes: ${JSON.stringify(semanticOutboundText)}`);
  await emitSessionEvent(createEvent("a-semantic-alt-enter", "session-a", "\x1b[?1049h"));
  await page.waitForFunction(() => {
    const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
    const gutter = document.querySelector('[data-pane-id="pane-a"] .terminal-timestamp-gutter');
    return host?.dataset.terminalSemanticHighlighting === "alternate"
      && host.dataset.terminalSemanticDecorationCount === "0"
      && gutter?.getAttribute("data-buffer-type") === "alternate"
      && gutter.querySelectorAll("time").length > 0;
  });
  const semanticAlternate = await activeHost.evaluate((host) => ({
    state: host.dataset.terminalSemanticHighlighting,
    decorations: Number(host.dataset.terminalSemanticDecorationCount ?? "-1"),
  }));
  await emitSessionEvent(createEvent("a-semantic-alt-exit", "session-a", "\x1b[?1049l"));
  await page.waitForFunction(() => {
    const gutter = document.querySelector('[data-pane-id="pane-a"] .terminal-timestamp-gutter');
    return gutter?.getAttribute("data-buffer-type") === "normal"
      && gutter.querySelectorAll("time").length > 0;
  });
  const semanticTimestampRestored = await page.locator('[data-pane-id="pane-a"] .terminal-timestamp-gutter').evaluate((gutter) => ({
    bufferType: gutter.getAttribute("data-buffer-type"),
    count: gutter.querySelectorAll("time").length,
  }));
  const normalBeforeVimSearch = await openAndAssertSearch("NORMAL-PROMPT");
  await emitSessionEvent(createEvent("a-real-vim-frame", "session-a", vimPty.alternateFrame));
  const vimSearch = await openAndAssertSearch(readmePtyMarker);
  const hiddenNormalSearch = await openAndAssertMissing("NORMAL-PROMPT");
  await emitSessionEvent(createEvent("a-real-vim-exit", "session-a", vimPty.exitFrame));
  const normalAfterVimSearch = await openAndAssertSearch("NORMAL-PROMPT");
  const hiddenVimSearch = await openAndAssertMissing(readmePtyMarker);

  await emitSessionEvent(createEvent("a-real-less-frame", "session-a", lessPty.alternateFrame));
  const lessSearch = await openAndAssertSearch(readmePtyMarker);
  const hiddenNormalDuringLessSearch = await openAndAssertMissing("NORMAL-PROMPT");
  await emitSessionEvent(createEvent("a-real-less-exit", "session-a", lessPty.exitFrame));
  const normalAfterLessSearch = await openAndAssertSearch("NORMAL-PROMPT");
  const hiddenLessSearch = await openAndAssertMissing(readmePtyMarker);

  await emitSessionEvent(createEvent("a-real-top-frame", "session-a", topPty.frame));
  const topSearch = await openAndAssertSearch("Tasks:");
  await emitSessionEvent(createEvent("a-real-top-exit", "session-a", topPty.exitFrame));

  const longLogStartedAt = Date.now();
  const retainedLogTimestamp = "2026-07-15T00:00:05.555555Z";
  const postTrimTimestamp = "2026-07-15T00:00:06.666666Z";
  await emitSessionEvent(createEvent("a-long-log", "session-a", longLogText, retainedLogTimestamp));
  const longLogSearch = await openAndAssertSearch(longLogTailMarker);
  const longLogDurationMs = Date.now() - longLogStartedAt;
  assert(longLogDurationMs < 15_000,
    `${longLogLineCount}-line terminal render/search took ${longLogDurationMs} ms`);
  const longLogTimestamps = await page.locator('[data-pane-id="pane-a"] .terminal-terminal-region').evaluate((region) => {
    const host = region.querySelector(".terminal-host");
    const timestamps = [...region.querySelectorAll(".terminal-timestamp-gutter time")];
    return {
      clocks: timestamps.map((timestamp) => timestamp.textContent ?? ""),
      values: timestamps.map((timestamp) => timestamp.getAttribute("datetime")),
      count: timestamps.length,
      markerCount: Number(host?.dataset.terminalTimestampMarkerCount ?? "-1"),
      rows: Number(host?.dataset.terminalTimestampRows ?? "-1"),
    };
  });
  assert(longLogTimestamps.count === longLogTimestamps.rows
    && longLogTimestamps.clocks.every((clock) => /^\d{2}:\d{2}:\d{2}\.\d{6}$/.test(clock))
    && longLogTimestamps.values.every((timestamp) => timestamp === retainedLogTimestamp)
    && longLogTimestamps.markerCount >= 0
    && longLogTimestamps.markerCount < 200,
  `long terminal output lost per-row microsecond timestamps or marker compaction: ${JSON.stringify(longLogTimestamps)}`);
  await emitSessionEvent(createEvent(
    "a-post-scrollback-trim",
    "session-a",
    "PORTMATE-POST-SCROLLBACK-TRIM\r\n",
    postTrimTimestamp,
  ));
  await page.waitForFunction(({ retained, latest }) => {
    const timestamps = [...document.querySelectorAll('[data-pane-id="pane-a"] .terminal-timestamp-gutter time')]
      .map((timestamp) => timestamp.getAttribute("datetime"));
    return timestamps.length > 1 && timestamps.includes(retained) && timestamps.includes(latest);
  }, { retained: retainedLogTimestamp, latest: postTrimTimestamp });
  const timestampAfterScrollbackTrim = await page.locator('[data-pane-id="pane-a"] .terminal-timestamp-gutter').evaluate((gutter) => ({
    count: gutter.querySelectorAll("time").length,
    timestamps: [...gutter.querySelectorAll("time")].map((time) => time.getAttribute("datetime")),
  }));
  assert(timestampAfterScrollbackTrim.timestamps[0] === retainedLogTimestamp
    && timestampAfterScrollbackTrim.timestamps.at(-1) === postTrimTimestamp,
  `scrollback marker eviction lost its retained-row timestamp anchor: ${JSON.stringify(timestampAfterScrollbackTrim)}`);

  await page.locator('[data-pane-id="pane-a"] [data-view-id="view-b"] [role="tab"]').click();
  await page.waitForFunction(() => (
    document.querySelector('[data-pane-id="pane-a"] [data-view-id="view-b"] [role="tab"]')?.getAttribute("aria-selected") === "true"
      && document.querySelector('[data-pane-id="pane-a"] .terminal-canvas')?.getAttribute("data-terminal-session-id") === "session-b"
  ));
  const cachedNormalTimestampState = await page.evaluate(async () => {
    const { terminalStateCache, terminalStateCacheKey } = await import("/src/terminal-state-cache.ts");
    const state = terminalStateCache.get(terminalStateCacheKey("session-a", "view-a"));
    return state ? {
      bytes: new TextEncoder().encode(state.serialized).byteLength,
      hasAlternateEnter: state.serialized.includes("\x1b[?1049h"),
      hasAlternateExit: state.serialized.includes("\x1b[?1049l"),
      includesLongTail: state.serialized.includes("PORTMATE-LONG-LOG-TAIL-006000"),
      seenInitial: ["a-normal", "a-alt-enter", "a-title", "a-color", "a-wide-mouse"]
        .map((eventId) => state.seenEventIds.includes(eventId)),
      seenCount: state.seenEventIds.length,
      timestamps: state.timestamps ?? [],
      alternateTimestamps: state.alternateTimestamps ?? [],
    } : null;
  });
  assert(cachedNormalTimestampState
    && !cachedNormalTimestampState.hasAlternateEnter
    && cachedNormalTimestampState.includesLongTail
    && cachedNormalTimestampState.seenInitial.every(Boolean)
    && cachedNormalTimestampState.timestamps[0]?.ts === retainedLogTimestamp
    && cachedNormalTimestampState.timestamps.at(-1)?.ts === postTrimTimestamp
    && cachedNormalTimestampState.alternateTimestamps.length === 0,
  `normal terminal state was not cached before its view unmounted: ${JSON.stringify(cachedNormalTimestampState)}`);
  await page.locator('[data-pane-id="pane-a"] [data-view-id="view-a"] [role="tab"]').click();
  await page.waitForFunction(() => {
    const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
    const canvas = document.querySelector('[data-pane-id="pane-a"] .terminal-canvas');
    return canvas?.getAttribute("data-terminal-session-id") === "session-a"
      && host?.dataset.terminalReady === "true";
  });
  await page.waitForTimeout(150);
  const timestampAfterTrimCacheRestore = await page.locator('[data-pane-id="pane-a"] .terminal-terminal-region').evaluate((region) => {
    const host = region.querySelector(".terminal-host");
    const gutter = region.querySelector(".terminal-timestamp-gutter");
    return {
      buffer: host?.getAttribute("data-terminal-buffer"),
      markerCount: Number(host?.getAttribute("data-terminal-timestamp-marker-count") ?? "-1"),
      ready: host?.getAttribute("data-terminal-ready"),
      restored: host?.getAttribute("data-terminal-restored"),
      serialization: host?.getAttribute("data-terminal-serialization"),
      count: gutter?.querySelectorAll("time").length ?? 0,
      timestamps: [...(gutter?.querySelectorAll("time") ?? [])].map((time) => time.getAttribute("datetime")),
    };
  });
  assert(timestampAfterTrimCacheRestore.buffer === "normal"
    && timestampAfterTrimCacheRestore.restored === "true"
    && timestampAfterTrimCacheRestore.timestamps[0] === retainedLogTimestamp
    && timestampAfterTrimCacheRestore.timestamps.at(-1) === postTrimTimestamp,
  `cached terminal restoration lost its scrollback timestamp interval: ${JSON.stringify({
    cached: cachedNormalTimestampState,
    restored: timestampAfterTrimCacheRestore,
  })}`);

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
  const onlineSelectionSearch = await inspectOnlineSearchPopup(selectionPopup);
  assert(new URL(onlineSelectionSearch.url).searchParams.get("q") === copiedWithPreference.trim(),
    `online search did not use the exact XTerm selection: ${JSON.stringify(onlineSelectionSearch)}`);
  assert(!onlineSelectionSearch.hasOpener && !onlineSelectionSearch.referrer,
    `online search retained opener/referrer access: ${JSON.stringify(onlineSelectionSearch)}`);
  await selectionPopup.close();
  await page.bringToFront();
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
  await page.locator(".terminal-context-menu .context-menu-row", { hasText: "选择全部" }).click();
  await page.waitForFunction(() => (
    document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalHasSelection === "true"
  ));
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
  await activeScreen.dispatchEvent("contextmenu", {
    bubbles: true,
    button: 2,
    cancelable: true,
    clientX: 420,
    clientY: 180,
  });
  await page.locator(".terminal-context-menu").waitFor();
  const fallbackPopupPromise = page.waitForEvent("popup");
  await page.locator(".terminal-context-menu .context-menu-row", { hasText: "在线搜索" }).click();
  const fallbackPopup = await fallbackPopupPromise;
  const onlineFallbackSearch = await inspectOnlineSearchPopup(fallbackPopup);
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

  await activeScreen.dispatchEvent("contextmenu", {
    bubbles: true,
    button: 2,
    cancelable: true,
    clientX: 420,
    clientY: 180,
  });
  await page.locator(".terminal-context-menu").waitFor();
  const terminalBox = await activeScreen.boundingBox();
  assert(terminalBox, "terminal screen has no bounding box for internal-scroll menu regression");
  await page.mouse.move(terminalBox.x + terminalBox.width / 2, terminalBox.y + terminalBox.height / 2);
  await page.mouse.wheel(0, 240);
  await page.locator(".terminal-context-menu").waitFor({ state: "detached" });

  await clearCalls();
  await activeScreen.click({ position: { x: 120, y: 80 } });
  await page.waitForFunction(() => window.__invokeCalls.some((call) => (
    call.command === "send_text" && /^\x1b\[<0;\d+;\d+[Mm]$/.test(call.args.text)
  )));
  const mouseTexts = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "send_text" && typeof call.args.text === "string")
    .map((call) => call.args.text)
    .filter((text) => text.startsWith("\x1b[<")));

  const terminalWebLinkUrl = "https://terminal.example.test/path?q=portmate";
  await clearCalls();
  await emitSessionEvent(createEvent(
    "a-terminal-web-link",
    "session-a",
    `\x1b[2J\x1b[H${terminalWebLinkUrl}`,
  ));
  await openAndAssertSearch(terminalWebLinkUrl);
  const terminalLinkGeometry = await page.locator('[data-pane-id="pane-a"] .terminal-host').evaluate((host) => {
    const screen = host.querySelector(".xterm-screen")?.getBoundingClientRect();
    const [cols, rows] = (host.dataset.terminalSize ?? "").split("x").map(Number);
    return screen && cols > 0 && rows > 0
      ? {
          left: screen.left,
          top: screen.top,
          cellWidth: screen.width / cols,
          cellHeight: screen.height / rows,
        }
      : null;
  });
  assert(terminalLinkGeometry
    && terminalLinkGeometry.cellWidth > 0
    && terminalLinkGeometry.cellHeight > 0,
  `terminal web link geometry is unavailable: ${JSON.stringify(terminalLinkGeometry)}`);
  const terminalLinkX = terminalLinkGeometry.left + terminalLinkGeometry.cellWidth * 10.5;
  const terminalLinkY = terminalLinkGeometry.top + terminalLinkGeometry.cellHeight * 0.5;
  await page.mouse.move(terminalLinkX, terminalLinkY);
  await page.waitForTimeout(100);
  const terminalLinkHover = await page.evaluate(({ x, y }) => {
    const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
    const xterm = host?.querySelector(".xterm");
    return {
      pointer: host?.querySelector(".xterm-cursor-pointer") !== null,
      renderer: host?.getAttribute("data-terminal-renderer"),
      size: host?.getAttribute("data-terminal-size"),
      targetClasses: document.elementsFromPoint(x, y).map((element) => element.className),
      xtermRect: xterm?.getBoundingClientRect().toJSON(),
      xtermPadding: xterm ? {
        left: getComputedStyle(xterm).paddingLeft,
        top: getComputedStyle(xterm).paddingTop,
      } : null,
    };
  }, { x: terminalLinkX, y: terminalLinkY });
  assert(terminalLinkHover.pointer,
    `terminal web link did not activate on hover: ${JSON.stringify({ terminalLinkGeometry, terminalLinkHover })}`);
  const terminalLinkPopupPromise = page.waitForEvent("popup");
  await page.mouse.click(terminalLinkX, terminalLinkY);
  const terminalLinkPopup = await terminalLinkPopupPromise;
  const terminalWebLink = await inspectIsolatedPopup(terminalLinkPopup, "terminal.example.test");
  assert(new URL(terminalWebLink.url).href === terminalWebLinkUrl
    && !terminalWebLink.hasOpener
    && !terminalWebLink.referrer,
  `terminal web link was not isolated: ${JSON.stringify(terminalWebLink)}`);
  await terminalLinkPopup.close();
  await page.bringToFront();
  const terminalLinkWrites = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "send_text" || call.command === "send_bytes" || call.command === "run_command"
  )));
  assert(terminalLinkWrites.length === 0,
    `terminal web link click wrote terminal input: ${JSON.stringify(terminalLinkWrites)}`);

  const triggerWebLinkUrl = "https://trigger.example.test/search?q=portmate";
  const pagesBeforeTrigger = context.pages().length;
  await clearCalls();
  await page.evaluate((value) => window.__emitTauriEvent("portmate-trigger-effect", {
    sessionId: "session-a",
    triggerId: "trigger-link",
    triggerLabel: "Lookup",
    kind: "custom-link",
    value,
  }), triggerWebLinkUrl);
  const triggerLinkNotice = page.locator(".notice-dialog", { hasText: triggerWebLinkUrl });
  await triggerLinkNotice.waitFor();
  await page.waitForTimeout(100);
  assert(context.pages().length === pagesBeforeTrigger,
    "custom-link trigger opened a popup without explicit user confirmation");
  await page.screenshot({ path: `${screenshotPrefix}-trigger-link.png`, fullPage: true });
  const triggerLinkPopupPromise = page.waitForEvent("popup");
  await triggerLinkNotice.getByRole("button", { name: "打开链接" }).click();
  const triggerLinkPopup = await triggerLinkPopupPromise;
  const triggerWebLink = await inspectIsolatedPopup(triggerLinkPopup, "trigger.example.test");
  assert(new URL(triggerWebLink.url).href === triggerWebLinkUrl
    && !triggerWebLink.hasOpener
    && !triggerWebLink.referrer,
  `trigger web link was not isolated: ${JSON.stringify(triggerWebLink)}`);
  await triggerLinkPopup.close();
  await page.bringToFront();
  await triggerLinkNotice.waitFor({ state: "detached" });
  const triggerLinkWrites = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "send_text" || call.command === "send_bytes" || call.command === "run_command"
  )));
  assert(triggerLinkWrites.length === 0,
    `trigger web link wrote terminal input: ${JSON.stringify(triggerLinkWrites)}`);

  await page.evaluate(() => window.__emitTauriEvent("portmate-trigger-effect", {
    sessionId: "session-a",
    triggerId: "trigger-unsafe-link",
    triggerLabel: "Unsafe",
    kind: "custom-link",
    value: "javascript:alert(1)",
  }));
  const unsafeTriggerNotice = page.locator(".notice-dialog", { hasText: "javascript:alert(1)" });
  await unsafeTriggerNotice.waitFor();
  assert(await unsafeTriggerNotice.getByRole("button", { name: "打开链接" }).count() === 0,
    "unsafe custom-link trigger exposed an open action");
  await unsafeTriggerNotice.getByRole("button", { name: "确定" }).click();

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
  await emitSessionEvent(createEvent("a-completion-alt-exit", "session-a", "\x1b[?1049l"));
  await page.waitForFunction(() => (
    document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalBuffer === "normal"
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

  const activeTextarea = page.locator('[data-pane-id="pane-a"] .xterm-helper-textarea');
  const activeCompletion = page.locator('[data-pane-id="pane-a"] .terminal-completion');
  async function reloadWithTerminalPrefs(patch) {
    await page.evaluate((nextPrefs) => {
      const prefs = JSON.parse(localStorage.getItem("portmate.terminalPrefs"));
      localStorage.setItem("portmate.terminalPrefs", JSON.stringify({ ...prefs, ...nextPrefs }));
    }, patch);
    await page.reload();
    await page.waitForFunction(() => (
      document.querySelectorAll(".terminal-host").length === 2
      && document.querySelectorAll('[data-terminal-resize-owner="active"]').length === 1
    ));
    await activeTextarea.focus();
  }
  await emitSessionEvent(createEvent("a-completion-bottom-anchor", "session-a", "\x1b[999;1H"));
  await page.waitForTimeout(100);
  const terminalSizeBeforeCompletion = await activeHost.getAttribute("data-terminal-size");
  await clearCalls();
  await page.keyboard.press("Enter");
  await page.keyboard.type("git s");
  await activeCompletion.waitFor();
  const completionPlacement = await activeCompletion.evaluate((completion) => {
    const canvas = completion.closest(".terminal-canvas");
    const terminalRegion = completion.closest(".terminal-terminal-region");
    const host = canvas?.querySelector(".terminal-host");
    const completionRect = completion.getBoundingClientRect();
    const hostRect = host?.getBoundingClientRect();
    const canvasRect = terminalRegion?.getBoundingClientRect();
    return {
      placement: canvas?.getAttribute("data-completion-placement"),
      cursorBottom: Number(canvas?.getAttribute("data-completion-cursor-bottom") ?? "-1"),
      shift: Number(canvas?.getAttribute("data-completion-shift") ?? "-1"),
      canvasTop: canvasRect?.top ?? -1,
      canvasHeight: canvasRect?.height ?? -1,
      hostHeight: hostRect?.height ?? -1,
      hostBottom: hostRect?.bottom ?? -1,
      completionTop: completionRect.top,
      completionBottom: completionRect.bottom,
      canvasBottom: canvasRect?.bottom ?? -1,
    };
  });
  const completionCursorBottom = completionPlacement.canvasTop + completionPlacement.cursorBottom;
  assert(completionPlacement.placement === "below"
    && completionPlacement.cursorBottom >= 0
    && completionPlacement.shift > 0
    && Math.abs(completionPlacement.hostHeight - completionPlacement.canvasHeight) <= 1
    && completionPlacement.completionTop >= completionCursorBottom - 1
    && completionPlacement.completionTop <= completionCursorBottom + 6
    && completionPlacement.completionBottom <= completionPlacement.canvasBottom + 1,
  `command completion obscured the terminal input surface: ${JSON.stringify(completionPlacement)}`);
  await page.waitForTimeout(120);
  await page.screenshot({ path: `${screenshotPrefix}-completion-below.png`, fullPage: true });
  await emitSessionEvent(createEvent("a-completion-anchor-move", "session-a", "\x1b[5A"));
  await page.waitForFunction((previousShift) => {
    const canvas = document.querySelector('[data-pane-id="pane-a"] .terminal-canvas');
    return Number(canvas?.getAttribute("data-completion-shift") ?? "-1") < previousShift;
  }, completionPlacement.shift);
  const completionAfterRemoteCursorMove = await activeCompletion.evaluate((completion) => {
    const canvas = completion.closest(".terminal-canvas");
    const terminalRegion = completion.closest(".terminal-terminal-region");
    const canvasRect = terminalRegion?.getBoundingClientRect();
    const completionRect = completion.getBoundingClientRect();
    const cursorBottom = Number(canvas?.getAttribute("data-completion-cursor-bottom") ?? "-1");
    return {
      cursorBottom,
      shift: Number(canvas?.getAttribute("data-completion-shift") ?? "-1"),
      gap: completionRect.top - ((canvasRect?.top ?? -1) + cursorBottom),
    };
  });
  assert(completionAfterRemoteCursorMove.shift < completionPlacement.shift
    && completionAfterRemoteCursorMove.gap >= 1
    && completionAfterRemoteCursorMove.gap <= 6,
  `command completion did not follow the remote cursor: ${JSON.stringify(completionAfterRemoteCursorMove)}`);
  const completionBeforePaste = await activeCompletion.textContent();
  assert(completionBeforePaste?.includes("status"), `command completion did not activate before paste: ${completionBeforePaste}`);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(100);
  const terminalSizeAfterCompletion = await activeHost.getAttribute("data-terminal-size");
  const completionResizeCalls = await resizeCalls();
  assert(terminalSizeBeforeCompletion === terminalSizeAfterCompletion && completionResizeCalls.length === 0,
    `command completion changed the remote PTY size: ${JSON.stringify({ terminalSizeBeforeCompletion, terminalSizeAfterCompletion, completionResizeCalls })}`);

  await clearCalls();
  await page.keyboard.type("terraform pl");
  await activeCompletion.waitFor();
  await page.waitForFunction(() => window.__invokeCalls
    .filter((call) => call.command === "send_text")
    .map((call) => call.args.text)
    .join("") === "terraform pl");
  const terraformCompletion = await activeCompletion.textContent();
  assert(terraformCompletion?.includes("plan") && terraformCompletion.includes("terraform [全局选项]"),
    `Terraform command completion did not expose its structured schema: ${terraformCompletion}`);
  await clearCalls();
  await page.keyboard.press("Tab");
  await page.waitForTimeout(150);
  const terraformTabCalls = await page.evaluate(() => window.__invokeCalls);
  assert(terraformTabCalls.some((call) => call.command === "send_text" && call.args.text === "an "),
    `Terraform Tab completion did not send the expected suffix: ${JSON.stringify({ terraformCompletion, terraformTabCalls })}`);
  await page.keyboard.press("Enter");

  await page.keyboard.type("cmd /");
  await activeCompletion.waitFor();
  const cmdCompletion = await activeCompletion.textContent();
  assert(cmdCompletion?.includes("/c") && cmdCompletion.includes("cmd [选项] [命令]"),
    `Windows slash-style command options were not rendered: ${cmdCompletion}`);
  await page.keyboard.press("Enter");

  await page.keyboard.type("winget.exe install ");
  await activeCompletion.waitFor();
  const wingetUsage = await activeCompletion.locator(".terminal-completion-usage").textContent();
  assert(wingetUsage?.includes("winget.exe install [选项] <查询>"),
    `Windows executable command context was not resolved: ${wingetUsage}`);
  await page.keyboard.press("Enter");

  await clearCalls();
  await page.keyboard.type("clear ");
  await activeCompletion.waitFor();
  const completionUsageOnly = await activeCompletion.evaluate((completion) => ({
    usage: completion.querySelector(".terminal-completion-usage")?.textContent ?? "",
    candidates: completion.querySelectorAll(".terminal-completion-list > button").length,
  }));
  assert(completionUsageOnly.usage.includes("clear") && completionUsageOnly.candidates === 0,
    `usage-only command completion was not rendered accurately: ${JSON.stringify(completionUsageOnly)}`);
  await page.waitForFunction(() => window.__invokeCalls
    .filter((call) => call.command === "send_text" && typeof call.args.text === "string")
    .map((call) => call.args.text)
    .join("")
    .endsWith("clear "));
  await clearCalls();
  await page.keyboard.press("Tab");
  await page.waitForFunction(() => window.__invokeCalls.some((call) => (
    call.command === "send_text" && call.args.text === "\t"
  )));
  assert(await activeCompletion.count() === 0, "usage-only completion swallowed native shell Tab completion");
  await page.keyboard.press("Enter");

  await page.keyboard.type("portmate-unknown ");
  await activeCompletion.waitFor();
  const unknownCompletionUsage = await activeCompletion.evaluate((completion) => ({
    usage: completion.querySelector(".terminal-completion-usage")?.textContent ?? "",
    candidates: completion.querySelectorAll(".terminal-completion-list > button").length,
  }));
  assert(unknownCompletionUsage.usage.includes("portmate-unknown [参数...]")
    && unknownCompletionUsage.candidates === 0,
  `unknown command did not receive a non-inserting parameter hint: ${JSON.stringify(unknownCompletionUsage)}`);
  await page.waitForFunction(() => window.__invokeCalls
    .filter((call) => call.command === "send_text" && typeof call.args.text === "string")
    .map((call) => call.args.text)
    .join("")
    .endsWith("portmate-unknown "));
  await clearCalls();
  await page.keyboard.press("Tab");
  await page.waitForFunction(() => window.__invokeCalls.some((call) => (
    call.command === "send_text" && call.args.text === "\t"
  )));
  assert(await activeCompletion.count() === 0, "unknown command hint swallowed native shell Tab completion");
  await page.keyboard.press("Enter");

  await page.keyboard.type("clear ");
  await activeCompletion.waitFor();
  await clearCalls();
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => {
    const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
    return document.querySelector('[data-pane-id="pane-a"] .terminal-completion') === null
      && host?.dataset.terminalKeyMode === "command";
  });
  const usageEscapeLeaked = await page.evaluate(() => window.__invokeCalls.some((call) => (
    call.command === "send_text" && call.args.text === "\x1b"
  )));
  assert(!usageEscapeLeaked, "closing a usage-only completion leaked Escape to the remote session");
  await page.keyboard.press("i");
  await page.waitForFunction(() => (
    document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalKeyMode === "remote"
  ));
  await page.keyboard.press("Enter");

  const semanticUnicodeCommand = '路由器(config)# echo "你好" /tmp/固件.bin';
  await emitSessionEvent(createEvent("a-semantic-before-paste", "session-a", `\r\n${semanticUnicodeCommand}\r\n`));
  await page.waitForFunction(() => {
    const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
    return host?.dataset.terminalSemanticHighlighting === "active"
      && Number(host.dataset.terminalSemanticDecorationCount ?? "0") >= 3;
  });
  const semanticBeforePaste = await activeHost.evaluate((host) => ({
    state: host.dataset.terminalSemanticHighlighting,
    decorations: Number(host.dataset.terminalSemanticDecorationCount ?? "-1"),
  }));
  await activeTextarea.dispatchEvent("paste", { bubbles: true, cancelable: true });
  await page.keyboard.type("git s");
  await page.waitForTimeout(100);
  assert(await activeCompletion.count() === 0, "plain terminal paste did not pause command completion");
  const semanticAfterPaste = await activeHost.evaluate((host) => ({
    state: host.dataset.terminalSemanticHighlighting,
    decorations: Number(host.dataset.terminalSemanticDecorationCount ?? "-1"),
  }));
  assert(semanticAfterPaste.state === "active"
    && semanticAfterPaste.decorations === semanticBeforePaste.decorations,
  `semantic highlighting was incorrectly cleared after completion synchronization loss: ${JSON.stringify({ semanticBeforePaste, semanticAfterPaste })}`);

  await page.keyboard.press("Enter");
  await page.keyboard.type("git s");
  await activeCompletion.waitFor();
  await page.waitForTimeout(100);
  const semanticResumed = await activeHost.evaluate((host) => ({
    state: host.dataset.terminalSemanticHighlighting,
    decorations: Number(host.dataset.terminalSemanticDecorationCount ?? "-1"),
    buffer: host.dataset.terminalBuffer,
  }));
  assert(semanticResumed.state === "active",
    `semantic highlighting did not resume after the next input boundary: ${JSON.stringify(semanticResumed)}`);
  const completionAfterPasteBoundary = await activeCompletion.textContent();
  assert(completionAfterPasteBoundary?.includes("status"), `command completion did not resume after pasted line boundary: ${completionAfterPasteBoundary}`);
  await page.keyboard.press("Enter");

  await page.evaluate(() => { window.__invokeCalls = []; });
  await activeTextarea.focus();
  await page.keyboard.type("git s");
  await activeCompletion.waitFor();
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalCursorStyle === "bar");
  const insertCursorBeforeNormal = await inspectCursorRendering(activeHost, "bar");
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => {
    const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
    return document.querySelector('[data-pane-id="pane-a"] .terminal-completion') === null
      && host?.dataset.terminalKeyMode === "command"
      && host?.dataset.terminalCursorStyle === "block";
  });
  const normalCursor = await inspectCursorRendering(activeHost, "block");
  const insertNormalEscapeLeaked = await page.evaluate(() => window.__invokeCalls.some((call) => (
    call.command === "send_text" && call.args.text === "\x1b"
  )));
  assert(!insertNormalEscapeLeaked, "Insert -> Normal mode switch leaked Escape to the remote session");
  await page.screenshot({ path: `${screenshotPrefix}-normal-cursor.png`, fullPage: true });
  await page.keyboard.press("i");
  await page.waitForFunction(() => {
    const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
    return host?.dataset.terminalKeyMode === "remote"
      && host?.dataset.terminalCursorStyle === "bar";
  });
  const insertCursorAfterNormal = await inspectCursorRendering(activeHost, "bar");
  const insertNormalMode = await activeHost.evaluate((host) => ({
    mode: host.dataset.terminalKeyMode,
    cursor: host.dataset.terminalCursorStyle,
  }));
  insertNormalMode.rendering = {
    before: insertCursorBeforeNormal,
    normal: normalCursor,
    after: insertCursorAfterNormal,
  };

  await page.evaluate(() => { window.__clipboardWrites = []; });
  const selectedWithoutPreference = await selectTerminalText();
  await page.waitForTimeout(100);
  const disabledClipboardWrites = await page.evaluate(() => window.__clipboardWrites);
  assert(disabledClipboardWrites.length === 0,
    `disabled copy-on-select wrote to the clipboard: ${JSON.stringify(disabledClipboardWrites)}`);

  await reloadWithTerminalPrefs({ completionEnabled: false });
  await page.keyboard.type("git s");
  await page.waitForTimeout(100);
  const completionDisabled = await activeCompletion.count() === 0;
  assert(completionDisabled, "disabled command completion still rendered candidates");
  await page.keyboard.press("Enter");

  await reloadWithTerminalPrefs({ semanticHighlightingEnabled: false });
  const semanticDisabled = await activeHost.evaluate((host) => ({
    state: host.dataset.terminalSemanticHighlighting,
    decorations: Number(host.dataset.terminalSemanticDecorationCount ?? "-1"),
  }));
  assert(semanticDisabled.state === "disabled" && semanticDisabled.decorations === 0,
    `disabled semantic highlighting still rendered decorations: ${JSON.stringify(semanticDisabled)}`);

  await reloadWithTerminalPrefs({
    completionEnabled: true,
    completionCommandNames: true,
    completionCommandOptions: false,
    completionCommandArgs: false,
    completionHistory: false,
    completionQuickCommands: false,
    completionTriggerChars: "3 字符",
    completionListHeight: "5 行",
    completionPreviewMode: "输入框",
  });
  await page.keyboard.type("gi");
  await page.waitForTimeout(100);
  const completionBeforeTrigger = await activeCompletion.count();
  assert(completionBeforeTrigger === 0,
    `three-character completion triggered too early: ${completionBeforeTrigger}`);
  await page.keyboard.type("t");
  await activeCompletion.waitFor();
  const completionPreferenceState = await activeCompletion.evaluate((completion) => ({
    previewMode: completion.getAttribute("data-preview-mode"),
    rows: completion.style.getPropertyValue("--terminal-completion-rows"),
    candidateCount: completion.querySelectorAll(".terminal-completion-list > button").length,
    preview: completion.querySelector(".terminal-completion-preview")?.textContent ?? "",
  }));
  assert(completionPreferenceState.previewMode === "input"
    && completionPreferenceState.rows === "5"
    && completionPreferenceState.candidateCount > 0
    && completionPreferenceState.candidateCount <= 5
    && completionPreferenceState.preview.startsWith("git"),
  `completion preferences did not drive the terminal surface: ${JSON.stringify(completionPreferenceState)}`);
  await page.keyboard.press("Escape");
  await page.keyboard.press("Enter");

  await reloadWithTerminalPrefs({
    completionCommandNames: false,
    completionCommandOptions: false,
    completionCommandArgs: false,
    completionHistory: false,
    completionQuickCommands: false,
    completionTriggerChars: "1 字符",
    semanticHighlightingEnabled: true,
  });
  await page.keyboard.type("git");
  await page.waitForTimeout(100);
  const completionSourcesDisabled = await activeCompletion.count() === 0;
  assert(completionSourcesDisabled, "disabled completion sources still rendered candidates");
  await page.keyboard.press("Enter");

  await page.evaluate(() => {
    localStorage.setItem("portmate.compat.terminalTheme", "portmate-light");
    localStorage.setItem("portmate.compat.terminalOpacity", "90");
  });
  await page.reload();
  await page.waitForFunction(() => (
    document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalTheme === "portmate-light"
    && (window.__tauriEventListeners.get("portmate-session-event") || []).length >= 2
  ));
  await emitSessionEvent(createEvent("a-light-alt-exit", "session-a", "\x1b[?1049l"));
  await emitSessionEvent(createEvent("a-light-semantic-command", "session-a", `\r\n${semanticCommand}`));
  await page.waitForFunction(() => (
    Number(document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalSemanticDecorationCount ?? "0") >= 6
  ));
  const semanticLight = await inspectSemanticRendering(activeHost, [
    "#36965a", "#347cc5", "#a8740b", "#159b8e", "#087f73", "#9757b8",
  ]);
  assert(semanticLight.renderer === "dom" && semanticLight.pixelColors.length >= 4,
    `semantic command did not render enough distinct light-theme colors: ${JSON.stringify(semanticLight)}`);

  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForFunction(() => [...document.querySelectorAll(".terminal-host")]
    .every((host) => /^\d+x\d+$/.test(host.dataset.terminalSize ?? "")));
  await page.screenshot({ path: `${screenshotPrefix}-mobile.png`, fullPage: true });
  await reloadWithTerminalPrefs({
    completionEnabled: true,
    completionCommandNames: true,
    completionCommandOptions: true,
    completionCommandArgs: true,
    completionHistory: true,
    completionQuickCommands: true,
    completionTriggerChars: "1 字符",
    completionListHeight: "7 行",
    completionPreviewMode: "无处",
  });
  await page.keyboard.press("Enter");
  await page.keyboard.type("git s");
  await activeCompletion.waitFor();
  await page.waitForTimeout(120);
  const mobileCompletionPlacement = await activeCompletion.evaluate((completion) => {
    const canvas = completion.closest(".terminal-canvas");
    const terminalRegion = completion.closest(".terminal-terminal-region");
    const completionRect = completion.getBoundingClientRect();
    const canvasRect = terminalRegion?.getBoundingClientRect();
    const cursorBottom = Number(canvas?.getAttribute("data-completion-cursor-bottom") ?? "-1");
    return {
      cursorBottom,
      canvasTop: canvasRect?.top ?? -1,
      canvasRight: canvasRect?.right ?? -1,
      canvasBottom: canvasRect?.bottom ?? -1,
      completionLeft: completionRect.left,
      completionTop: completionRect.top,
      completionRight: completionRect.right,
      completionBottom: completionRect.bottom,
    };
  });
  const mobileCursorBottom = mobileCompletionPlacement.canvasTop + mobileCompletionPlacement.cursorBottom;
  assert(mobileCompletionPlacement.cursorBottom >= 0
    && mobileCompletionPlacement.completionTop >= mobileCursorBottom - 1
    && mobileCompletionPlacement.completionTop <= mobileCursorBottom + 6
    && mobileCompletionPlacement.completionRight <= mobileCompletionPlacement.canvasRight + 1
    && mobileCompletionPlacement.completionBottom <= mobileCompletionPlacement.canvasBottom + 1,
  `mobile command completion obscured or overflowed the terminal: ${JSON.stringify(mobileCompletionPlacement)}`);
  await page.screenshot({ path: `${screenshotPrefix}-mobile-completion.png`, fullPage: true });
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
    searches: {
      ansiSearch,
      trueColorSearch,
      wideSearch,
      restoredSearch,
      replayProbeSearch,
      normalBeforeVimSearch,
      vimSearch,
      hiddenNormalSearch,
      normalAfterVimSearch,
      hiddenVimSearch,
      lessSearch,
      hiddenNormalDuringLessSearch,
      normalAfterLessSearch,
      hiddenLessSearch,
      topSearch,
      longLogSearch,
      semanticSearch,
    },
    semanticHighlighting: {
      initial: initialSemanticState,
      dark: semanticDark,
      light: semanticLight,
      alternate: semanticAlternate,
      beforePaste: semanticBeforePaste,
      afterPaste: semanticAfterPaste,
      resumed: semanticResumed,
      disabled: semanticDisabled,
      outboundTextUnchanged: semanticOutboundText === semanticOutboundCommand,
    },
    timestamps: {
      initialAlternate: initialTimestampAlternate,
      beforeRestoreAlternate: timestampAlternateBeforeRestore,
      restoredAlternate: restoredTimestampAlternate,
      restoredNormal: restoredTimestampNormal,
      semanticRestored: semanticTimestampRestored,
      afterScrollbackTrim: timestampAfterScrollbackTrim,
      cachedAfterScrollbackTrim: cachedNormalTimestampState,
      afterTrimCacheRestore: timestampAfterTrimCacheRestore,
    },
    vimPty: { bytes: vimPty.bytes, alternateScreen: true, restoredNormalScreen: true },
    lessPty: { bytes: lessPty.bytes, alternateScreen: true, restoredNormalScreen: true },
    topPty: { bytes: topPty.bytes, clearedScreen: true, restoredCursor: true },
    longLog: { lines: longLogLineCount, bytes: Buffer.byteLength(longLogText), durationMs: longLogDurationMs },
    selections: { selectedWithPreference, selectedWithoutPreference },
    copiedWithPreference,
    onlineSearches: { selection: onlineSelectionSearch, fallback: onlineFallbackSearch },
    terminalWebLink,
    triggerWebLink,
    mouseTexts,
    leakedMouseTexts,
    completionPasteBoundary: {
      beforePaste: completionBeforePaste?.includes("status") ?? false,
      paused: true,
      resumed: completionAfterPasteBoundary?.includes("status") ?? false,
    },
    completionPlacement,
    completionAfterRemoteCursorMove,
    completionUsageOnly,
    mobileCompletionPlacement,
    insertNormalMode,
    completionPreferences: {
      disabled: completionDisabled,
      triggerCharacters: 3,
      beforeTrigger: completionBeforeTrigger,
      ...completionPreferenceState,
      sourcesDisabled: completionSourcesDisabled,
    },
    disabledClipboardWrites,
    desktopLayout,
    mobileLayout,
    screenshots: [
      `${screenshotPrefix}-completion-below.png`,
      `${screenshotPrefix}-normal-cursor.png`,
      `${screenshotPrefix}-trigger-link.png`,
      `${screenshotPrefix}-desktop.png`,
      `${screenshotPrefix}-mobile.png`,
      `${screenshotPrefix}-mobile-completion.png`,
    ],
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
