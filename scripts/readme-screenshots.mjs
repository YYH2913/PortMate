import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { createServer } from "node:net";
import { resolve } from "node:path";
import { chromium } from "playwright-core";

// Documentation fixtures only: no native backend, host files, credentials, or network devices.
const outputDir = resolve(process.env.PORTMATE_README_SCREENSHOT_DIR ?? "docs/images");
const timestamp = "2026-09-05T02:42:00.123456Z";
function session(id, name, connection, group) {
  const kind = connection.kind;
  return {
    profile: {
      id, name, kind, group, tags: [kind], connection,
      terminal: {
        term: "xterm-256color", rows: 32, cols: 100, scrollback: 4096,
        fontFamily: "JetBrains Mono, monospace", fontSize: 15,
        theme: "portmate-dark", backgroundOpacity: 100,
      },
      logging: { enabled: false, raw: false, text: false, jsonl: false, redactSecrets: true, pathTemplate: "{profile}/{date}/{session}.jsonl", retentionDays: 0 },
      triggers: [],
      transfer: { sftp: kind === "ssh", scp: kind === "ssh", tftp: true, xmodem: true, ymodem: true, zmodem: true, rateLimitBytesPerSecond: null, defaultLocalDir: null },
    },
    runtime: {
      sessionId: id, paneId: `${id}:main`, status: "connected", title: name, cwd: null,
      connectedSince: timestamp, lastActivity: timestamp, lastDisconnect: null,
      lastDisconnectReason: null, activeTransport: kind,
    },
    logLines: 20, lastLine: `${name} ready`,
  };
}
const sessions = [
  session("gateway", "Gateway SSH", {
    kind: "ssh", endpoint: { host: "192.0.2.1", port: 22 }, username: "root",
    reconnect: true, reconnectDelayMs: 1000, keepaliveEnabled: true,
    keepaliveIntervalSeconds: 30, keepaliveMaxMissed: 3, tcpKeepaliveEnabled: null,
    proxy: { enabled: false, kind: "socks5", host: "", port: 1080, username: "", passwordSecretRef: null },
    passwordSecretRef: null, passphraseSecretRef: null, trustedHostKeys: [], jumps: [], identityRefs: [],
    hostKeyPolicy: { mode: "trust-on-first-use", alias: null, trustScope: "profile", allowRotation: false, checkIp: false },
    identityPolicy: { identitiesOnly: true, authOrder: ["public-key", "password"], recordSuccess: true, lastSuccessful: null },
    agentPolicy: { enabled: false, forwarding: false, offerMode: "after-profile-keys" }, tunnels: [],
  }, "Network"),
  session("uart", "Board UART", { kind: "serial", port: "/dev/ttyUSB0", baudRate: 115200, dataBits: 8, parity: "none", stopBits: 1, flowControl: "none" }, "Lab"),
  session("local", "Local Shell", { kind: "shell", program: "/bin/bash", args: [], cwd: "/workspace", env: {} }, "Local"),
];
const texts = {
  gateway: [
    "OpenWrt 24.10.2 | lab-gateway\r\n\r\n",
    "root@lab-gateway:~# uptime\r\n 10:42:01 up 3 days, load average: 0.08, 0.04, 0.01\r\n\r\n",
    "root@lab-gateway:~# ip -br address\r\nlo       UNKNOWN  127.0.0.1/8\r\nbr-lan   UP       192.0.2.1/24\r\neth1     UP       198.51.100.2/24\r\n\r\n",
    "root@lab-gateway:~# df -h /overlay\r\nFilesystem       Size  Used  Avail  Use%\r\n/dev/root        256M   42M   214M   16%\r\n\r\n",
    "root@lab-gateway:~# logread -l 4\r\nINFO network: interface lan is up\r\nINFO dnsmasq: lease 192.0.2.10 board\r\nWARN ntp: waiting for time source\r\nOK health: all services running\r\n\r\n",
    "root@lab-gateway:~# ",
  ],
  uart: [
    "U-Boot 2025.04\r\n\r\nBoard: lab-devkit\r\nDRAM:  512 MiB\r\nFlash:  32 MiB\r\n\r\n",
    "=> printenv ipaddr serverip\r\nipaddr=192.0.2.10\r\nserverip=192.0.2.2\r\n\r\n",
    "=> tftpboot ${loadaddr} firmware.bin\r\nUsing ethernet device\r\nTFTP from server 192.0.2.2\r\nFilename 'firmware.bin'\r\nLoading: ################\r\n         ################\r\ndone\r\nBytes transferred = 8388608\r\n\r\n",
    "=> ",
  ],
  local: ["operator@workstation:~/firmware$ ls\r\nfirmware.bin  release-notes.txt  sha256.txt\r\n\r\noperator@workstation:~/firmware$ "],
};
const events = Object.fromEntries(Object.entries(texts).map(([sessionId, chunks]) => [sessionId, chunks.map((text, index) => ({
  id: `docs-${sessionId}-${index}`, sessionId, paneId: `${sessionId}:main`,
  ts: `2026-09-05T02:42:${String(index + 1).padStart(2, "0")}.123456Z`,
  direction: "inbound", stream: "stdout", bytesRef: null, text, annotations: {},
}))]));
const workspace = {
  version: 4, activePaneId: "pane-main", activeId: "gateway", tabColors: {},
  root: {
    kind: "split", id: "split-main", direction: "vertical", ratio: 0.61,
    first: { kind: "pane", id: "pane-main", activeViewId: "view-gateway", views: [
      { id: "view-gateway", sessionId: "gateway", title: "Gateway SSH", color: "", keyMode: "remote" },
      { id: "view-local", sessionId: "local", title: "Local Shell", color: "", keyMode: "remote" },
    ] },
    second: { kind: "pane", id: "pane-uart", activeViewId: "view-uart", views: [
      { id: "view-uart", sessionId: "uart", title: "Board UART", color: "", keyMode: "remote" },
    ] },
  },
};
const grants = [
  { clientId: "lab-assistant", name: "Lab Assistant", scopes: ["read-sessions", "read-logs", "read-transfers", "write-input", "transfer"], allowedSessions: ["gateway", "uart"], confirmWrites: true, expiresAt: null, revokedAt: null },
  { clientId: "audit-reader", name: "Audit Reader", scopes: ["read-sessions", "read-logs"], allowedSessions: ["gateway"], confirmWrites: true, expiresAt: null, revokedAt: null },
];

const reservation = createServer();
await new Promise((done) => reservation.listen(0, "127.0.0.1", done));
const port = reservation.address().port;
await new Promise((done) => reservation.close(done));
const url = `http://127.0.0.1:${port}`;
const vite = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], { stdio: ["ignore", "pipe", "pipe"] });
let viteOutput = "";
vite.stdout.on("data", (chunk) => { viteOutput += chunk; });
vite.stderr.on("data", (chunk) => { viteOutput += chunk; });
let browser;
try {
  let ready = false;
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try { ready = (await fetch(url)).ok; } catch { /* Vite is starting. */ }
    if (ready) break;
    await new Promise((done) => setTimeout(done, 100));
  }
  assert.ok(ready, viteOutput);
  await mkdir(outputDir, { recursive: true });
  browser = await chromium.launch({ executablePath: process.env.PORTMATE_CHROME ?? "/usr/bin/google-chrome", headless: true, args: ["--no-sandbox", "--enable-unsafe-swiftshader"] });
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce", timezoneId: "Australia/Perth", locale: "en-US" });
  await context.addInitScript(({ sessions, events, workspace, grants, timestamp }) => {
    localStorage.clear();
    localStorage.setItem("portmate.workspace.v1", JSON.stringify(workspace));
    localStorage.setItem("portmate.terminalPrefs", JSON.stringify({ startupMode: "none", lockOnIdle: false, requireMasterPassword: false, semanticHighlightingEnabled: true }));
    const callbacks = new Map();
    const listeners = new Map();
    let nextId = 0;
    window.__readmeEmit = (event, payload) => {
      for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
    };
    const httpConfig = {
      listenHost: "127.0.0.1", clientHost: "127.0.0.1", port: 8787,
      allowedOrigins: ["http://127.0.0.1:8787"], clientId: "lab-assistant", trusted: false,
      allowRemote: false, remoteAccess: false, endpoint: "http://127.0.0.1:8787/mcp",
      clientEndpoint: "http://127.0.0.1:8787/mcp", tokenRef: "keychain:mcp-http-token",
      tokenAvailable: false, defaultOrigin: "http://127.0.0.1:8787",
      executable: "/opt/portmate/portmate-mcp", storePath: "/workspace/portmate-store.sqlite3", startCommand: "",
    };
    const unregister = (event, id) => {
      listeners.set(event, (listeners.get(event) ?? []).filter((item) => item !== id));
      callbacks.delete(id);
    };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: unregister };
    window.__TAURI_INTERNALS__ = {
      transformCallback: (callback) => { callbacks.set(++nextId, callback); return nextId; },
      unregisterCallback: (id) => callbacks.delete(id),
      convertFileSrc: (path) => path,
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      invoke: async (command, args = {}) => {
        if (command === "plugin:event|listen") { listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]); return args.handler; }
        if (command === "plugin:event|unlisten") { unregister(args.event, args.eventId); return null; }
        if (command === "list_sessions") return sessions;
        if (command === "tail_log") return events[args.sessionId] ?? [];
        if (command.includes("command_history")) return { revision: 1, entries: [], migrated: true };
        if (command === "list_host_keys") return { keys: [] };
        if (command === "portable_vault_status") return { exists: false, unlocked: false, path: "/workspace/credentials.stronghold" };
        if (command === "list_mcp_grants") return grants;
        if (command === "mcp_http_config") return httpConfig;
        if (command === "mcp_http_access_config") return { config: httpConfig, token: null };
        if (command === "mcp_http_runtime_status") return { phase: "stopped", endpoint: null, pid: null, startedAt: null, message: null };
        if (command === "list_files") {
          const { remote, path } = args.request;
          return (remote ? [["config", true, 0], ["logs", true, 0], ["firmware.bin", false, 8388608], ["network.conf", false, 1240]]
            : [["releases", true, 0], ["firmware.bin", false, 8388608], ["release-notes.txt", false, 3072], ["sha256.txt", false, 128]])
            .map(([name, isDir, size]) => ({ name, isDir, size, path: `${path.replace(/\/$/, "")}/${name}`, modified: timestamp }));
        }
        if (command.startsWith("list_")) return [];
        if (command.startsWith("plugin:event|") || command === "resize_session") return null;
        throw new Error(`No native operation is available in README fixtures: ${command}`);
      },
    };
  }, { sessions, events, workspace, grants, timestamp });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  // Fixtures are local-only; links and native operations cannot reach actual devices.
  await page.route("**/*", (route) => new URL(route.request().url()).origin === url ? route.continue() : route.abort());
  await page.goto(url);
  await page.waitForFunction(() => document.querySelectorAll('.terminal-host[data-terminal-ready="true"]').length === 2);
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-main"] .terminal-host')?.dataset.terminalSemanticDecorationCount > 0);
  const capture = async (name, locator = page) => {
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(180);
    await locator.screenshot({ path: resolve(outputDir, `${name}.png`), animations: "disabled" });
    console.log(`Captured ${name}.png`);
  };
  await capture("workspace");
  await page.getByRole("button", { name: "工作区", exact: true }).click();
  await page.getByRole("button", { name: "文件管理器", exact: true }).click();
  await page.getByRole("textbox", { name: "本地路径", exact: true }).fill("/workspace/firmware");
  await page.getByRole("textbox", { name: "本地路径", exact: true }).press("Enter");
  await page.getByRole("textbox", { name: "远端路径", exact: true }).fill("/tmp");
  await page.getByRole("textbox", { name: "远端路径", exact: true }).press("Enter");
  await page.getByText("release-notes.txt", { exact: true }).waitFor();
  await capture("file-manager");
  await page.getByRole("button", { name: "工具", exact: true }).click();
  await page.getByRole("button", { name: "MCP Bridge", exact: true }).click();
  await page.locator(".mcp-grant-select", { hasText: "Lab Assistant" }).click();
  await capture("mcp-grants");
  await page.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector(".mcp-dialog"));
  const pane = page.locator('[data-pane-id="pane-main"]');
  await pane.getByRole("button", { name: "对照", exact: true }).click();
  await page.evaluate(() => {
    for (let index = 0; index < 6; index += 1) {
      const text = `\r\nRX ${index + 1}: temp=24.${index}C voltage=3.30V OK\r\n`;
      const bytes = [...new TextEncoder().encode(text)];
      const event = { id: `docs-packet-${index}`, sessionId: "gateway", paneId: "gateway:main", ts: `2026-09-05T02:43:0${index}.123456Z`, direction: "inbound", stream: "stdout", text, bytesRef: null, annotations: {} };
      window.__readmeEmit("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
    }
  });
  await page.locator(".terminal-byte-inspector").first().waitFor();
  await capture("terminal-hex", pane);
  assert.deepEqual(errors, [], "README screenshots must not hide browser exceptions");
  console.log(`Screenshots written to ${outputDir}`);
} finally {
  await browser?.close();
  vite.kill("SIGTERM");
}
