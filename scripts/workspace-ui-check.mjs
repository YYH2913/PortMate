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

async function samplePngCenter(browser, png) {
  const sampleContext = await browser.newContext();
  try {
    const samplePage = await sampleContext.newPage();
    const source = `data:image/png;base64,${png.toString("base64")}`;
    await samplePage.setContent(`<canvas></canvas><img src="${source}">`);
    return await samplePage.evaluate(async () => {
      const image = document.querySelector("img");
      await image.decode();
      const canvas = document.querySelector("canvas");
      canvas.width = image.naturalWidth;
      canvas.height = image.naturalHeight;
      const context = canvas.getContext("2d");
      context.drawImage(image, 0, 0);
      return [...context.getImageData(Math.floor(canvas.width / 2), Math.floor(canvas.height / 2), 1, 1).data];
    });
  } finally {
    await sampleContext.close();
  }
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
const isoNow = new Date(recordedAt).toISOString().replace("Z", "000Z");

function createSession(id, name, kind, group, tags, connection) {
  return {
    profile: {
      id,
      name,
      kind,
      group,
      tags,
      connection: connection.kind === "ssh" || connection.kind === "tmux"
        ? { ...connection, identityRefs: connection.identityRefs ?? [] }
        : connection,
      terminal: {
        term: "xterm-256color",
        rows: 28,
        cols: 100,
        scrollback: 200000,
        fontFamily: "JetBrains Mono, monospace",
        fontSize: 13,
        theme: "portmate-dark",
        backgroundOpacity: 100,
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
    reconnect: true,
    reconnectDelayMs: 1000,
    keepaliveEnabled: true,
    keepaliveIntervalSeconds: 30,
    keepaliveMaxMissed: 3,
    tcpKeepaliveEnabled: null,
    proxy: {
      enabled: false,
      kind: "socks5",
      host: "127.0.0.1",
      port: 1080,
      username: "",
      passwordSecretRef: null,
    },
    passwordSecretRef: null,
    passphraseSecretRef: null,
    hostKeyPolicy: {
      mode: "trust-on-first-use",
      alias: null,
      trustScope: "profile",
      allowRotation: false,
      checkIp: false,
    },
    trustedHostKeys: [],
    jumps: [],
    identityRefs: [{
      id: "edge-system-key",
      label: "Initial identity",
      source: "system-file",
      fingerprintSha256: "SHA256:edge-system-key",
      path: "/home/operator/.ssh/id_ed25519",
      secretRef: null,
    }],
    identityPolicy: {
      identitiesOnly: true,
      authOrder: ["public-key", "keyboard-interactive", "password"],
      recordSuccess: true,
      lastSuccessful: null,
    },
    agentPolicy: {
      enabled: false,
      forwarding: false,
      offerMode: "after-profile-keys",
    },
    tunnels: [],
  }),
  createSession("bench-uart", "Bench UART", "serial", "Lab", ["hardware"], {
    kind: "serial",
    port: "/dev/ttyUSB0 ",
    baudRate: 115200,
    dataBits: 8,
    parity: "none",
    stopBits: 1,
    flowControl: "none",
  }),
  createSession("local-shell", "Local Shell", "shell", "Local", ["development"], {
    kind: "shell",
    program: "/bin/zsh",
    args: ["-l"],
    cwd: "/workspace",
    env: {},
  }),
];
sessions[0].runtime.lastDisconnect = new Date(recordedAt - 60_000).toISOString();
sessions[0].runtime.lastDisconnectReason = "SSH keepalive timeout";
sessions[1].runtime.lastDisconnect = "invalid";
sessions[1].runtime.lastDisconnectReason = ` serial\n  cable ${"x".repeat(300)} `;

const events = sessions.map((session) => ({
  id: `event-${session.profile.id}`,
  sessionId: session.profile.id,
  paneId: `${session.profile.id}:main`,
  ts: isoNow,
  direction: "inbound",
  stream: "stdout",
  bytesRef: null,
  text: session.profile.id === "edge-router"
    ? `${session.profile.name}\r\n$ \r\nroot@OpenWrt:~# grep -n "wireless lan" /etc/config/wireless 192.168.1.1 42\r\n`
    : `${session.profile.name}\r\n$ `,
  annotations: {},
}));

const mcpGrants = [
  {
    clientId: "ops-console",
    name: "Operations Console",
    scopes: ["read-sessions", "read-logs", "read-transfers", "read-tunnels", "write-input"],
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

const customScripts = [
  {
    id: "69c06a07-dc48-4d4e-9498-6f42b6deab21",
    name: "Inspect service",
    description: "Read service state",
    content: "systemctl status portmate",
    allowAllSessions: false,
    allowedSessionIds: ["edge-router"],
    mcpEnabled: true,
    createdAt: isoNow,
    updatedAt: isoNow,
  },
];

const mcpHttpConfig = {
  listenHost: "127.0.0.1",
  clientHost: "127.0.0.1",
  port: 8787,
  allowedOrigins: ["http://127.0.0.1:8787", "http://localhost:8787"],
  clientId: "portmate-local",
  trusted: false,
  allowRemote: false,
  remoteAccess: false,
  endpoint: "http://127.0.0.1:8787/mcp",
  clientEndpoint: "http://127.0.0.1:8787/mcp",
  tokenRef: "keychain:mcp-http-token",
  tokenAvailable: true,
  defaultOrigin: "http://127.0.0.1:8787",
  executable: "/usr/bin/portmate-mcp",
  storePath: "/home/operator/.local/share/dev.portmate.desktop/portmate-store.sqlite3",
  startCommand: "PORTMATE_STORE_PATH='/home/operator/.local/share/dev.portmate.desktop/portmate-store.sqlite3' PORTMATE_MCP_HTTP=1 PORTMATE_MCP_HTTP_ADDR='127.0.0.1:8787' PORTMATE_MCP_HTTP_ORIGINS='http://127.0.0.1:8787,http://localhost:8787' PORTMATE_MCP_CLIENT_ID='portmate-local' PORTMATE_MCP_HTTP_ALLOW_REMOTE=0 PORTMATE_MCP_TRUSTED=0 '/usr/bin/portmate-mcp' --http",
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
  await context.addInitScript(({ initialSessions, initialEvents, initialWorkspace, initialMcpGrants, initialMcpAudit, initialMcpHttpConfig, initialCustomScripts, historyTimestamp }) => {
    const deferStartupSessions = sessionStorage.getItem("portmate.workspaceUiCheck.deferStartupSessions") === "true";
    const deferStartupDomains = sessionStorage.getItem("portmate.workspaceUiCheck.deferStartupDomains") === "true";
    const recoverInactiveStartup = sessionStorage.getItem("portmate.workspaceUiCheck.recoverInactiveStartup") === "true";
    const recoverSilentSshStartup = sessionStorage.getItem("portmate.workspaceUiCheck.recoverSilentSshStartup") === "true";
    sessionStorage.removeItem("portmate.workspaceUiCheck.deferStartupSessions");
    sessionStorage.removeItem("portmate.workspaceUiCheck.deferStartupDomains");
    sessionStorage.removeItem("portmate.workspaceUiCheck.recoverInactiveStartup");
    sessionStorage.removeItem("portmate.workspaceUiCheck.recoverSilentSshStartup");
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
    if (recoverInactiveStartup) {
      const session = window.__sessions.find((item) => item.profile.id === "local-shell");
      session.runtime.status = "error";
      session.runtime.connectedSince = null;
      session.runtime.lastDisconnect = new Date().toISOString();
      session.runtime.lastDisconnectReason = "previous startup failure";
      localStorage.setItem("portmate.terminalPrefs", JSON.stringify({
        historyEnabled: true,
        historyLimit: "100",
        historyRetentionDays: "30",
        startupMode: "specific",
        startupSessions: ["local-shell", "", "", ""],
        lockOnIdle: false,
        requireMasterPassword: false,
        oneKeyCompletionEnabled: false,
      }));
    }
    if (recoverSilentSshStartup) {
      const session = window.__sessions.find((item) => item.profile.id === "edge-router");
      session.runtime.status = "error";
      session.runtime.connectedSince = null;
      session.runtime.lastDisconnect = new Date().toISOString();
      session.runtime.lastDisconnectReason = "previous startup failure";
      localStorage.setItem("portmate.terminalPrefs", JSON.stringify({
        historyEnabled: true,
        historyLimit: "100",
        historyRetentionDays: "30",
        startupMode: "specific",
        startupSessions: ["edge-router", "", "", ""],
        lockOnIdle: false,
        requireMasterPassword: false,
        oneKeyCompletionEnabled: false,
      }));
    }
    window.__events = structuredClone(initialEvents);
    window.__oneKeys = [];
    window.__hostKeys = [];
    window.__hostKeySequence = 0;
    window.__hostKeyScanMode = "unknown";
    window.__deferHostKeyMutations = false;
    window.__pendingHostKeyMutations = [];
    window.__deferProfileMutations = false;
    window.__pendingProfileMutations = [];
    window.__deferSessionProfileSaves = false;
    window.__pendingSessionProfileSaves = [];
    window.__deferSessionValidation = false;
    window.__pendingSessionValidation = [];
    window.__profileMutationFailureMode = null;
    window.__secretSequence = 0;
    window.__sessionCredentialSequence = 0;
    window.__secrets = {};
    window.__deferSecretWrites = false;
    window.__pendingSecretWrites = [];
    window.__failSecretWriteAt = 0;
    window.__failNextProfileSave = false;
    window.__deferTerminalExportDirectoryPicker = false;
    window.__pendingTerminalExportDirectoryPickers = [];
    window.__deferTerminalTextExports = false;
    window.__pendingTerminalTextExports = [];
    window.__failNextTerminalTextExport = false;
    window.__portableVault = { exists: false, unlocked: false, path: "/tmp/portmate-test-vault.stronghold" };
    window.__deferVaultMutations = false;
    window.__pendingVaultMutations = [];
    window.__migrationRecovery = null;
    window.__deferMigrationPreviews = false;
    window.__pendingMigrationPreviews = [];
    window.__deferMigrationDiagnostics = false;
    window.__pendingMigrationDiagnostics = [];
    window.__mcpGrants = structuredClone(initialMcpGrants);
    window.__customScripts = structuredClone(initialCustomScripts);
    window.__customScriptSequence = 0;
    window.__injectConcurrentCustomScriptBeforeSave = false;
    window.__deferCustomScriptRuns = false;
    window.__pendingCustomScriptRuns = [];
    window.__mcpHttpConfig = structuredClone(initialMcpHttpConfig);
    window.__mcpHttpRuntime = { phase: "stopped", endpoint: null, pid: null, startedAt: null, message: null };
    window.__deferMcpHttpRuntimeStatus = false;
    window.__pendingMcpHttpRuntimeStatuses = [];
    window.__deferMcpHttpRuntimeAction = false;
    window.__pendingMcpHttpRuntimeActions = [];
    window.__buildMcpHttpConfig = (settings) => {
      const host = settings.listenHost.trim();
      const endpointHost = host.includes(":") ? `[${host.replace(/^\[|\]$/g, "")}]` : host;
      const address = `${endpointHost}:${settings.port}`;
      const clientHost = settings.clientHost.trim().replace(/^\[|\]$/g, "");
      const clientEndpointHost = clientHost.includes(":") ? `[${clientHost}]` : clientHost;
      const origins = settings.allowedOrigins.length
        ? [...settings.allowedOrigins]
        : [`http://${endpointHost}:${settings.port}`];
      const remoteAccess = host !== "127.0.0.1" && host !== "::1";
      const remote = ` PORTMATE_MCP_HTTP_ALLOW_REMOTE=${remoteAccess ? 1 : 0}`;
      const trusted = ` PORTMATE_MCP_TRUSTED=${settings.trusted ? 1 : 0}`;
      return {
        ...window.__mcpHttpConfig,
        ...structuredClone(settings),
        allowedOrigins: origins,
        remoteAccess,
        endpoint: `http://${address}/mcp`,
        clientEndpoint: `http://${clientEndpointHost}:${settings.port}/mcp`,
        defaultOrigin: origins[0],
        startCommand: `PORTMATE_STORE_PATH='${window.__mcpHttpConfig.storePath}' PORTMATE_MCP_HTTP=1 PORTMATE_MCP_HTTP_ADDR='${address}' PORTMATE_MCP_HTTP_ORIGINS='${origins.join(",")}' PORTMATE_MCP_CLIENT_ID='${settings.clientId}'${remote}${trusted} '${window.__mcpHttpConfig.executable}' --http`,
      };
    };
    window.__transfers = [];
    window.__deferTransferMutations = false;
    window.__pendingTransferMutations = [];
    window.__commandHistory = { entries: [], migrated: false, revision: 0 };
    window.__injectCommandHistoryStartupRace = true;
    window.__clipboardText = "";
    window.__clipboardWriteFailures = 0;
    window.__closeSessionError = false;
    window.__failSessionOpenFor = recoverSilentSshStartup ? "edge-router" : "";
    window.__deferSessionOpens = false;
    window.__pendingSessionOpens = [];
    window.__deferSessionCloses = false;
    window.__pendingSessionCloses = [];
    window.__deferSessionProfileDeletes = false;
    window.__pendingSessionProfileDeletes = [];
    window.__emitSessionProfileDeleteBeforeResolve = false;
    window.__deferDetachedOwnerCommands = false;
    window.__pendingDetachedOwnerCommands = [];
    window.__detachedReattachResult = null;
    window.__deferChildWindowCreates = false;
    window.__pendingChildWindowCreates = [];
    window.__deferTerminalSends = false;
    window.__pendingTerminalSends = [];
    window.__sessionOpenErrors = {};
    window.__deferFileLoads = false;
    window.__pendingFileLoads = [];
    window.__deferFileMutations = false;
    window.__pendingFileMutations = [];
    window.__deferFileBatches = false;
    window.__pendingFileBatches = [];
    window.__deferFileProperties = false;
    window.__pendingFileProperties = [];
    window.__deferTailLogs = false;
    window.__pendingTailLogs = [];
    window.__failTailLogs = 0;
    window.__failOneKeyLists = 0;
    window.__oneKeySequence = 0;
    window.__deferOneKeyMutations = false;
    window.__pendingOneKeyMutations = [];
    window.__deferOneKeySends = false;
    window.__pendingOneKeySends = [];
    window.__deferTunnelRefresh = false;
    window.__pendingTunnelRefresh = [];
    window.__tunnels = [];
    window.__deferTunnelMutations = false;
    window.__pendingTunnelMutations = [];
    window.__tmuxStates = {};
    window.__deferTmuxReads = false;
    window.__pendingTmuxReads = [];
    window.__deferSysmon = false;
    window.__pendingSysmon = [];
    window.__serialCaptureFrames = [];
    window.__deferSerialCaptureReads = false;
    window.__pendingSerialCaptureReads = [];
    window.__deferSerialCaptureOperations = false;
    window.__pendingSerialCaptureOperations = [];
    window.__deferSerialControls = false;
    window.__pendingSerialControls = [];
    window.__deferSessionLists = deferStartupSessions;
    window.__pendingSessionLists = [];
    window.__deferTransferLists = deferStartupDomains;
    window.__pendingTransferLists = [];
    window.__deferGrantLists = deferStartupDomains;
    window.__pendingGrantLists = [];
    window.__deferGrantMutations = false;
    window.__pendingGrantMutations = [];
    window.__deferMcpApprovalResponses = false;
    window.__pendingMcpApprovalResponses = [];
    window.__logShards = [
      { path: "logs/a.txt", format: "txt", size: 32, modifiedAt: new Date().toISOString() },
      { path: "logs/b.jsonl", format: "jsonl", size: 48, modifiedAt: new Date().toISOString() },
    ];
    window.__deferLogPreviews = false;
    window.__pendingLogPreviews = [];
    window.__deferLogShardLists = false;
    window.__pendingLogShardLists = [];
    window.__deferLogMutations = false;
    window.__pendingLogMutations = [];
    window.__deferMcpHttpConfig = false;
    window.__pendingMcpHttpConfig = [];
    window.__deferMcpHttpMutations = false;
    window.__pendingMcpHttpMutations = [];
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
        writeText: async (value) => {
          if (window.__clipboardWriteFailures > 0) {
            window.__clipboardWriteFailures -= 1;
            throw new Error("simulated clipboard denial");
          }
          window.__clipboardText = String(value);
        },
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
        if (command === "plugin:webview|create_webview_window" && window.__deferChildWindowCreates) {
          return new Promise((resolve, reject) => window.__pendingChildWindowCreates.push({
            args: structuredClone(args),
            reject,
            resolve: () => resolve(null),
          }));
        }
        if (command === "plugin:dialog|save") return "/tmp/portmate-picked-terminal.txt";
        if (command === "plugin:dialog|open") {
          if (!window.__deferTerminalExportDirectoryPicker) return "/tmp/portmate-terminal-text";
          return new Promise((resolve) => window.__pendingTerminalExportDirectoryPickers.push({ resolve }));
        }
        if (command === "plugin:path|join") return args.paths.join("/").replace(/\/{2,}/g, "/");
        if (command === "list_sessions") {
          if (!window.__deferSessionLists) return window.__sessions;
          return new Promise((resolve) => {
            window.__pendingSessionLists.push({ result: structuredClone(window.__sessions), resolve });
          });
        }
        if (command === "list_command_history") {
          return structuredClone(window.__commandHistory);
        }
        if (command === "migrate_command_history") {
          if (!window.__commandHistory.migrated) {
            window.__commandHistory.entries = structuredClone(args.entries ?? []);
            window.__commandHistory.migrated = true;
            window.__commandHistory.revision += 1;
          }
          const snapshot = structuredClone(window.__commandHistory);
          window.__emitTauriEvent("portmate-command-history-updated", snapshot);
          if (window.__injectCommandHistoryStartupRace) {
            window.__injectCommandHistoryStartupRace = false;
            window.__commandHistory.entries = [
              { command: "cross-window during startup", recordedAt: Date.now() },
              ...window.__commandHistory.entries,
            ];
            window.__commandHistory.revision += 1;
            window.__emitTauriEvent(
              "portmate-command-history-updated",
              structuredClone(window.__commandHistory),
            );
          }
          return snapshot;
        }
        if (command === "record_command_history") {
          window.__commandHistory.entries = [
            { command: args.command, recordedAt: Date.now() },
            ...window.__commandHistory.entries.filter((entry) => entry.command !== args.command),
          ].slice(0, args.limit);
          window.__commandHistory.migrated = true;
          window.__commandHistory.revision += 1;
          const snapshot = structuredClone(window.__commandHistory);
          window.__emitTauriEvent("portmate-command-history-updated", snapshot);
          return snapshot;
        }
        if (command === "merge_command_history") {
          const byCommand = new Map();
          for (const entry of [...(args.entries ?? []), ...window.__commandHistory.entries]) {
            const current = byCommand.get(entry.command);
            if (!current || entry.recordedAt > current.recordedAt) byCommand.set(entry.command, entry);
          }
          window.__commandHistory.entries = [...byCommand.values()]
            .sort((left, right) => right.recordedAt - left.recordedAt)
            .slice(0, args.limit);
          window.__commandHistory.migrated = true;
          window.__commandHistory.revision += 1;
          const snapshot = structuredClone(window.__commandHistory);
          window.__emitTauriEvent("portmate-command-history-updated", snapshot);
          return snapshot;
        }
        if (command === "normalize_command_history") {
          const normalized = window.__commandHistory.entries.slice(0, args.limit);
          if (JSON.stringify(normalized) !== JSON.stringify(window.__commandHistory.entries)) {
            window.__commandHistory.entries = normalized;
            window.__commandHistory.revision += 1;
          }
          const snapshot = structuredClone(window.__commandHistory);
          window.__emitTauriEvent("portmate-command-history-updated", snapshot);
          return snapshot;
        }
        if (command === "clear_command_history") {
          window.__commandHistory.entries = [];
          window.__commandHistory.migrated = true;
          window.__commandHistory.revision += 1;
          const snapshot = structuredClone(window.__commandHistory);
          window.__emitTauriEvent("portmate-command-history-updated", snapshot);
          return snapshot;
        }
        if (command === "save_session_profile") {
          if (args.proxyPasswordUpdate?.action === "set" && args.proxyPasswordUpdate.storage !== "portable") {
            throw new Error("new proxy passwords must explicitly target Stronghold");
          }
          if (window.__failNextProfileSave) {
            window.__failNextProfileSave = false;
            throw new Error("simulated Profile save failure");
          }
          const complete = () => {
            const index = window.__sessions.findIndex((session) => session.profile.id === args.profile.id);
            const saved = index >= 0
              ? {
                ...window.__sessions[index],
                profile: structuredClone(args.profile),
              }
              : {
                profile: structuredClone(args.profile),
                runtime: {
                  sessionId: args.profile.id,
                  paneId: `${args.profile.id}:main`,
                  status: "disconnected",
                  title: args.profile.name,
                  cwd: null,
                  connectedSince: null,
                  lastActivity: null,
                  lastDisconnect: null,
                  lastDisconnectReason: null,
                  activeTransport: null,
                },
                logLines: 0,
                lastLine: "",
              };
            if (index >= 0) window.__sessions[index] = saved;
            else window.__sessions.push(saved);
            return structuredClone(saved);
          };
          if (!window.__deferSessionProfileSaves) return complete();
          const result = complete();
          return new Promise((resolve) => window.__pendingSessionProfileSaves.push({
            args: structuredClone(args),
            resolve: () => resolve(structuredClone(result)),
          }));
        }
        if (command === "save_secret") {
          if (args.request?.storage !== "portable") {
            throw new Error("new user secrets must explicitly target Stronghold");
          }
          const result = { secretRef: `stronghold:test-secret-${++window.__secretSequence}` };
          if (window.__failSecretWriteAt === window.__secretSequence) {
            window.__failSecretWriteAt = 0;
            throw new Error("simulated secret write failure");
          }
          if (!window.__deferSecretWrites) {
            window.__secrets[result.secretRef] = true;
            return result;
          }
          return new Promise((resolve) => window.__pendingSecretWrites.push({
            result,
            resolve: () => {
              window.__secrets[result.secretRef] = true;
              resolve(result);
            },
          }));
        }
        if (command === "delete_secret") {
          delete window.__secrets[args.secretRef];
          return null;
        }
        if (command === "stage_session_credentials") {
          return {
            credentialHandle: `session-credential:test-${++window.__sessionCredentialSequence}`,
            expiresInMs: 30_000,
          };
        }
        if (command === "update_client_identity") {
          const index = window.__sessions.findIndex((session) => session.profile.id === args.request.profileId);
          if (window.__profileMutationFailureMode === "rename") {
            window.__profileMutationFailureMode = null;
            window.__sessions[index].profile.name = "Externally renamed router";
            throw new Error("simulated conflicting Profile mutation");
          }
          if (window.__profileMutationFailureMode === "empty") {
            window.__profileMutationFailureMode = null;
            window.__sessions = [];
            throw new Error("simulated deleted Profile mutation");
          }
          const summary = structuredClone(window.__sessions[index]);
          const identityIndex = summary.profile.connection.identityRefs.findIndex((identity) => identity.id === args.request.identityId);
          summary.profile.connection.identityRefs[identityIndex] = {
            ...summary.profile.connection.identityRefs[identityIndex],
            label: args.request.label,
            source: args.request.source,
            fingerprintSha256: args.request.fingerprintSha256,
            path: args.request.path,
            secretRef: args.request.secretRef,
          };
          window.__sessions[index] = summary;
          const result = {
            summary: structuredClone(summary),
            oldSecretDeleted: false,
            oldSecretShared: false,
            cleanupWarning: null,
          };
          if (!window.__deferProfileMutations) return result;
          return new Promise((resolve) => window.__pendingProfileMutations.push({ result, resolve }));
        }
        if (command === "tail_log") {
          if (window.__failTailLogs > 0) {
            window.__failTailLogs -= 1;
            throw new Error("simulated tail_log failure");
          }
          if (!window.__deferTailLogs) return window.__events.filter((event) => event.sessionId === args.sessionId);
          return new Promise((resolve) => {
            window.__pendingTailLogs.push({ args: structuredClone(args), resolve });
          });
        }
        if (command === "list_transfers") {
          const result = structuredClone(window.__transfers);
          if (!window.__deferTransferLists) return result;
          return new Promise((resolve) => window.__pendingTransferLists.push({ result, resolve }));
        }
        if (command === "start_transfer") {
          const request = structuredClone(args.request);
          const task = {
            id: `transfer-${window.__invokeCalls.filter((call) => call.command === "start_transfer").length}`,
            ...request,
            bytesTotal: 0,
            bytesDone: 0,
            status: "queued",
            message: "queued",
            startedAt: null,
            finishedAt: null,
            averageBytesPerSecond: null,
          };
          const complete = () => {
            window.__transfers.push(task);
            return structuredClone(task);
          };
          if (!window.__deferTransferMutations) return complete();
          return new Promise((resolve) => window.__pendingTransferMutations.push({
            command,
            args: structuredClone(args),
            result: structuredClone(task),
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "retry_transfer" || command === "cancel_transfer") {
          const existing = window.__transfers.find((task) => task.id === args.transferId);
          const retrying = command === "retry_transfer";
          const task = {
            ...(existing ?? {
              id: args.transferId,
              sessionId: "edge-router",
              protocol: "sftp",
              source: `/tmp/${args.transferId}.bin`,
              destination: `/srv/${args.transferId}.bin`,
              bytesTotal: 1024,
              bytesDone: 0,
              startedAt: null,
              finishedAt: null,
              averageBytesPerSecond: null,
            }),
            ...(retrying ? {
              id: `retry-${args.transferId}-${window.__invokeCalls.filter((call) => call.command === "retry_transfer").length}`,
              bytesDone: 0,
              status: "queued",
              message: "queued for retry",
              startedAt: null,
              finishedAt: null,
              averageBytesPerSecond: null,
            } : {
              status: "cancelled",
              message: "cancelled",
            }),
          };
          const complete = () => {
            const index = window.__transfers.findIndex((item) => item.id === task.id);
            if (index >= 0) window.__transfers[index] = task;
            else window.__transfers.push(task);
            return structuredClone(task);
          };
          if (!window.__deferTransferMutations) return complete();
          return new Promise((resolve) => window.__pendingTransferMutations.push({
            command,
            args: structuredClone(args),
            result: structuredClone(task),
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "list_serial_capture") {
          const afterIndex = args.afterId
            ? window.__serialCaptureFrames.findIndex((frame) => frame.id === args.afterId)
            : -1;
          const reset = Boolean(args.afterId) && afterIndex < 0;
          const frames = !args.afterId || reset
            ? window.__serialCaptureFrames
            : window.__serialCaptureFrames.slice(afterIndex + 1);
          const result = {
            frames: structuredClone(frames),
            reset,
            totalFrames: window.__serialCaptureFrames.length,
            capturedBytes: window.__serialCaptureFrames.reduce((total, frame) => total + frame.originalLength, 0),
          };
          if (!window.__deferSerialCaptureReads) return result;
          return new Promise((resolve) => window.__pendingSerialCaptureReads.push({
            args: structuredClone(args),
            result,
            resolve,
          }));
        }
        if (command === "list_serial_capture_history") {
          return { frames: [], enabled: false, totalFrames: 0, capturedBytes: 0, droppedFrames: 0, unavailableFrames: 0 };
        }
        if (command === "clear_serial_capture") {
          const complete = () => {
            window.__serialCaptureFrames = [];
            return { frames: [], reset: false, totalFrames: 0, capturedBytes: 0 };
          };
          if (!window.__deferSerialCaptureOperations) return complete();
          return new Promise((resolve) => window.__pendingSerialCaptureOperations.push({
            command,
            args: structuredClone(args),
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "export_serial_capture" || command === "export_serial_capture_history") {
          const complete = () => ({
            path: "/tmp/portmate-serial-capture.txt",
            checksumPath: "/tmp/portmate-serial-capture.txt.sha256",
            sha256: "d".repeat(64),
            size: 64,
            frames: args.request.frameIds.length,
            capturedBytes: 3,
            truncatedFrames: 0,
          });
          if (!window.__deferSerialCaptureOperations) return complete();
          return new Promise((resolve) => window.__pendingSerialCaptureOperations.push({
            command,
            args: structuredClone(args),
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "export_terminal_text") {
          if (window.__failNextTerminalTextExport) {
            window.__failNextTerminalTextExport = false;
            throw new Error("simulated terminal text export failure");
          }
          const result = {
            path: args.request.destinationPath || "/tmp/portmate-terminal-export.txt",
            checksumPath: `${args.request.destinationPath || "/tmp/portmate-terminal-export.txt"}.sha256`,
            sha256: "b".repeat(64),
            size: new TextEncoder().encode(args.request.text).byteLength,
            sessionId: args.request.sessionId,
            viewId: args.request.viewId,
            source: args.request.source,
          };
          if (!window.__deferTerminalTextExports) return result;
          return new Promise((resolve) => window.__pendingTerminalTextExports.push({
            args: structuredClone(args),
            result: structuredClone(result),
            resolve: () => resolve(structuredClone(result)),
          }));
        }
        if (command === "list_log_shards") {
          const result = structuredClone(window.__logShards);
          if (!window.__deferLogShardLists) return result;
          return new Promise((resolve) => window.__pendingLogShardLists.push({
            result,
            resolve: () => resolve(structuredClone(result)),
          }));
        }
        if (command === "read_log_shard") {
          if (!window.__deferLogPreviews) {
            return { path: args.path, content: `preview:${args.path}`, encoding: "utf8", bytesRead: 16, truncated: false };
          }
          return new Promise((resolve) => {
            window.__pendingLogPreviews.push({ args: structuredClone(args), resolve });
          });
        }
        if (command === "archive_log_shards" || command === "export_session_bundle_archive" || command === "delete_log_shards") {
          const result = command === "archive_log_shards"
            ? {
              path: "/tmp/portmate-logs.tar.gz",
              checksumPath: "/tmp/portmate-logs.tar.gz.sha256",
              sha256: "a".repeat(64),
              size: 40,
              shards: args.request.paths.length,
              sourceBytes: 80,
            }
            : command === "export_session_bundle_archive"
              ? {
                path: "/tmp/portmate-session.tar.gz",
                checksumPath: "/tmp/portmate-session.tar.gz.sha256",
                signaturePath: "/tmp/portmate-session.tar.gz.sig",
                sha256: "b".repeat(64),
                signatureAlgorithm: "Ed25519",
                signingPublicKey: "c".repeat(64),
                size: 64,
                files: 4,
                rawLogSegments: 0,
                attachments: args.request.attachmentPaths.length,
                redacted: args.request.redactSecrets,
                warnings: [],
              }
              : {
                deleted: args.paths.length,
                bytesDeleted: window.__logShards
                  .filter((shard) => args.paths.includes(shard.path))
                  .reduce((total, shard) => total + shard.size, 0),
              };
          const complete = () => {
            if (command === "delete_log_shards") {
              window.__logShards = window.__logShards.filter((shard) => !args.paths.includes(shard.path));
            }
            return structuredClone(result);
          };
          if (!window.__deferLogMutations) return complete();
          return new Promise((resolve) => window.__pendingLogMutations.push({
            command,
            args: structuredClone(args),
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "list_files") {
          if (!window.__deferFileLoads) return [];
          return new Promise((resolve) => {
            window.__pendingFileLoads.push({ args: structuredClone(args), resolve });
          });
        }
        if (["create_directory", "create_file", "delete_paths", "rename_path", "move_paths", "chmod_path"].includes(command)) {
          if (!window.__deferFileMutations) return null;
          return new Promise((resolve, reject) => window.__pendingFileMutations.push({
            command,
            args: structuredClone(args),
            resolve: () => resolve(null),
            reject,
          }));
        }
        if (command === "start_file_batch") {
          const request = structuredClone(args.request);
          const result = {
            tasks: request.paths.map((path, index) => ({
              id: `file-batch-${window.__invokeCalls.filter((call) => call.command === "start_file_batch").length}-${index}`,
              sessionId: request.sessionId,
              protocol: "sftp",
              source: request.sourceRemote ? `remote:${path}` : path,
              destination: request.destinationRemote ? `remote:${request.destination}` : request.destination,
              bytesTotal: 32,
              bytesDone: 0,
              status: "queued",
              message: "queued",
              startedAt: null,
              finishedAt: null,
              averageBytesPerSecond: null,
            })),
            skipped: [],
            directoriesPrepared: 0,
            totalBytes: request.paths.length * 32,
          };
          if (!window.__deferFileBatches) return structuredClone(result);
          return new Promise((resolve) => window.__pendingFileBatches.push({
            command,
            args: structuredClone(args),
            result: structuredClone(result),
            resolve: () => resolve(structuredClone(result)),
          }));
        }
        if (command === "file_properties") {
          if (window.__deferFileProperties) {
            return new Promise((resolve) => {
              window.__pendingFileProperties.push({ args: structuredClone(args), resolve });
            });
          }
          const path = args.request.path;
          return { name: path.split("/").at(-1), path, remote: args.request.remote, kind: "file", isDir: false, isFile: true, isSymlink: false, size: 0 };
        }
        if (command === "list_tunnels") {
          const result = structuredClone(window.__tunnels.filter((tunnel) => (
            tunnel.spec.sessionId === args.sessionId
          )));
          if (!window.__deferTunnelRefresh) return result;
          return new Promise((resolve) => {
            window.__pendingTunnelRefresh.push({ args: structuredClone(args), result, resolve });
          });
        }
        if (command === "create_tunnel") {
          const request = structuredClone(args.request);
          const spec = {
            id: `tunnel-${window.__invokeCalls.filter((call) => call.command === "create_tunnel").length}`,
            label: request.mode === "dynamic"
              ? `${request.bindHost}:${request.bindPort} -> SOCKS5`
              : `${request.bindHost}:${request.bindPort} -> ${request.targetHost}:${request.targetPort}`,
            ...request,
            enabled: true,
          };
          const status = {
            spec,
            activeConnections: 0,
            totalConnections: 0,
            tcpToSshBytes: 0,
            sshToTcpBytes: 0,
            lastActivity: null,
            lastError: null,
          };
          const complete = () => {
            window.__tunnels.push(status);
            return structuredClone(spec);
          };
          if (!window.__deferTunnelMutations) return complete();
          return new Promise((resolve) => window.__pendingTunnelMutations.push({
            command,
            args: structuredClone(args),
            result: structuredClone(spec),
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "stop_tunnel") {
          const existing = window.__tunnels.find((tunnel) => tunnel.spec.id === args.tunnelId) ?? null;
          const complete = () => {
            window.__tunnels = window.__tunnels.filter((tunnel) => tunnel.spec.id !== args.tunnelId);
            return structuredClone(existing);
          };
          if (!window.__deferTunnelMutations) return complete();
          return new Promise((resolve) => window.__pendingTunnelMutations.push({
            command,
            args: structuredClone(args),
            result: structuredClone(existing),
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "list_tmux_state") {
          const result = structuredClone(window.__tmuxStates[args.sessionId] ?? {
            sessions: [],
            windows: [],
            panes: [],
          });
          if (!window.__deferTmuxReads) return result;
          return new Promise((resolve) => window.__pendingTmuxReads.push({
            args: structuredClone(args),
            result,
            resolve: () => resolve(structuredClone(result)),
          }));
        }
        if (command === "list_sysmon_history" || command === "refresh_sysmon") {
          if (!window.__deferSysmon) return command === "list_sysmon_history" ? [] : null;
          return new Promise((resolve) => {
            window.__pendingSysmon.push({ command, args: structuredClone(args), resolve });
          });
        }
        if (command === "check_ssh_health") {
          const result = {
            sessionId: args.sessionId,
            runtimeId: "runtime-health-check",
            checkedAt: new Date().toISOString(),
            status: "healthy",
            backend: "russh",
            authenticationMethod: "public-key",
            terminalChannelOpen: true,
            transportRoundTripMs: 7,
            channelRoundTripMs: 11,
            sftpRoundTripMs: args.probeSftp ? 13 : null,
            transportError: null,
            terminalError: null,
            channelError: null,
            sftpError: null,
            sftpProbed: Boolean(args.probeSftp),
          };
          if (!window.__deferSessionValidation) return result;
          return new Promise((resolve) => window.__pendingSessionValidation.push({
            command,
            args: structuredClone(args),
            resolve: () => resolve(structuredClone(result)),
          }));
        }
        if (command === "list_mcp_grants") {
          const result = structuredClone(window.__mcpGrants);
          if (!window.__deferGrantLists) return result;
          return new Promise((resolve) => window.__pendingGrantLists.push({ result, resolve }));
        }
        if (command === "list_custom_scripts") return structuredClone(window.__customScripts);
        if (command === "save_custom_script") {
          const request = structuredClone(args.request);
          const existing = window.__customScripts.find((script) => script.id === request.id);
          if (existing && existing.updatedAt !== request.expectedUpdatedAt) {
            throw new Error("custom script changed in another window");
          }
          if (window.__injectConcurrentCustomScriptBeforeSave) {
            window.__injectConcurrentCustomScriptBeforeSave = false;
            const concurrentSequence = ++window.__customScriptSequence;
            const concurrentNow = new Date(Date.now() + concurrentSequence).toISOString();
            window.__customScripts.push({
              id: `00000000-0000-4000-8000-${String(concurrentSequence).padStart(12, "0")}`,
              name: "Concurrent window script",
              description: "Created in another window",
              content: "hostname",
              allowAllSessions: false,
              allowedSessionIds: ["edge-router"],
              mcpEnabled: false,
              createdAt: concurrentNow,
              updatedAt: concurrentNow,
            });
          }
          const now = new Date(Date.now() + ++window.__customScriptSequence).toISOString();
          const saved = {
            id: existing?.id ?? `00000000-0000-4000-8000-${String(window.__customScriptSequence).padStart(12, "0")}`,
            name: request.name.trim(),
            description: request.description.trim(),
            content: request.content.replace(/\r\n?/g, "\n"),
            allowAllSessions: request.allowAllSessions,
            allowedSessionIds: request.allowAllSessions ? [] : [...new Set(request.allowedSessionIds)],
            mcpEnabled: request.mcpEnabled,
            createdAt: existing?.createdAt ?? now,
            updatedAt: now,
          };
          const index = window.__customScripts.findIndex((script) => script.id === saved.id);
          if (index >= 0) window.__customScripts[index] = saved;
          else window.__customScripts.push(saved);
          return {
            scripts: structuredClone(window.__customScripts),
            savedId: saved.id,
          };
        }
        if (command === "delete_custom_script") {
          const existing = window.__customScripts.find((script) => script.id === args.request.id);
          if (!existing || existing.updatedAt !== args.request.expectedUpdatedAt) {
            throw new Error("custom script changed in another window");
          }
          window.__customScripts = window.__customScripts.filter((script) => script.id !== args.request.id);
          return structuredClone(window.__customScripts);
        }
        if (command === "run_custom_script") {
          const script = window.__customScripts.find((item) => item.id === args.request.scriptId);
          if (!script) throw new Error("unknown custom script");
          if (script.updatedAt !== args.request.expectedUpdatedAt) {
            throw new Error("custom script changed in another window; refresh and try again");
          }
          const event = {
            id: `script-event-${window.__invokeCalls.length}`,
            sessionId: args.request.sessionId,
            paneId: `${args.request.sessionId}:main`,
            ts: new Date().toISOString(),
            direction: "outbound",
            stream: "stdin",
            bytesRef: null,
            text: null,
            annotations: { customScriptId: script.id },
          };
          window.__events.push(event);
          const result = structuredClone(event);
          if (!window.__deferCustomScriptRuns) return result;
          return new Promise((resolve) => window.__pendingCustomScriptRuns.push({ result, resolve }));
        }
        if (command === "list_one_keys") {
          if (window.__failOneKeyLists > 0) {
            window.__failOneKeyLists -= 1;
            throw new Error("simulated list_one_keys failure");
          }
          return structuredClone(window.__oneKeys);
        }
        if (command === "send_one_key") {
          if (!window.__deferOneKeySends) return null;
          return new Promise((resolve) => window.__pendingOneKeySends.push({ resolve }));
        }
        if (command === "save_one_key") {
          const request = args.request;
          for (const update of [request.passwordUpdate, request.passphraseUpdate]) {
            if (update.action === "set" && update.storage !== "portable") {
              throw new Error("new OneKey secrets must explicitly target Stronghold");
            }
          }
          const existing = window.__oneKeys.find((item) => item.id === request.id);
          const id = request.id || `one-key-${++window.__oneKeySequence}`;
          const now = new Date().toISOString();
          const secretPresent = (update, current) => (
            update.action === "set" ? true : update.action === "clear" ? false : current
          );
          const saved = {
            id,
            label: request.label,
            kind: request.kind,
            username: request.username,
            hasPassword: secretPresent(request.passwordUpdate, existing?.hasPassword ?? false),
            hasPassphrase: secretPresent(request.passphraseUpdate, existing?.hasPassphrase ?? false),
            identity: existing?.identity ?? null,
            sessionIds: [...request.sessionIds],
            createdAt: existing?.createdAt ?? now,
            updatedAt: now,
          };
          const index = window.__oneKeys.findIndex((item) => item.id === id);
          if (index >= 0) window.__oneKeys[index] = saved;
          else window.__oneKeys.push(saved);
          const result = { items: structuredClone(window.__oneKeys), savedId: id };
          if (!window.__deferOneKeyMutations) return result;
          return new Promise((resolve) => window.__pendingOneKeyMutations.push({ result, resolve }));
        }
        if (command === "delete_one_key") {
          window.__oneKeys = window.__oneKeys.filter((item) => item.id !== args.request.id);
          const result = structuredClone(window.__oneKeys);
          if (!window.__deferOneKeyMutations) return result;
          return new Promise((resolve) => window.__pendingOneKeyMutations.push({ result, resolve }));
        }
        if (command === "list_host_keys") return { keys: structuredClone(window.__hostKeys) };
        if (command === "scan_ssh_host_key") {
          const profile = args.request.profile;
          const mismatch = window.__hostKeyScanMode === "mismatch";
          const observation = {
            host: profile.connection.endpoint.host,
            port: profile.connection.endpoint.port,
            alias: profile.connection.hostKeyPolicy.alias || profile.id,
            algorithm: "ssh-ed25519",
            publicKeyBase64: mismatch ? "U0NBTi1TRUNPTkQ=" : "U0NBTi1GSVJTVA==",
          };
          const fingerprint = mismatch ? "SHA256:scan-second" : "SHA256:scan-first";
          const expected = window.__hostKeys.filter((key) => (
            key.profileId === profile.id
              && key.alias === observation.alias
              && key.port === observation.port
              && key.algorithm === observation.algorithm
          ));
          const result = {
            label: "目标 SSH",
            observation,
            evaluation: mismatch
              ? {
                status: "mismatch",
                alias: observation.alias,
                host: observation.host,
                port: observation.port,
                algorithm: observation.algorithm,
                expected: structuredClone(expected),
                observedFingerprintSha256: fingerprint,
                reason: "simulated host key rotation",
              }
              : {
                status: "unknown",
                alias: observation.alias,
                host: observation.host,
                port: observation.port,
                algorithm: observation.algorithm,
                fingerprintSha256: fingerprint,
                reason: "simulated first observation",
              },
          };
          if (!window.__deferSessionValidation) return result;
          return new Promise((resolve) => window.__pendingSessionValidation.push({
            command,
            args: structuredClone(args),
            resolve: () => resolve(structuredClone(result)),
          }));
        }
        if (command === "trust_scanned_host_key") {
          const complete = () => {
            const { profile, observation, decision } = args.request;
            if (decision === "replace-for-profile") {
              window.__hostKeys = window.__hostKeys.filter((key) => !(
                key.profileId === profile.id
                  && key.alias === observation.alias
                  && key.port === observation.port
                  && key.algorithm === observation.algorithm
              ));
            }
            const now = new Date().toISOString();
            const key = {
              id: `host-key-${++window.__hostKeySequence}`,
              profileId: profile.id,
              alias: observation.alias || profile.connection.hostKeyPolicy.alias || profile.id,
              host: observation.host,
              port: observation.port,
              algorithm: observation.algorithm,
              fingerprintSha256: observation.publicKeyBase64 === "U0NBTi1TRUNPTkQ="
                ? "SHA256:scan-second"
                : "SHA256:scan-first",
              publicKeyBase64: observation.publicKeyBase64,
              scope: decision === "append-to-project" ? "project" : "profile",
              label: "scanned host key",
              firstSeen: now,
              lastSeen: now,
            };
            window.__hostKeys.push(key);
            return structuredClone(key);
          };
          if (!window.__deferSessionValidation) return complete();
          return new Promise((resolve) => window.__pendingSessionValidation.push({
            command,
            args: structuredClone(args),
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "list_ssh_agent_identities") return [];
        if (command === "portable_vault_status") return structuredClone(window.__portableVault);
        if (command === "unlock_portable_vault") {
          const result = { ...window.__portableVault, exists: true, unlocked: true };
          if (!window.__deferVaultMutations) {
            window.__portableVault = result;
            return structuredClone(result);
          }
          return new Promise((resolve) => window.__pendingVaultMutations.push({
            resolve: () => {
              window.__portableVault = result;
              resolve(structuredClone(result));
            },
          }));
        }
        if (command === "lock_portable_vault") {
          const result = { ...window.__portableVault, unlocked: false };
          if (!window.__deferVaultMutations) {
            window.__portableVault = result;
            return structuredClone(result);
          }
          return new Promise((resolve) => window.__pendingVaultMutations.push({
            resolve: () => {
              window.__portableVault = result;
              resolve(structuredClone(result));
            },
          }));
        }
        if (command === "get_profile_secret_migration_recovery") return structuredClone(window.__migrationRecovery);
        if (command === "preview_profile_secret_migration") {
          const result = {
            planToken: `preview-${window.__invokeCalls.filter((call) => call.command === command).length}`,
            targetStorage: args.request.targetStorage,
            selectedProfileCount: args.request.profileIds.length,
            affectedProfileCount: args.request.profileIds.length,
            eligibleReferenceCount: args.request.profileIds.length,
            eligibleSecretCount: args.request.profileIds.length,
            retainedSharedSecretCount: 0,
            retainedInFlightSecretCount: 0,
            alreadyTargetReferenceCount: 0,
            excludedReservedReferenceCount: 0,
          };
          if (!window.__deferMigrationPreviews) return structuredClone(result);
          return new Promise((resolve) => window.__pendingMigrationPreviews.push({
            args: structuredClone(args),
            resolve: () => resolve(structuredClone(result)),
          }));
        }
        if (command === "export_profile_secret_migration_diagnostics") {
          const result = {
            path: "/tmp/portmate-migration-diagnostic.json",
            checksumPath: "/tmp/portmate-migration-diagnostic.json.sha256",
            sha256: "a".repeat(64),
            size: 512,
            migrationId: window.__migrationRecovery?.migrationId ?? null,
            journalValid: true,
            warnings: [],
          };
          if (!window.__deferMigrationDiagnostics) return structuredClone(result);
          return new Promise((resolve) => window.__pendingMigrationDiagnostics.push({
            resolve: () => resolve(structuredClone(result)),
          }));
        }
        if (command === "import_known_hosts") {
          const [alias = `host-${window.__hostKeySequence + 1}`, algorithm = "ssh-ed25519", publicKeyBase64 = "AAAA"] = args.request.contents.trim().split(/\s+/);
          const id = `host-key-${++window.__hostKeySequence}`;
          const now = new Date().toISOString();
          window.__hostKeys.push({
            id,
            profileId: args.request.profileId,
            alias,
            host: alias,
            port: 22,
            algorithm,
            fingerprintSha256: `SHA256:${id}`,
            publicKeyBase64,
            scope: "profile",
            label: null,
            firstSeen: now,
            lastSeen: now,
          });
          const result = { keys: structuredClone(window.__hostKeys) };
          if (!window.__deferHostKeyMutations) return result;
          return new Promise((resolve) => window.__pendingHostKeyMutations.push({ result, resolve }));
        }
        if (command === "update_host_key") {
          const index = window.__hostKeys.findIndex((key) => key.id === args.request.keyId);
          window.__hostKeys[index] = {
            ...window.__hostKeys[index],
            profileId: args.request.profileId,
            alias: args.request.alias,
            host: args.request.host,
            port: args.request.port,
            scope: args.request.scope,
            label: args.request.label,
          };
          const result = { keys: structuredClone(window.__hostKeys) };
          if (!window.__deferHostKeyMutations) return result;
          return new Promise((resolve) => window.__pendingHostKeyMutations.push({ result, resolve }));
        }
        if (command === "list_mcp_audit") return initialMcpAudit;
        if (command === "mcp_http_config") {
          if (!window.__deferMcpHttpConfig) return structuredClone(window.__mcpHttpConfig);
          return new Promise((resolve) => window.__pendingMcpHttpConfig.push({ resolve }));
        }
        if (command === "preview_mcp_http_config") return window.__buildMcpHttpConfig(args.settings);
        if (command === "save_mcp_http_settings") {
          window.__mcpHttpConfig = window.__buildMcpHttpConfig(args.settings);
          const result = structuredClone(window.__mcpHttpConfig);
          if (!window.__deferMcpHttpMutations) return result;
          return new Promise((resolve) => window.__pendingMcpHttpMutations.push({ result, resolve }));
        }
        if (command === "mcp_http_runtime_status") {
          const result = structuredClone(window.__mcpHttpRuntime);
          if (!window.__deferMcpHttpRuntimeStatus) return result;
          return new Promise((resolve) => window.__pendingMcpHttpRuntimeStatuses.push({ result, resolve }));
        }
        if (command === "start_mcp_http") {
          window.__mcpHttpRuntime = {
            phase: "running",
            endpoint: window.__mcpHttpConfig.endpoint,
            pid: 4242,
            startedAt: new Date().toISOString(),
            message: null,
          };
          const result = structuredClone(window.__mcpHttpRuntime);
          if (!window.__deferMcpHttpRuntimeAction) return result;
          return new Promise((resolve) => window.__pendingMcpHttpRuntimeActions.push({ result, resolve }));
        }
        if (command === "stop_mcp_http") {
          window.__mcpHttpRuntime = { phase: "stopped", endpoint: null, pid: null, startedAt: null, message: null };
          return structuredClone(window.__mcpHttpRuntime);
        }
        if (command === "list_mcp_approvals") return [];
        if (command === "respond_mcp_approval") {
          if (!window.__deferMcpApprovalResponses) return null;
          return new Promise((resolve, reject) => window.__pendingMcpApprovalResponses.push({
            args: structuredClone(args),
            reject,
            resolve,
          }));
        }
        if (command === "save_mcp_grant") {
          const index = window.__mcpGrants.findIndex((grant) => grant.clientId === args.grant.clientId);
          if (index >= 0) window.__mcpGrants[index] = structuredClone(args.grant);
          else window.__mcpGrants.push(structuredClone(args.grant));
          const result = structuredClone(window.__mcpGrants);
          if (!window.__deferGrantMutations) return result;
          return new Promise((resolve) => window.__pendingGrantMutations.push({ result, resolve }));
        }
        if (command === "revoke_mcp_grant") {
          window.__mcpGrants = window.__mcpGrants.filter((grant) => grant.clientId !== args.clientId);
          const result = structuredClone(window.__mcpGrants);
          if (!window.__deferGrantMutations) return result;
          return new Promise((resolve) => window.__pendingGrantMutations.push({ result, resolve }));
        }
        if (command === "rotate_mcp_http_token") return { config: structuredClone(window.__mcpHttpConfig), token: "portmate-test-token" };
        if (command === "export_mcp_audit") {
          return {
            path: "/tmp/portmate-mcp-audit.jsonl",
            checksumPath: "/tmp/portmate-mcp-audit.jsonl.sha256",
            sha256: "a".repeat(64),
            size: 384,
            records: args.request.recordIds.length,
          };
        }
        if (command === "send_text" || command === "send_bytes") {
          if (!window.__deferTerminalSends) return null;
          return new Promise((resolve, reject) => window.__pendingTerminalSends.push({
            command,
            args,
            reject,
            resolve,
          }));
        }
        if (command === "serial_set_lines") {
          const index = window.__sessions.findIndex((session) => session.profile.id === args.request.sessionId);
          if (index < 0) throw new Error(`unknown session: ${args.request.sessionId}`);
          const current = window.__sessions[index];
          const result = {
            ...current,
            profile: {
              ...current.profile,
              connection: {
                ...current.profile.connection,
                ...(args.request.dtr === undefined ? {} : { dtr: args.request.dtr }),
                ...(args.request.rts === undefined ? {} : { rts: args.request.rts }),
              },
            },
          };
          const complete = () => {
            window.__sessions[index] = structuredClone(result);
            return structuredClone(result);
          };
          if (!window.__deferSerialControls) return complete();
          return new Promise((resolve) => window.__pendingSerialControls.push({
            command,
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "serial_send_break") {
          if (!window.__deferSerialControls) return null;
          return new Promise((resolve) => window.__pendingSerialControls.push({
            command,
            resolve: () => resolve(null),
          }));
        }
        if (command === "open_session" || command === "open_session_with_one_key") {
          const sessionId = command === "open_session" ? args.request.sessionId : args.sessionId;
          if (window.__sessionOpenErrors[sessionId]) {
            throw new Error(window.__sessionOpenErrors[sessionId]);
          }
          if (window.__failSessionOpenFor === sessionId) {
            throw new Error("simulated silent startup failure");
          }
          const index = window.__sessions.findIndex((item) => item.profile.id === sessionId);
          if (index < 0) return null;
          const result = {
            ...window.__sessions[index],
            runtime: {
              ...window.__sessions[index].runtime,
              status: "connected",
              connectedSince: new Date().toISOString(),
              lastActivity: new Date().toISOString(),
            },
          };
          if (!window.__deferSessionOpens) {
            window.__sessions[index] = result;
            return structuredClone(result);
          }
          window.__sessions[index] = {
            ...window.__sessions[index],
            runtime: {
              ...window.__sessions[index].runtime,
              status: "connecting",
              connectedSince: null,
              lastActivity: new Date().toISOString(),
            },
          };
          return new Promise((resolve) => window.__pendingSessionOpens.push({ result: structuredClone(result), resolve }));
        }
        if (command === "close_session") {
          if (window.__closeSessionError) throw new Error("simulated close failure");
          const index = window.__sessions.findIndex((item) => item.profile.id === args.sessionId);
          if (index < 0) return null;
          const session = {
            ...window.__sessions[index],
            runtime: {
              ...window.__sessions[index].runtime,
              status: "disconnected",
              connectedSince: null,
              lastActivity: new Date().toISOString(),
              lastDisconnect: new Date().toISOString(),
              lastDisconnectReason: "user closed session",
            },
          };
          const complete = () => {
            window.__sessions[index] = structuredClone(session);
            return structuredClone(session);
          };
          if (!window.__deferSessionCloses) return complete();
          return new Promise((resolve) => window.__pendingSessionCloses.push({
            result: structuredClone(session),
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "delete_session_profile") {
          const deletedProfileId = args.sessionId;
          const complete = () => {
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
            window.__customScripts = window.__customScripts.map((script) => {
              if (script.allowAllSessions || !script.allowedSessionIds.includes(deletedProfileId)) return script;
              const allowedSessionIds = script.allowedSessionIds.filter((sessionId) => sessionId !== deletedProfileId);
              return {
                ...script,
                allowedSessionIds,
                mcpEnabled: allowedSessionIds.length ? script.mcpEnabled : false,
                updatedAt: new Date().toISOString(),
              };
            });
            const response = {
              deletedProfileId,
              sessions: window.__sessions,
              oneKeys: [],
              hostKeys: { keys: [] },
              grants: window.__mcpGrants,
            };
            if (window.__emitSessionProfileDeleteBeforeResolve) {
              window.__emitSessionProfileDeleteBeforeResolve = false;
              window.__emitTauriEvent(
                "portmate-session-profile-deleted",
                structuredClone(response),
              );
            }
            return response;
          };
          if (!window.__deferSessionProfileDeletes) return complete();
          return new Promise((resolve) => window.__pendingSessionProfileDeletes.push({
            deletedProfileId,
            resolve: () => resolve(complete()),
          }));
        }
        if (command === "list_serial_ports") return ["/dev/ttyUSB0"];
        if (command === "plugin:event|emit_to" && args.event === "portmate-detached-pane-command"
          && window.__deferDetachedOwnerCommands) {
          return new Promise((resolve, reject) => window.__pendingDetachedOwnerCommands.push({
            args: structuredClone(args),
            resolve: () => resolve(null),
            reject,
          }));
        }
        if (command === "plugin:event|emit_to"
          && args.event === "portmate-detached-pane-command"
          && args.payload?.action === "reattach"
          && window.__detachedReattachResult) {
          const configured = structuredClone(window.__detachedReattachResult);
          queueMicrotask(() => window.__emitTauriEvent("portmate-detached-pane-result", {
            windowId: args.payload.windowId,
            requestId: args.payload.requestId,
            action: "reattach",
            ...configured,
          }));
          return null;
        }
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
    initialCustomScripts: customScripts,
    historyTimestamp: recordedAt,
  });

  const page = await context.newPage();
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(appUrl);
  await page.waitForSelector('.terminal-host[data-terminal-size] .xterm-screen');
  await page.waitForFunction(() => {
    const host = document.querySelector(".terminal-pane.active .terminal-host");
    return host?.getAttribute("data-terminal-semantic-highlighting") === "active"
      && Number(host.getAttribute("data-terminal-semantic-decoration-count")) >= 6;
  });
  await page.waitForFunction(() => localStorage.getItem("portmate.workspacePanels.v2") !== null);
  await page.getByRole("textbox", { name: "筛选资源管理器会话", exact: true }).waitFor();
  await page.waitForFunction(() => window.__commandHistory.migrated
    && window.__commandHistory.entries.length === 3
    && JSON.parse(localStorage.getItem("portmate.commandHistory") || "null")?.entries?.[0]?.command === "cross-window during startup");
  const migratedCommandHistory = await page.evaluate(() => ({
    snapshot: structuredClone(window.__commandHistory),
    listCalls: window.__invokeCalls.filter((call) => call.command === "list_command_history").length,
    migrationCalls: window.__invokeCalls.filter((call) => call.command === "migrate_command_history").length,
    listenerCallIndex: window.__invokeCalls.findIndex((call) => call.command === "plugin:event|listen"
      && call.args.event === "portmate-command-history-updated"),
    listCallIndex: window.__invokeCalls.findIndex((call) => call.command === "list_command_history"),
    local: JSON.parse(localStorage.getItem("portmate.commandHistory") || "null"),
  }));
  assert(migratedCommandHistory.listCalls === 1
    && migratedCommandHistory.migrationCalls === 1
    && migratedCommandHistory.listenerCallIndex >= 0
    && migratedCommandHistory.listenerCallIndex < migratedCommandHistory.listCallIndex
    && migratedCommandHistory.snapshot.revision === 2
    && JSON.stringify(migratedCommandHistory.snapshot.entries) === JSON.stringify(migratedCommandHistory.local.entries),
  `legacy command history migration lost its startup event barrier: ${JSON.stringify(migratedCommandHistory)}`);

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
      connectionHealthTitle: document.querySelector(".connection-toggle")?.getAttribute("title") ?? "",
      connectionHealthDescription: document.querySelector(".connection-toggle")?.getAttribute("aria-description") ?? "",
      explorerHealthTitle: [...document.querySelectorAll(".tree-session")]
        .find((button) => button.textContent?.includes("Edge Router"))?.getAttribute("title") ?? "",
      explorerHealthDescription: [...document.querySelectorAll(".tree-session")]
        .find((button) => button.textContent?.includes("Edge Router"))?.getAttribute("aria-description") ?? "",
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
    && JSON.stringify(initial.docks.right) === JSON.stringify(["sysmon", "history"])
    && JSON.stringify(initial.docks.bottom) === JSON.stringify(["sender"]),
  `default dock layout is wrong: ${JSON.stringify(initial.docks)}`);
  assert(initial.snapshotVersion === 7
    && initial.sizes.left === null && initial.sizes.right === null && initial.sizes.bottom === null,
  `legacy panel snapshot did not migrate to bounded v7 sizes: ${JSON.stringify(initial)}`);
  assert(initial.connectionSummaryRows === 0 && initial.connectionControls === 1,
    `connection context is still duplicated: ${JSON.stringify(initial)}`);
  assert(JSON.stringify(initial.paneHeaderActions) === JSON.stringify(["断开 Edge Router"]),
    `low-frequency pane actions are still permanently visible: ${JSON.stringify(initial.paneHeaderActions)}`);
  assert(initial.connectionHealthTitle.includes("已连接")
    && initial.connectionHealthTitle.includes("上次断开")
    && initial.connectionHealthTitle.includes("SSH keepalive timeout"),
  `pane connection health is missing: ${initial.connectionHealthTitle}`);
  assert(initial.explorerHealthTitle.includes("已连接")
    && initial.explorerHealthTitle.includes("上次断开")
    && initial.explorerHealthTitle.includes("SSH keepalive timeout"),
  `resource health is missing: ${initial.explorerHealthTitle}`);
  assert(initial.connectionHealthDescription.includes("已连接")
    && initial.connectionHealthDescription.includes("SSH keepalive timeout"),
  `pane connection health description is missing: ${initial.connectionHealthDescription}`);
  assert(initial.explorerHealthDescription.includes("已连接")
    && initial.explorerHealthDescription.includes("SSH keepalive timeout"),
  `resource health description is missing: ${initial.explorerHealthDescription}`);
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
    && !initial.panels.history && !initial.panels.sysmon && !initial.panels.sender,
  `migrated v2 panel snapshot is wrong: ${JSON.stringify(initial.panels)}`);

  async function togglePanel(label) {
    await page.locator(".menu-trigger", { hasText: "工作区" }).click();
    await page.locator(".menu-popover button", { hasText: label }).click();
  }

  await page.locator('.workspace-pane-tab[data-view-id="view-edge"]').click({ button: "right" });
  const workspaceViewMenu = page.locator(".workspace-view-context-menu");
  await workspaceViewMenu.waitFor();
  const focusModeButton = page.locator('[aria-label="进入专注模式"], [aria-label="退出专注模式"]');
  const focusModeBeforeMenuShortcut = await focusModeButton.getAttribute("aria-pressed");
  await page.keyboard.press("Alt+Enter");
  assert(await focusModeButton.getAttribute("aria-pressed") === focusModeBeforeMenuShortcut
    && await workspaceViewMenu.count() === 1,
  "a workspace shortcut crossed the view context-menu layer");
  await page.keyboard.press("Escape");
  await workspaceViewMenu.waitFor({ state: "detached" });

  await page.locator('.workspace-pane-tab[data-view-id="view-edge"]').click({ button: "right" });
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
  const focusModeBeforeOutsideClick = await focusModeButton.getAttribute("aria-pressed");
  await focusModeButton.click();
  await workspaceViewMenu.waitFor({ state: "detached" });
  assert(await focusModeButton.getAttribute("aria-pressed") === focusModeBeforeOutsideClick,
    "the click that dismissed a context menu also activated the underlying control");

  await page.locator('.workspace-pane-tab[data-view-id="view-edge"]').click({ button: "right" });
  await workspaceViewMenu.waitFor();
  const popupSelectionBoundary = await page.evaluate(() => {
    const menu = document.querySelector(".workspace-view-context-menu");
    const background = document.querySelector(".workspace-pane-tab-label span:last-child")?.firstChild;
    const selection = document.getSelection();
    selection?.removeAllRanges();
    if (menu && background) {
      const range = document.createRange();
      range.selectNodeContents(background);
      selection?.addRange(range);
      document.dispatchEvent(new Event("selectionchange"));
    }
    return {
      selection: selection?.toString() ?? "",
      focusInsidePopup: Boolean(document.activeElement?.closest(".workspace-view-context-menu")),
    };
  });
  assert(popupSelectionBoundary.selection === "" && popupSelectionBoundary.focusInsidePopup,
    `a browser selection crossed the context-menu layer: ${JSON.stringify(popupSelectionBoundary)}`);
  await page.waitForFunction(() => (window.__tauriEventListeners.get("portmate-mcp-approval") || []).length > 0);
  const popupTakeoverApprovalId = await page.evaluate(() => {
    const now = Date.now();
    const approval = {
      id: "33333333-3333-4333-8333-333333333333",
      clientId: "popup-takeover-check",
      action: "run_command",
      sessionId: "edge-router",
      scope: "write-input",
      createdAt: new Date(now).toISOString(),
      expiresAt: new Date(now + 60_000).toISOString(),
    };
    window.__emitTauriEvent("portmate-mcp-approval", approval);
    return approval.id;
  });
  const popupTakeoverApproval = page.getByRole("alertdialog", { name: "MCP 写操作审批", exact: true });
  await popupTakeoverApproval.waitFor();
  await workspaceViewMenu.waitFor({ state: "detached" });
  await page.keyboard.press("Escape");
  await popupTakeoverApproval.waitFor({ state: "detached" });
  assert(await workspaceViewMenu.count() === 0,
    "a context menu resurfaced after a higher modal closed");
  assert(await page.evaluate((approvalId) => window.__invokeCalls.some((call) => (
    call.command === "respond_mcp_approval"
      && call.args.approvalId === approvalId
      && call.args.approved === false
  )), popupTakeoverApprovalId),
  "the popup takeover approval did not close through its modal boundary");

  const edgeTabLabel = page.locator('.workspace-pane-tab[data-view-id="view-edge"] .workspace-pane-tab-label');
  await edgeTabLabel.dispatchEvent("auxclick", { button: 1 });
  const middleClickRename = page.locator(".workspace-view-rename-dialog");
  await middleClickRename.waitFor();
  assert(await middleClickRename.getByLabel("视图名称", { exact: true }).inputValue() === "Edge",
    "middle-clicking a workspace view tab did not open its rename dialog");
  await middleClickRename.getByRole("button", { name: "取消", exact: true }).click();
  await middleClickRename.waitFor({ state: "detached" });

  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  const sessionMenuState = await page.locator(".menu-popover button").evaluateAll((buttons) => Object.fromEntries(
    buttons.map((button) => [button.textContent?.trim(), button.disabled]),
  ));
  assert(sessionMenuState["启动会话"] && !sessionMenuState["关闭会话"] && !sessionMenuState["会话设置"]
    && sessionMenuState["导入会话"] === false
    && !Object.hasOwn(sessionMenuState, "导入 OpenSSH 配置")
    && !Object.hasOwn(sessionMenuState, "导入 PuTTY 配置")
    && !Object.hasOwn(sessionMenuState, "导入本地 Shell"),
    `connected session menu capabilities are wrong: ${JSON.stringify(sessionMenuState)}`);
  await page.locator(".menu-trigger", { hasText: "会话" }).click();

  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "导入会话" }).click();
  const openSshImportDialog = page.locator(".session-config-import-dialog");
  await openSshImportDialog.waitFor();
  const openSshConfigInput = openSshImportDialog.getByRole("textbox", { name: "OpenSSH 配置内容", exact: true });
  await page.evaluate(() => {
    window.__pendingImportFileReads = [];
    window.__originalImportFileText = File.prototype.text;
    File.prototype.text = function deferredImportFileText() {
      const file = this;
      return new Promise((resolve, reject) => window.__pendingImportFileReads.push({
        name: file.name,
        resolve: () => window.__originalImportFileText.call(file).then(resolve, reject),
      }));
    };
  });
  const importFileInput = openSshImportDialog.locator('input[type="file"]');
  await importFileInput.setInputFiles({ name: "first.conf", mimeType: "text/plain", buffer: Buffer.from("Host first\n  HostName first.example.test") });
  await importFileInput.setInputFiles({ name: "second.conf", mimeType: "text/plain", buffer: Buffer.from("Host second\n  HostName second.example.test") });
  await page.waitForFunction(() => window.__pendingImportFileReads.length === 2);
  assert(await openSshImportDialog.getByRole("button", { name: "导入", exact: true }).isDisabled()
    && await openSshImportDialog.getByRole("button", { name: "取消", exact: true }).isDisabled()
    && await openSshImportDialog.locator(".dialog-title > button").isDisabled()
    && await openSshImportDialog.getByRole("button", { name: "PuTTY", exact: true }).isDisabled()
    && await openSshConfigInput.isDisabled()
    && await openSshImportDialog.getByRole("button", { name: "选择文件", exact: true }).isEnabled(),
  "a pending Session import file read did not lock stale preview actions");
  await page.evaluate(() => window.__pendingImportFileReads.find((pending) => pending.name === "second.conf").resolve());
  await openSshImportDialog.getByRole("checkbox", { name: "导入 second", exact: true }).waitFor();
  assert(await openSshImportDialog.getByRole("button", { name: "导入", exact: true }).isEnabled()
    && await openSshConfigInput.isEnabled(),
  "Session import controls did not recover after the latest file read completed");
  await page.evaluate(() => window.__pendingImportFileReads.find((pending) => pending.name === "first.conf").resolve());
  await page.waitForTimeout(100);
  const importFileReadState = await page.evaluate(() => {
    File.prototype.text = window.__originalImportFileText;
    return {
      sourceName: document.querySelector(".session-import-source-header > span")?.textContent,
      source: document.querySelector('[aria-label="OpenSSH 配置内容"]')?.value,
    };
  });
  assert(importFileReadState.sourceName === "second.conf"
    && importFileReadState.source.includes("second.example.test")
    && !importFileReadState.source.includes("first.example.test"),
  `an older Session import file read replaced the latest selection: ${JSON.stringify(importFileReadState)}`);
  await openSshConfigInput.fill(`Host *
  ServerAliveInterval 60
  User imported-default
  DynamicForward 1080
Host production
  HostName app.example.test
  IdentityFile ~/.ssh/id_deploy
  ProxyJump ops@bastion.example.test:2222
Host staging
  HostName staging.example.test
  Port 2203`);
  await openSshImportDialog.locator(".session-import-row").first().waitFor();
  assert(await openSshImportDialog.getByRole("button", { name: "OpenSSH", exact: true }).getAttribute("aria-pressed") === "true"
    && await openSshImportDialog.locator(".session-import-row").count() === 2
    && await openSshImportDialog.getByRole("checkbox", { name: "导入 production", exact: true }).isChecked()
    && await openSshImportDialog.getByRole("checkbox", { name: "导入 staging", exact: true }).isChecked()
    && await openSshImportDialog.getByRole("button", { name: "导入", exact: true }).isEnabled()
    && await openSshImportDialog.locator(".session-import-row", { hasText: "production" }).locator("code").textContent() === "imported-default@app.example.test:22"
    && (await openSshImportDialog.locator(".session-import-row", { hasText: "production" }).textContent()).includes("1 个转发")
    && !(await openSshImportDialog.textContent()).includes("Host * 不是字面条目"),
  "OpenSSH config import preview did not apply safe Host * defaults to literal Host entries");
  await page.screenshot({ path: `${screenshotPrefix}-openssh-import.png`, fullPage: true });
  await page.evaluate(() => {
    window.__sessionImportPrompts = [];
    window.__originalSessionImportConfirm = window.confirm;
    window.confirm = (message) => {
      window.__sessionImportPrompts.push(String(message));
      return false;
    };
  });
  await openSshImportDialog.getByRole("button", { name: "PuTTY", exact: true }).click();
  await openSshImportDialog.getByRole("button", { name: "取消", exact: true }).click();
  assert(await openSshConfigInput.inputValue().then((value) => value.includes("Host production"))
    && await openSshImportDialog.getByRole("button", { name: "OpenSSH", exact: true }).getAttribute("aria-pressed") === "true",
  "Session import discarded its OpenSSH draft after a cancelled switch or close");
  await page.evaluate(() => {
    window.confirm = (message) => {
      window.__sessionImportPrompts.push(String(message));
      return true;
    };
  });
  await openSshImportDialog.getByRole("button", { name: "PuTTY", exact: true }).click();
  await openSshImportDialog.getByRole("textbox", { name: "PuTTY 配置内容", exact: true }).waitFor();
  await openSshImportDialog.getByRole("button", { name: "取消", exact: true }).click();
  await openSshImportDialog.waitFor({ state: "detached" });
  const sessionImportPrompts = await page.evaluate(() => {
    window.confirm = window.__originalSessionImportConfirm;
    return window.__sessionImportPrompts;
  });
  assert(sessionImportPrompts.length === 3
    && sessionImportPrompts[0].includes("切换格式")
    && sessionImportPrompts[1].includes("关闭窗口")
    && sessionImportPrompts[2].includes("切换格式")
    && sessionImportPrompts.every((prompt) => !prompt.includes("production") && !prompt.includes("app.example.test")),
  `Session import draft confirmations are incomplete or expose source content: ${JSON.stringify(sessionImportPrompts)}`);

  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "导入会话" }).click();
  const puttyImportDialog = page.locator(".session-config-import-dialog");
  await puttyImportDialog.waitFor();
  await puttyImportDialog.getByRole("button", { name: "PuTTY", exact: true }).click();
  await puttyImportDialog.getByRole("textbox", { name: "PuTTY 配置内容", exact: true }).fill(`Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Gateway%20SSH]
"HostName"="gateway.example.test"
"PortNumber"=dword:0000089a
"Protocol"="ssh"
"UserName"="operator"
"ProxyMethod"=dword:00000002
"ProxyHost"="socks.example.test"
"ProxyPort"=dword:00000438

[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Bench%20Serial]
"Protocol"="serial"
"SerialLine"="COM7"
"SerialSpeed"=dword:0001c200
"SerialDataBits"=dword:00000008
"SerialStopHalfbits"=dword:00000004
"SerialParity"=dword:00000002
"SerialFlowControl"=dword:00000002`);
  await puttyImportDialog.locator(".session-import-row").first().waitFor();
  assert(await puttyImportDialog.locator(".session-import-row").count() === 2
    && await puttyImportDialog.getByRole("checkbox", { name: "导入 Gateway SSH", exact: true }).isChecked()
    && await puttyImportDialog.getByRole("checkbox", { name: "导入 Bench Serial", exact: true }).isChecked()
    && await puttyImportDialog.getByRole("button", { name: "导入", exact: true }).isEnabled()
    && (await puttyImportDialog.textContent()).includes("SSH 代理"),
  "PuTTY import preview did not preserve SSH and serial sessions");
  await page.screenshot({ path: `${screenshotPrefix}-putty-import.png`, fullPage: true });
  await puttyImportDialog.getByRole("textbox", { name: "PuTTY 配置内容", exact: true }).fill("");
  await puttyImportDialog.getByRole("button", { name: "取消", exact: true }).click();
  await puttyImportDialog.waitFor({ state: "detached" });

  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "导入会话" }).click();
  const shellImportDialog = page.locator(".session-config-import-dialog");
  await shellImportDialog.waitFor();
  await shellImportDialog.getByRole("button", { name: "Shell", exact: true }).click();
  await shellImportDialog.getByRole("textbox", { name: "Shell 列表内容", exact: true }).fill(`# /etc/shells
/bin/zsh
/usr/bin/bash
/bin/bash -l`);
  await shellImportDialog.locator(".session-import-row").first().waitFor();
  assert(await shellImportDialog.locator(".session-import-row").count() === 2
    && await shellImportDialog.getByRole("checkbox", { name: "导入 zsh", exact: true }).isChecked()
    && await shellImportDialog.getByRole("checkbox", { name: "导入 bash", exact: true }).isChecked()
    && await shellImportDialog.getByRole("button", { name: "导入", exact: true }).isEnabled()
    && (await shellImportDialog.textContent()).includes("不是可直接导入的 Shell 路径"),
  "local Shell import preview did not preserve safe shell paths and reject arguments");
  await page.screenshot({ path: `${screenshotPrefix}-shell-import.png`, fullPage: true });
  await shellImportDialog.getByRole("textbox", { name: "Shell 列表内容", exact: true }).fill("");
  await shellImportDialog.getByRole("button", { name: "取消", exact: true }).click();
  await shellImportDialog.waitFor({ state: "detached" });

  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "会话设置" }).click();
  const jumpHostDialog = page.locator(".session-settings-dialog");
  await jumpHostDialog.waitFor();
  await jumpHostDialog.getByRole("combobox", { name: "会话配置项", exact: true }).selectOption("SSH");
  const jumpGroup = jumpHostDialog.getByRole("group", { name: "Jump Host:", exact: true });
  await jumpGroup.getByRole("button", { name: "添加跳板", exact: true }).click();
  await jumpGroup.getByRole("button", { name: "添加跳板", exact: true }).click();
  const jumpRows = jumpGroup.locator(".jump-hop");
  await jumpRows.nth(0).locator('input[placeholder="host"]').fill("jump-one.example.test");
  await jumpRows.nth(0).locator('input[placeholder="password"]').fill("first-staged-secret");
  await jumpRows.nth(0).locator('button[title="保存跳板密码"]').click();
  await page.waitForFunction(() => document.querySelectorAll('.jump-hop input[placeholder="password secretRef"]')[0]?.value);
  await jumpRows.nth(1).locator('input[placeholder="host"]').fill("jump-two.example.test");
  await jumpRows.nth(1).locator('input[placeholder="password"]').fill("second-staged-secret");
  await jumpRows.nth(1).locator('button[title="保存跳板密码"]').click();
  await page.waitForFunction(() => document.querySelectorAll('.jump-hop input[placeholder="password secretRef"]')[1]?.value);
  await jumpRows.nth(1).locator('input[placeholder="passphrase"]').fill("second-draft-passphrase");
  const firstJumpSecretRef = await jumpRows.nth(0).locator('input[placeholder="password secretRef"]').inputValue();
  const secondJumpSecretRef = await jumpRows.nth(1).locator('input[placeholder="password secretRef"]').inputValue();
  assert(firstJumpSecretRef && secondJumpSecretRef && firstJumpSecretRef !== secondJumpSecretRef,
    `Jump Host secret setup did not produce distinct refs: ${firstJumpSecretRef}, ${secondJumpSecretRef}`);
  await jumpGroup.getByRole("button", { name: "删除跳板 1", exact: true }).click();
  assert(await jumpRows.count() === 1
    && await jumpRows.nth(0).locator('input[placeholder="host"]').inputValue() === "jump-two.example.test"
    && await jumpRows.nth(0).locator('input[placeholder="password secretRef"]').inputValue() === secondJumpSecretRef
    && await jumpRows.nth(0).locator('input[placeholder="passphrase"]').inputValue() === "second-draft-passphrase",
  "deleting the first Jump Host did not preserve the second hop, staged ref, and local secret draft");
  await jumpHostDialog.getByRole("button", { name: "保存", exact: true }).click();
  await jumpHostDialog.waitFor({ state: "detached" });
  const savedJumpState = await page.evaluate(({ firstSecretRef, secondSecretRef }) => ({
    jumps: window.__sessions.find((session) => session.profile.id === "edge-router")?.profile.connection.jumps ?? [],
    retainedSecrets: Object.keys(window.__secrets),
    deletedRefs: window.__invokeCalls
      .filter((call) => call.command === "delete_secret")
      .map((call) => call.args.secretRef),
    firstSecretRef,
    secondSecretRef,
  }), { firstSecretRef: firstJumpSecretRef, secondSecretRef: secondJumpSecretRef });
  assert(savedJumpState.jumps.length === 1
    && savedJumpState.jumps[0].host === "jump-two.example.test"
    && savedJumpState.jumps[0].passwordSecretRef === secondJumpSecretRef
    && savedJumpState.retainedSecrets.includes(secondJumpSecretRef)
    && !savedJumpState.retainedSecrets.includes(firstJumpSecretRef)
    && savedJumpState.deletedRefs.includes(firstJumpSecretRef)
    && !savedJumpState.deletedRefs.includes(secondJumpSecretRef),
  `deleted Jump Host or its staged Secret returned after save: ${JSON.stringify(savedJumpState)}`);

  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "会话设置" }).click();
  const reopenedJumpHostDialog = page.locator(".session-settings-dialog");
  await reopenedJumpHostDialog.waitFor();
  await reopenedJumpHostDialog.getByRole("combobox", { name: "会话配置项", exact: true }).selectOption("SSH");
  const reopenedJumpGroup = reopenedJumpHostDialog.getByRole("group", { name: "Jump Host:", exact: true });
  assert(await reopenedJumpGroup.locator(".jump-hop").count() === 1
    && await reopenedJumpGroup.locator('input[placeholder="host"]').inputValue() === "jump-two.example.test",
  "saved Jump Host deletion was not restored when reopening Session Settings");
  await reopenedJumpGroup.getByRole("button", { name: "删除跳板 1", exact: true }).click();
  await reopenedJumpHostDialog.getByRole("button", { name: "保存", exact: true }).click();
  await reopenedJumpHostDialog.waitFor({ state: "detached" });
  const clearedJumpHosts = await page.evaluate(() => (
    window.__sessions.find((session) => session.profile.id === "edge-router")?.profile.connection.jumps ?? []
  ));
  assert(clearedJumpHosts.length === 0,
    `last Jump Host returned after deletion: ${JSON.stringify(clearedJumpHosts)}`);

  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "会话设置" }).click();
  const concurrentProfileDialog = page.locator(".session-settings-dialog");
  await concurrentProfileDialog.waitFor();
  const concurrentProfileSetup = await page.evaluate(() => {
    const session = window.__sessions.find((item) => item.profile.id === "edge-router");
    const baselineGroup = session.profile.group;
    window.__sessions = window.__sessions.map((item) => item.profile.id === "edge-router"
      ? { ...item, profile: { ...item.profile, group: "Concurrent external group" } }
      : item);
    return {
      baselineGroup,
      listCalls: window.__invokeCalls.filter((call) => call.command === "list_sessions").length,
    };
  });
  await page.waitForFunction(({ listCalls }) => (
    window.__invokeCalls.filter((call) => call.command === "list_sessions").length > listCalls
  ), concurrentProfileSetup);
  await page.locator(".workspace-dock-content.panel-explorer .tree-folder", { hasText: "Concurrent external group" }).waitFor();
  await concurrentProfileDialog.getByRole("button", { name: "保存", exact: true }).click();
  await concurrentProfileDialog.waitFor({ state: "detached" });
  const concurrentProfileSave = await page.evaluate(() => {
    const calls = window.__invokeCalls.filter((call) => call.command === "save_session_profile");
    return calls.at(-1)?.args ?? null;
  });
  assert(concurrentProfileSave?.expectedProfile?.group === concurrentProfileSetup.baselineGroup
    && concurrentProfileSave?.profile?.group === concurrentProfileSetup.baselineGroup,
  `Session Settings replaced its edit-time Profile baseline with a newer poll: ${JSON.stringify({
    setup: concurrentProfileSetup,
    save: concurrentProfileSave,
  })}`);

  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  const toolMenuState = await page.locator(".menu-popover button").evaluateAll((buttons) => Object.fromEntries(
    buttons.map((button) => [button.textContent?.trim(), button.disabled]),
  ));
  assert(!toolMenuState["传输任务"] && !toolMenuState["端口转发"] && !toolMenuState.Tmux
    && toolMenuState["串口分析器"],
  `SSH tool capabilities are wrong: ${JSON.stringify(toolMenuState)}`);
  const transferOperationFixtures = [
    { id: "retry-transfer-a", status: "failed", source: "/tmp/retry-a.bin", message: "failed A" },
    { id: "retry-transfer-b", status: "cancelled", source: "/tmp/retry-b.bin", message: "cancelled B" },
    { id: "cancel-transfer-a", status: "running", source: "/tmp/cancel-a.bin", message: "running A" },
    { id: "cancel-transfer-b", status: "running", source: "/tmp/cancel-b.bin", message: "running B" },
  ].map((task) => ({
    sessionId: "edge-router",
    protocol: "sftp",
    destination: `/srv/${task.id}.bin`,
    bytesTotal: 1024,
    bytesDone: task.status === "running" ? 512 : 0,
    startedAt: task.status === "running" ? new Date().toISOString() : null,
    finishedAt: null,
    averageBytesPerSecond: task.status === "running" ? 256 : null,
    ...task,
  }));
  await page.evaluate((tasks) => {
    window.__transfers = structuredClone(tasks);
    window.__deferTransferMutations = true;
    window.__pendingTransferMutations = [];
    for (const task of tasks) window.__emitTauriEvent("portmate-transfer-task", task);
  }, transferOperationFixtures);
  await page.locator(".menu-popover button", { hasText: "传输任务" }).click();
  const transferDialog = page.locator(".transfer-dialog");
  await transferDialog.waitFor();
  const sshTransferOptions = await transferDialog.locator("select").locator("option")
    .evaluateAll((options) => options.map((option) => ({ value: option.value, label: option.textContent })));
  assert(JSON.stringify(sshTransferOptions.map((option) => option.value)) === JSON.stringify(["sftp", "scp", "xmodem", "ymodem", "zmodem"]),
    `SSH transfer capabilities are wrong: ${JSON.stringify(sshTransferOptions)}`);
  assert(await transferDialog.locator("select").inputValue() === "sftp",
    "SSH transfer dialog did not select its first enabled protocol");
  const retryTransferA = transferDialog.locator(".transfer-row", { hasText: "/tmp/retry-a.bin" });
  const retryTransferB = transferDialog.locator(".transfer-row", { hasText: "/tmp/retry-b.bin" });
  const cancelTransferA = transferDialog.locator(".transfer-row", { hasText: "/tmp/cancel-a.bin" });
  const cancelTransferB = transferDialog.locator(".transfer-row", { hasText: "/tmp/cancel-b.bin" });
  await retryTransferA.getByRole("button", { name: "重试", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingTransferMutations.length === 1);
  assert(await retryTransferA.getByRole("button", { name: "重试", exact: true }).isDisabled()
    && !await retryTransferB.getByRole("button", { name: "重试", exact: true }).isDisabled(),
  "retrying one transfer did not disable only that transfer");
  await retryTransferB.getByRole("button", { name: "重试", exact: true }).click();
  await cancelTransferA.getByRole("button", { name: "取消", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingTransferMutations.length === 3);
  assert(await cancelTransferA.getByRole("button", { name: "取消", exact: true }).isDisabled()
    && !await cancelTransferB.getByRole("button", { name: "取消", exact: true }).isDisabled(),
  "cancelling one transfer did not leave another transfer independently actionable");
  await cancelTransferB.getByRole("button", { name: "取消", exact: true }).click();
  await page.waitForFunction(() => window.__pendingTransferMutations.length === 4);
  const transferMutationCalls = await page.evaluate(() => window.__pendingTransferMutations.map((pending) => ({
    command: pending.command,
    transferId: pending.args.transferId,
  })));
  assert(JSON.stringify(transferMutationCalls) === JSON.stringify([
    { command: "retry_transfer", transferId: "retry-transfer-a" },
    { command: "retry_transfer", transferId: "retry-transfer-b" },
    { command: "cancel_transfer", transferId: "cancel-transfer-a" },
    { command: "cancel_transfer", transferId: "cancel-transfer-b" },
  ]), `transfer operations were duplicated or globally serialized: ${JSON.stringify(transferMutationCalls)}`);
  await page.evaluate(() => {
    for (const pending of window.__pendingTransferMutations.splice(0)) pending.resolve();
  });
  const transferMutationNotice = page.locator(".notice-dialog");
  await transferMutationNotice.waitFor();
  await transferMutationNotice.getByRole("button", { name: "确定", exact: true }).click();

  const retryFailedTransfersButton = transferDialog.getByRole("button", { name: "重试失败", exact: true });
  await page.waitForFunction(() => ![...document.querySelectorAll(".transfer-dialog button")]
    .find((button) => button.textContent?.trim() === "重试失败")?.disabled);
  const batchTransferCallBaseline = await page.evaluate(() => window.__invokeCalls.length);
  await retryFailedTransfersButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingTransferMutations.length === 1);
  await page.waitForTimeout(50);
  const firstBatchTransferState = await page.evaluate(() => ({
    pending: window.__pendingTransferMutations.map((item) => ({
      command: item.command,
      transferId: item.args.transferId,
    })),
    rowActionsEnabled: [...document.querySelectorAll(".transfer-row-actions button")]
      .filter((button) => button.textContent?.trim() === "重试" || button.textContent?.trim() === "取消")
      .some((button) => !button.disabled),
  }));
  assert(JSON.stringify(firstBatchTransferState.pending) === JSON.stringify([
    { command: "retry_transfer", transferId: "retry-transfer-a" },
  ]) && !firstBatchTransferState.rowActionsEnabled && await retryFailedTransfersButton.isDisabled(),
  `a duplicate transfer batch started later items concurrently: ${JSON.stringify(firstBatchTransferState)}`);
  const expectedBatchTransferIds = [
    "retry-transfer-a",
    "retry-transfer-b",
    "cancel-transfer-a",
    "cancel-transfer-b",
  ];
  for (const transferId of expectedBatchTransferIds) {
    await page.waitForFunction((expectedId) => window.__pendingTransferMutations.length === 1
      && window.__pendingTransferMutations[0].command === "retry_transfer"
      && window.__pendingTransferMutations[0].args.transferId === expectedId, transferId);
    await page.evaluate(() => window.__pendingTransferMutations.shift().resolve());
  }
  await page.waitForFunction(() => document.querySelectorAll(".notice-dialog").length === 1
    && ![...document.querySelectorAll(".transfer-dialog button")]
      .find((button) => button.textContent?.trim() === "重试失败")?.disabled);
  const batchTransferCalls = await page.evaluate((baseline) => window.__invokeCalls.slice(baseline)
    .filter((call) => call.command === "retry_transfer")
    .map((call) => call.args.transferId), batchTransferCallBaseline);
  assert(JSON.stringify(batchTransferCalls) === JSON.stringify(expectedBatchTransferIds),
    `transfer batch emitted duplicate or out-of-order retries: ${JSON.stringify(batchTransferCalls)}`);
  await transferMutationNotice.getByRole("button", { name: "确定", exact: true }).click();

  await transferDialog.locator(".dialog-field", { hasText: "来源:" }).locator("input").fill("/tmp/start-once.bin");
  await transferDialog.locator(".dialog-field", { hasText: "目标:" }).locator("input").fill("remote:/srv/start-once.bin");
  await transferDialog.evaluate((form) => {
    form.requestSubmit();
    form.requestSubmit();
  });
  await page.waitForFunction(() => window.__pendingTransferMutations.filter((pending) => pending.command === "start_transfer").length === 1);
  assert(await transferDialog.getByRole("button", { name: "执行中", exact: true }).isDisabled(),
    "pending transfer start did not disable the submit action");
  const pendingStartTransfer = await page.evaluate(() => window.__pendingTransferMutations
    .filter((pending) => pending.command === "start_transfer").length);
  assert(pendingStartTransfer === 1, `transfer start was submitted more than once: ${pendingStartTransfer}`);
  await page.screenshot({ path: `${screenshotPrefix}-transfer.png`, fullPage: true });
  await transferDialog.locator(".utility-actions button", { hasText: "取消" }).click();
  await transferDialog.waitFor({ state: "detached" });
  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.locator(".menu-popover button", { hasText: "传输任务" }).click();
  const replacementTransferDialog = page.locator(".transfer-dialog");
  await replacementTransferDialog.waitFor();
  await page.evaluate(() => {
    const pending = window.__pendingTransferMutations.find((item) => item.command === "start_transfer");
    pending.resolve();
    window.__pendingTransferMutations = [];
    window.__deferTransferMutations = false;
  });
  await page.waitForTimeout(100);
  assert(await replacementTransferDialog.count() === 1 && await page.locator(".notice-dialog").count() === 0,
    "a late transfer response mutated or obscured the replacement dialog");
  await replacementTransferDialog.locator(".utility-actions button", { hasText: "取消" }).click();
  await replacementTransferDialog.waitFor({ state: "detached" });

  await page.evaluate(() => {
    window.__deferTunnelRefresh = true;
    window.__pendingTunnelRefresh = [];
  });
  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.locator(".menu-popover button", { hasText: "端口转发" }).click();
  const tunnelDialog = page.locator(".utility-dialog", { hasText: "端口转发" });
  await tunnelDialog.waitFor();
  const tunnelBindHost = tunnelDialog.locator(".dialog-field", { hasText: "监听:" }).locator("input");
  const tunnelBindPort = tunnelDialog.locator(".dialog-field", { hasText: /^端口:/ }).locator("input");
  const tunnelTargetHost = tunnelDialog.locator(".dialog-field", { hasText: "目标:" }).locator("input");
  const tunnelTargetPort = tunnelDialog.locator(".dialog-field", { hasText: "目标端口:" }).locator("input");
  const tunnelCreate = tunnelDialog.locator(".utility-actions button", { hasText: "创建" });
  assert(await tunnelBindHost.getAttribute("maxlength") === "255"
    && await tunnelTargetHost.getAttribute("maxlength") === "255",
  "tunnel host inputs do not expose the backend character bound");
  assert(await tunnelBindPort.getAttribute("type") === "number"
    && await tunnelBindPort.getAttribute("min") === "0"
    && await tunnelBindPort.getAttribute("max") === "65535"
    && await tunnelTargetPort.getAttribute("min") === "1",
  "tunnel port inputs do not expose the backend numeric bounds");
  await tunnelBindPort.fill("65536");
  assert(await tunnelCreate.isDisabled(), "oversized tunnel bind port left Create enabled");
  await tunnelBindPort.fill("0");
  await tunnelTargetPort.fill("0");
  assert(await tunnelCreate.isDisabled(), "zero tunnel target port left Create enabled");
  await tunnelTargetPort.fill("22");
  await tunnelTargetHost.fill("bad host");
  assert(await tunnelCreate.isDisabled(), "whitespace tunnel target left Create enabled");
  await tunnelTargetHost.fill("127.0.0.1");
  assert(!await tunnelCreate.isDisabled(), "valid tunnel fields did not restore Create");

  await tunnelDialog.locator(".dialog-field", { hasText: "模式:" }).locator("select").selectOption("dynamic");
  assert(await tunnelDialog.locator(".dialog-field", { hasText: "目标:" }).count() === 0,
    "dynamic tunnel retained its fixed target fields");
  await tunnelDialog.getByRole("button", { name: "添加目标路由", exact: true }).click();
  const routeHost = tunnelDialog.getByRole("textbox", { name: "路由目标 1", exact: true });
  const routePort = tunnelDialog.getByRole("spinbutton", { name: "路由端口 1", exact: true });
  await routeHost.fill("bad..host");
  assert(await tunnelCreate.isDisabled(), "invalid dynamic route host left Create enabled");
  await routeHost.fill("10.9.8.7/8");
  await routePort.fill("0");
  assert(await tunnelCreate.isDisabled(), "zero dynamic route port left Create enabled");
  await routePort.fill("443");
  await routeHost.blur();
  assert(await routeHost.inputValue() === "10.0.0.0/8" && !await tunnelCreate.isDisabled(),
    "valid dynamic CIDR route was not normalized or accepted");
  await tunnelDialog.getByRole("button", { name: "添加目标路由", exact: true }).click();
  await tunnelDialog.getByRole("textbox", { name: "路由目标 2", exact: true }).fill("10.0.0.0/8");
  await tunnelDialog.getByRole("spinbutton", { name: "路由端口 2", exact: true }).fill("443");
  assert(await tunnelCreate.isDisabled(), "duplicate normalized dynamic route left Create enabled");
  await tunnelDialog.getByRole("button", { name: "删除目标路由 2", exact: true }).click();
  assert(!await tunnelCreate.isDisabled(), "removing duplicate dynamic route did not restore Create");
  await page.screenshot({ path: `${screenshotPrefix}-tunnel.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const mobileTunnelBounds = await tunnelDialog.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return {
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      width: rect.width,
      scrollWidth: dialog.scrollWidth,
    };
  });
  assert(mobileTunnelBounds.left >= 0 && mobileTunnelBounds.right <= 390
    && mobileTunnelBounds.top >= 0 && mobileTunnelBounds.bottom <= 844
    && mobileTunnelBounds.scrollWidth <= mobileTunnelBounds.width,
  `dynamic tunnel dialog overflows on mobile: ${JSON.stringify(mobileTunnelBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-tunnel-mobile.png`, fullPage: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.waitForFunction(() => window.__pendingTunnelRefresh.length >= 1);
  await page.waitForTimeout(100);
  const tunnelRefreshBaseline = await page.evaluate(() => window.__pendingTunnelRefresh.length);
  await page.waitForTimeout(2_150);
  assert(await page.evaluate(() => window.__pendingTunnelRefresh.length) === tunnelRefreshBaseline,
    "slow tunnel refreshes overlapped instead of waiting for the active request");
  await page.evaluate(() => {
    for (const pending of window.__pendingTunnelRefresh) pending.resolve([]);
    window.__pendingTunnelRefresh = [];
    window.__deferTunnelRefresh = false;
    window.__deferTunnelMutations = true;
    window.__pendingTunnelMutations = [];
  });
  const tunnelCreateCallsBefore = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "create_tunnel").length);
  await tunnelDialog.evaluate((form) => {
    form.requestSubmit();
    form.requestSubmit();
  });
  await page.waitForFunction(() => window.__pendingTunnelMutations
    .filter((pending) => pending.command === "create_tunnel").length === 1);
  assert(await tunnelDialog.getByRole("button", { name: "创建中", exact: true }).isDisabled(),
    "pending tunnel create did not disable the submit action");
  const tunnelCreateCallsAfter = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "create_tunnel").length);
  assert(tunnelCreateCallsAfter === tunnelCreateCallsBefore + 1,
    `tunnel create was submitted more than once: ${tunnelCreateCallsAfter - tunnelCreateCallsBefore}`);
  await page.evaluate(() => {
    const pending = window.__pendingTunnelMutations.find((item) => item.command === "create_tunnel");
    pending.resolve();
    window.__pendingTunnelMutations = [];
    window.__deferTunnelMutations = false;
  });
  await tunnelDialog.waitFor({ state: "detached" });
  const tunnelCreateRequest = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "create_tunnel").at(-1)?.args.request);
  assert(tunnelCreateRequest?.mode === "dynamic"
    && tunnelCreateRequest.targetHost === ""
    && tunnelCreateRequest.targetPort === 0
    && JSON.stringify(tunnelCreateRequest.routeRules) === JSON.stringify([
      { host: "10.0.0.0/8", port: 443 },
    ]),
  `dynamic tunnel route rules were not submitted intact: ${JSON.stringify(tunnelCreateRequest)}`);
  const tunnelNotice = page.locator(".notice-dialog", { hasText: "已创建 dynamic tunnel" });
  await tunnelNotice.waitFor();
  await tunnelNotice.getByRole("button", { name: "确定", exact: true }).click();

  const tunnelStopFixtures = ["tunnel-stop-a", "tunnel-stop-b"].map((id, index) => ({
    spec: {
      id,
      sessionId: "edge-router",
      mode: "local",
      label: `Stop tunnel ${index + 1}`,
      bindHost: "127.0.0.1",
      bindPort: 11001 + index,
      targetHost: "10.0.0.1",
      targetPort: 22,
      routeRules: [],
      enabled: true,
    },
    activeConnections: 0,
    totalConnections: 0,
    tcpToSshBytes: 0,
    sshToTcpBytes: 0,
    lastActivity: null,
    lastError: null,
  }));
  await page.evaluate((tunnels) => {
    window.__tunnels = structuredClone(tunnels);
    window.__deferTunnelMutations = true;
    window.__pendingTunnelMutations = [];
  }, tunnelStopFixtures);
  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.locator(".menu-popover button", { hasText: "端口转发" }).click();
  const tunnelStopDialog = page.locator(".utility-dialog", { hasText: "端口转发" });
  const tunnelStopA = tunnelStopDialog.locator(".tunnel-row", { hasText: "Stop tunnel 1" });
  const tunnelStopB = tunnelStopDialog.locator(".tunnel-row", { hasText: "Stop tunnel 2" });
  await tunnelStopA.waitFor();
  await tunnelStopA.getByRole("button", { name: "停止 tunnel", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingTunnelMutations.length === 1);
  assert(await tunnelStopA.getByRole("button", { name: "停止 tunnel", exact: true }).isDisabled()
    && !await tunnelStopB.getByRole("button", { name: "停止 tunnel", exact: true }).isDisabled(),
  "stopping one tunnel did not leave another tunnel independently actionable");
  await tunnelStopB.getByRole("button", { name: "停止 tunnel", exact: true }).click();
  await page.waitForFunction(() => window.__pendingTunnelMutations.length === 2);
  const tunnelStopCalls = await page.evaluate(() => window.__pendingTunnelMutations.map((pending) => ({
    command: pending.command,
    tunnelId: pending.args.tunnelId,
  })));
  assert(JSON.stringify(tunnelStopCalls) === JSON.stringify([
    { command: "stop_tunnel", tunnelId: "tunnel-stop-a" },
    { command: "stop_tunnel", tunnelId: "tunnel-stop-b" },
  ]), `tunnel stops were duplicated or globally serialized: ${JSON.stringify(tunnelStopCalls)}`);
  await tunnelStopDialog.locator(".utility-actions button", { hasText: "取消" }).click();
  await tunnelStopDialog.waitFor({ state: "detached" });
  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.locator(".menu-popover button", { hasText: "端口转发" }).click();
  const replacementTunnelDialog = page.locator(".utility-dialog", { hasText: "端口转发" });
  await replacementTunnelDialog.waitFor();
  await page.evaluate(() => {
    for (const pending of window.__pendingTunnelMutations.splice(0)) pending.resolve();
    window.__deferTunnelMutations = false;
  });
  await page.waitForTimeout(100);
  assert(await replacementTunnelDialog.count() === 1 && await page.locator(".notice-dialog").count() === 0,
    "a late tunnel stop response mutated or obscured the replacement dialog");
  await replacementTunnelDialog.locator(".utility-actions button", { hasText: "取消" }).click();
  await replacementTunnelDialog.waitFor({ state: "detached" });

  {
    const page = await context.newPage();
    const sessionBoundUtilityErrors = [];
    page.on("pageerror", (error) => sessionBoundUtilityErrors.push(error.message));
    await page.goto(`${appUrl}?workspaceWindow=1&windowId=session-bound-utility-regression`);
    const initialEdgeRouterTree = page.locator(
      ".workspace-dock-content.panel-explorer .tree-session",
      { hasText: "Edge Router" },
    );
    await initialEdgeRouterTree.waitFor();
    await initialEdgeRouterTree.click();
    await page.evaluate(() => {
      const edge = structuredClone(window.__sessions.find((session) => session.profile.id === "edge-router"));
      window.__edgeRouterFixture = edge;
      const backup = structuredClone(edge);
      backup.profile.id = "backup-router";
      backup.profile.name = "Backup Router";
      backup.runtime.sessionId = "backup-router";
      backup.runtime.paneId = "backup-router:main";
      backup.runtime.title = "Backup Router";
      window.__sessions.push(backup);
      window.__tmuxStates["edge-router"] = {
        sessions: [{ name: "edge-work", windows: 1, attached: 1, created: new Date().toISOString() }],
        windows: [{
          session: "edge-work",
          windowIndex: 0,
          windowId: "@1",
          name: "edge-window",
          panes: 1,
          active: true,
          synchronized: false,
        }],
        panes: [{
          session: "edge-work",
          windowIndex: 0,
          paneIndex: 0,
          paneId: "%1",
          active: true,
          synchronized: false,
          command: "zsh",
          title: "edge pane",
        }],
      };
      window.__tmuxStates["backup-router"] = {
        sessions: [{ name: "backup-work", windows: 1, attached: 0, created: new Date().toISOString() }],
        windows: [{
          session: "backup-work",
          windowIndex: 0,
          windowId: "@2",
          name: "backup-window",
          panes: 1,
          active: true,
          synchronized: false,
        }],
        panes: [{
          session: "backup-work",
          windowIndex: 0,
          paneIndex: 0,
          paneId: "%2",
          active: true,
          synchronized: false,
          command: "bash",
          title: "backup pane",
        }],
      };
      window.__emitTauriEvent("portmate-session-profile-updated", backup);
    });
    const backupRouterTree = page.locator(
      ".workspace-dock-content.panel-explorer .tree-session",
      { hasText: "Backup Router" },
    );
    const edgeRouterTree = page.locator(
      ".workspace-dock-content.panel-explorer .tree-session",
      { hasText: "Edge Router" },
    );
    await backupRouterTree.waitFor();
    await backupRouterTree.click();
    await edgeRouterTree.click();
    const deleteEdgeRouterExternally = async () => page.evaluate(() => {
      window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
      window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
    });
    const restoreEdgeRouter = async () => {
      await page.evaluate(() => {
        const edge = structuredClone(window.__edgeRouterFixture);
        window.__sessions.push(edge);
        window.__emitTauriEvent("portmate-session-profile-updated", edge);
      });
      await edgeRouterTree.waitFor();
      await edgeRouterTree.click();
    };

    await page.evaluate(() => {
      window.__deferTransferMutations = true;
      window.__pendingTransferMutations = [];
    });
    await page.locator(".menu-trigger", { hasText: "工具" }).click();
    await page.locator(".menu-popover button", { hasText: "传输任务" }).click();
    const switchingTransferDialog = page.locator(".transfer-dialog");
    await switchingTransferDialog.locator(".dialog-field", { hasText: "来源:" }).locator("input").fill("/tmp/edge-switch.bin");
    await switchingTransferDialog.locator(".dialog-field", { hasText: "目标:" }).locator("input").fill("remote:/tmp/edge-switch.bin");
    await switchingTransferDialog.evaluate((form) => form.requestSubmit());
    await page.waitForFunction(() => window.__pendingTransferMutations.length === 1);
    await deleteEdgeRouterExternally();
    await page.waitForFunction(() => document.querySelector(".transfer-dialog .dialog-field input")?.value === "Backup Router");
    const switchedTransferState = await switchingTransferDialog.evaluate((dialog) => ({
      source: [...dialog.querySelectorAll(".dialog-field")]
        .find((field) => field.textContent?.includes("来源:"))?.querySelector("input")?.value,
      destination: [...dialog.querySelectorAll(".dialog-field")]
        .find((field) => field.textContent?.includes("目标:"))?.querySelector("input")?.value,
      action: dialog.querySelector('.utility-actions button[type="submit"]')?.textContent?.trim(),
    }));
    assert(switchedTransferState.source === ""
      && switchedTransferState.destination === ""
      && switchedTransferState.action === "开始",
    `Transfer state leaked across sessions: ${JSON.stringify(switchedTransferState)}`);
    await page.evaluate(() => {
      window.__pendingTransferMutations.shift().resolve();
      window.__deferTransferMutations = false;
    });
    await page.waitForTimeout(100);
    assert(await page.locator(".notice-dialog").count() === 0,
      "a transfer response from the previous session produced a stale notice");
    await switchingTransferDialog.locator(".utility-actions button", { hasText: "取消" }).click();
    await switchingTransferDialog.waitFor({ state: "detached" });

    await restoreEdgeRouter();
    await page.evaluate(() => {
      window.__deferTunnelMutations = true;
      window.__pendingTunnelMutations = [];
    });
    await page.locator(".menu-trigger", { hasText: "工具" }).click();
    await page.locator(".menu-popover button", { hasText: "端口转发" }).click();
    const switchingTunnelDialog = page.locator(".utility-dialog", { hasText: "端口转发" });
    await switchingTunnelDialog.locator(".dialog-field", { hasText: "监听:" }).locator("input").fill("0.0.0.0");
    await switchingTunnelDialog.evaluate((form) => form.requestSubmit());
    await page.waitForFunction(() => window.__pendingTunnelMutations.length === 1);
    await deleteEdgeRouterExternally();
    await page.waitForFunction(() => [...document.querySelectorAll(".utility-dialog .dialog-field")]
      .find((field) => field.textContent?.includes("会话:"))?.querySelector("input")?.value === "Backup Router");
    const switchedTunnelState = await switchingTunnelDialog.evaluate((dialog) => ({
      bindHost: [...dialog.querySelectorAll(".dialog-field")]
        .find((field) => field.textContent?.includes("监听:"))?.querySelector("input")?.value,
      action: dialog.querySelector('.utility-actions button[type="submit"]')?.textContent?.trim(),
    }));
    assert(switchedTunnelState.bindHost === "127.0.0.1" && switchedTunnelState.action === "创建",
      `Tunnel state leaked across sessions: ${JSON.stringify(switchedTunnelState)}`);
    await page.evaluate(() => {
      window.__pendingTunnelMutations.shift().resolve();
      window.__deferTunnelMutations = false;
    });
    await page.waitForTimeout(100);
    assert(await page.locator(".notice-dialog").count() === 0,
      "a tunnel response from the previous session produced a stale notice");
    await switchingTunnelDialog.locator(".utility-actions button", { hasText: "取消" }).click();
    await switchingTunnelDialog.waitFor({ state: "detached" });

    await restoreEdgeRouter();
    await page.locator(".menu-trigger", { hasText: "工具" }).click();
    await page.locator(".menu-popover button", { hasText: "Tmux" }).click();
    const switchingTmuxDialog = page.locator(".tmux-dialog");
    await switchingTmuxDialog.locator('[data-tmux-session="edge-work"]').waitFor();
    await page.evaluate(() => {
      window.__deferTmuxReads = true;
      window.__pendingTmuxReads = [];
    });
    await deleteEdgeRouterExternally();
    await page.waitForFunction(() => window.__pendingTmuxReads.some((request) => (
      request.args.sessionId === "backup-router"
    )));
    assert(await switchingTmuxDialog.locator('[data-tmux-session="edge-work"]').count() === 0,
      "Tmux displayed the previous session tree while loading the replacement session");
    await page.evaluate(() => {
      for (const pending of window.__pendingTmuxReads.filter((request) => (
        request.args.sessionId === "backup-router"
      ))) pending.resolve();
      window.__pendingTmuxReads = [];
      window.__deferTmuxReads = false;
    });
    await switchingTmuxDialog.locator('[data-tmux-session="backup-work"]').waitFor();
    await switchingTmuxDialog.getByRole("button", { name: "关闭 Tmux", exact: true }).click();
    await switchingTmuxDialog.waitFor({ state: "detached" });

    await restoreEdgeRouter();
    await page.locator(".menu-trigger", { hasText: "工具" }).click();
    await page.locator(".menu-popover button", { hasText: "日志管理" }).click();
    const switchingLogManager = page.locator(".log-manager-dialog");
    await switchingLogManager.waitFor();
    const bundleSessionSelect = switchingLogManager.getByRole("combobox", { name: "导出会话", exact: true });
    assert(await bundleSessionSelect.inputValue() === "edge-router",
      "session bundle did not start from the active session");
    await page.evaluate(() => {
      window.__deferLogMutations = true;
      window.__pendingLogMutations = [];
    });
    await switchingLogManager.getByRole("button", { name: "导出会话包", exact: true }).click();
    await page.waitForFunction(() => window.__pendingLogMutations.length === 1);
    await deleteEdgeRouterExternally();
    await page.waitForFunction(() => (
      document.querySelector('select[aria-label="导出会话"]')?.value === "backup-router"
      && [...document.querySelectorAll(".log-bundle-controls button")]
        .some((button) => button.textContent?.includes("导出会话包") && !button.disabled)
    ));
    await page.evaluate(() => {
      window.__deferLogMutations = false;
      window.__pendingLogMutations.shift().resolve();
      window.__pendingLogMutations = [];
    });
    await page.waitForTimeout(100);
    assert(await page.locator(".notice-dialog").count() === 0
      && await switchingLogManager.locator(".log-bundle-result").count() === 0,
    "a session bundle response survived deletion of its Profile");
    await switchingLogManager.getByRole("button", { name: "导出会话包", exact: true }).click();
    const replacementBundleNotice = page.locator(".notice-dialog", { hasText: "会话包已导出" });
    await replacementBundleNotice.waitFor();
    const replacementBundleRequest = await page.evaluate(() => window.__invokeCalls
      .filter((call) => call.command === "export_session_bundle_archive").at(-1)?.args.request);
    assert(replacementBundleRequest?.sessionId === "backup-router",
      `session bundle retained a deleted Profile ID: ${JSON.stringify(replacementBundleRequest)}`);
    await replacementBundleNotice.getByRole("button", { name: "确定", exact: true }).click();
    await switchingLogManager.getByRole("button", { name: "关闭", exact: true }).last().click();
    await switchingLogManager.waitFor({ state: "detached" });

    await restoreEdgeRouter();
    await page.evaluate(() => {
      window.__sessions = window.__sessions.filter((session) => session.profile.id !== "backup-router");
      delete window.__tmuxStates["backup-router"];
      window.__emitTauriEvent("portmate-session-profile-deleted", "backup-router");
    });
    await backupRouterTree.waitFor({ state: "detached" });
    assert(sessionBoundUtilityErrors.length === 0,
      `session-bound utility lifecycle browser exceptions: ${JSON.stringify(sessionBoundUtilityErrors)}`);
    await page.close();
  }

  const sysmonSnapshot = (sessionId, cpuPercent, networkInterfaces = []) => ({
    sessionId,
    ts: new Date().toISOString(),
    uptimeSeconds: 60,
    cpuPercent,
    memoryPercent: 25,
    rxKbps: 1,
    txKbps: 2,
    loadAverage: [0.1, 0.2, 0.3],
    memoryTotalBytes: 1024,
    memoryAvailableBytes: 768,
    processes: [{ pid: 42, name: `${sessionId}-worker`, cpuPercent: 4.2, memoryPercent: 2.5, rssBytes: 52_428_800 }],
    disks: [],
    networkInterfaces,
  });
  const benchSysmon = sysmonSnapshot("bench-uart", 22.2, [{
    name: "eth0",
    addresses: ["fe80::25/64", "127.0.0.1/8", "192.168.33.121/24", "2001:db8::42/64"],
    rxBytes: 1024,
    txBytes: 2048,
    rxKbps: 1,
    txKbps: 2,
  }]);
  await page.evaluate(() => {
    window.__deferSysmon = true;
    window.__pendingSysmon = [];
  });
  await togglePanel("Sysmon 侧栏");
  const sysmonSidebar = page.locator('.workspace-dock-content[data-panel="sysmon"]');
  await sysmonSidebar.waitFor();
  await page.waitForFunction(() => window.__pendingSysmon.some((request) => (
    request.command === "refresh_sysmon" && request.args.sessionId === "edge-router"
  )));
  const sysmonAppletToggle = page.getByRole("button", { name: "启动 Sysmon 监控", exact: true });
  await sysmonAppletToggle.click();
  await page.waitForFunction(() => window.__pendingSysmon.filter((request) => (
    request.command === "refresh_sysmon" && request.args.sessionId === "edge-router"
  )).length >= 2);
  const sysmonAppletStop = page.getByRole("button", { name: "停止 Sysmon 监控", exact: true });
  await sysmonAppletStop.click();
  await page.waitForFunction(() => !document.querySelector(".sysmon-applet-toggle svg")?.classList.contains("loading"));
  assert(await page.getByRole("button", { name: "启动 Sysmon 监控", exact: true }).count() === 1,
    "stopping a pending Sysmon sample left the status applet busy");
  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" }).click();
  await page.waitForFunction(() => window.__pendingSysmon.some((request) => (
    request.command === "refresh_sysmon" && request.args.sessionId === "bench-uart"
  )));
  await page.evaluate(({ bench, staleEdge }) => {
    for (const pending of window.__pendingSysmon.filter((request) => request.args.sessionId === "bench-uart")) {
      pending.resolve(pending.command === "list_sysmon_history" ? [bench] : bench);
    }
    for (const pending of window.__pendingSysmon.filter((request) => request.args.sessionId === "edge-router")) {
      pending.resolve(pending.command === "list_sysmon_history" ? [staleEdge] : staleEdge);
    }
    window.__pendingSysmon = [];
  }, { bench: benchSysmon, staleEdge: sysmonSnapshot("edge-router", 99.9) });
  await page.waitForFunction(() => document.querySelector('.workspace-dock-content[data-panel="sysmon"]')?.textContent?.includes("22.2%"));
  const sysmonSidebarText = await sysmonSidebar.textContent();
  assert(sysmonSidebarText.includes("Bench UART")
    && sysmonSidebarText.includes("22.2%")
    && sysmonSidebarText.includes("25.0%")
    && sysmonSidebarText.includes("bench-uart-worker")
    && !sysmonSidebarText.includes("99.9%"),
  `Sysmon sidebar accepted stale data or omitted metrics: ${sysmonSidebarText}`);
  await page.screenshot({ path: `${screenshotPrefix}-sysmon-sidebar.png`, fullPage: true });

  await sysmonSidebar.getByRole("button", { name: "打开 Sysmon 详情", exact: true }).click();
  const sysmonDialog = page.locator(".sysmon-dialog");
  await sysmonDialog.waitFor();
  await page.waitForFunction(() => window.__pendingSysmon.some((request) => request.args.sessionId === "bench-uart"));
  await page.evaluate(({ bench }) => {
    for (const pending of window.__pendingSysmon.filter((request) => request.args.sessionId === "bench-uart")) {
      pending.resolve(pending.command === "list_sysmon_history" ? [bench] : bench);
    }
    window.__pendingSysmon = [];
  }, { bench: benchSysmon });
  await page.waitForFunction(() => document.querySelector(".sysmon-dialog")?.textContent?.includes("22.2%"));
  await sysmonDialog.getByRole("button", { name: /^网络/ }).click();
  const sysmonNetworkAddress = await sysmonDialog.locator(".sysmon-network-table tbody tr td").nth(1).textContent();
  const sysmonNetworkTitle = await sysmonDialog.locator(".sysmon-network-table tbody tr td").nth(1).getAttribute("title");
  assert(sysmonNetworkAddress === "192.168.33.121/24 · 2001:db8::42/64 +2",
    `Sysmon table did not prioritize usable addresses: ${JSON.stringify(sysmonNetworkAddress)}`);
  assert(sysmonNetworkTitle === "192.168.33.121/24 / 2001:db8::42/64 / fe80::25/64 / 127.0.0.1/8",
  `Sysmon address tooltip order is wrong: ${JSON.stringify(sysmonNetworkTitle)}`);
  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Edge Router" })
    .evaluate((button) => button.click());
  await page.waitForTimeout(50);
  assert(!await page.evaluate(() => window.__pendingSysmon.some(
    (request) => request.args.sessionId === "edge-router",
  )), "a scripted click behind Sysmon scheduled a hidden refresh");
  assert((await sysmonDialog.textContent()).includes("Bench UART"),
    "a scripted click crossed the Sysmon modal boundary");
  await sysmonDialog.getByRole("button", { name: "关闭 Sysmon", exact: true }).click();
  await sysmonDialog.waitFor({ state: "detached" });
  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Edge Router" }).click();
  await sysmonSidebar.getByRole("button", { name: "打开 Sysmon 详情", exact: true }).click();
  await sysmonDialog.waitFor();
  await page.waitForFunction(() => window.__pendingSysmon.some((request) => request.args.sessionId === "edge-router"));
  await page.evaluate(({ edge }) => {
    for (const pending of window.__pendingSysmon.filter((request) => request.args.sessionId === "edge-router")) {
      pending.resolve(pending.command === "list_sysmon_history" ? [edge] : edge);
    }
    window.__pendingSysmon = [];
  }, { edge: sysmonSnapshot("edge-router", 33.3) });
  await page.waitForFunction(() => document.querySelector(".sysmon-dialog")?.textContent?.includes("33.3%"));
  const sysmonText = await sysmonDialog.textContent();
  assert(sysmonText.includes("Edge Router") && sysmonText.includes("33.3%") && !sysmonText.includes("22.2%"),
    `Sysmon details did not use the newly active session: ${sysmonText}`);
  await sysmonDialog.getByRole("button", { name: "关闭 Sysmon", exact: true }).click();
  await sysmonDialog.waitFor({ state: "detached" });

  const sysmonRightResizer = page.getByRole("separator", { name: "调整右侧停靠区宽度", exact: true });
  await sysmonRightResizer.focus();
  await sysmonRightResizer.press("ArrowLeft");
  await sysmonRightResizer.press("ArrowLeft");
  await page.waitForFunction(() => JSON.parse(
    localStorage.getItem("portmate.workspacePanels.v2") || "null",
  )?.sizes?.right === 312);
  await togglePanel("Sysmon 侧栏");
  await sysmonSidebar.waitFor({ state: "detached" });
  await togglePanel("Sysmon 侧栏");
  await sysmonSidebar.waitFor();
  const restoredSysmonSidebar = await page.evaluate(() => {
    const snapshot = JSON.parse(localStorage.getItem("portmate.workspacePanels.v2") || "null");
    return {
      active: document.querySelector('.workspace-dock[data-dock="right"]')?.getAttribute("data-active-panel"),
      width: document.querySelector('.workspace-dock[data-dock="right"]')?.getBoundingClientRect().width ?? 0,
      right: snapshot.docks.right,
      visible: snapshot.panels.sysmon,
    };
  });
  assert(restoredSysmonSidebar.active === "sysmon"
    && restoredSysmonSidebar.width >= 311 && restoredSysmonSidebar.width <= 313
    && restoredSysmonSidebar.right[0] === "sysmon"
    && restoredSysmonSidebar.visible,
  `Sysmon dock position or width did not survive hide/reopen: ${JSON.stringify(restoredSysmonSidebar)}`);
  await page.getByRole("separator", { name: "调整右侧停靠区宽度", exact: true }).dblclick();
  await page.waitForFunction(() => JSON.parse(
    localStorage.getItem("portmate.workspacePanels.v2") || "null",
  )?.sizes?.right === null);
  await togglePanel("Sysmon 侧栏");
  await sysmonSidebar.waitFor({ state: "detached" });
  await page.evaluate(() => {
    window.__pendingSysmon = [];
    window.__deferSysmon = false;
  });

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
  assert(await page.locator('.transfer-mode-switch button[aria-pressed="true"]').textContent() === "自动 loadx",
    "Serial Modem transfer did not default to the device load receiver");
  await page.screenshot({ path: `${screenshotPrefix}-transfer-load.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const mobileLoadTransferBounds = await page.locator(".transfer-dialog").evaluate((dialog) => ({
    left: dialog.getBoundingClientRect().left,
    right: dialog.getBoundingClientRect().right,
    top: dialog.getBoundingClientRect().top,
    bottom: dialog.getBoundingClientRect().bottom,
    scrollWidth: dialog.scrollWidth,
    width: dialog.clientWidth,
  }));
  assert(mobileLoadTransferBounds.left >= 0 && mobileLoadTransferBounds.right <= 390
    && mobileLoadTransferBounds.top >= 0 && mobileLoadTransferBounds.bottom <= 844
    && mobileLoadTransferBounds.scrollWidth <= mobileLoadTransferBounds.width,
  `Mobile loadx transfer dialog overflowed: ${JSON.stringify(mobileLoadTransferBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-transfer-load-mobile.png`, fullPage: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.locator('.transfer-dialog .dialog-field', { hasText: "本地文件:" }).locator("input").fill("/tmp/firmware.bin");
  await page.locator('.transfer-dialog .dialog-field', { hasText: "加载地址:" }).locator("input").fill("0x80000000");
  await page.locator('.transfer-dialog .dialog-field', { hasText: "传输波特率:" }).locator("input").fill("115200");
  await page.locator(".transfer-dialog .utility-actions button", { hasText: "开始" }).click();
  await page.waitForFunction(() => window.__invokeCalls.some((call) => call.command === "start_transfer"));
  const serialLoadTransfer = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "start_transfer").at(-1)?.args.request);
  assert(serialLoadTransfer?.sessionId === "bench-uart"
    && serialLoadTransfer.protocol === "xmodem"
    && serialLoadTransfer.source === "/tmp/firmware.bin"
    && serialLoadTransfer.destination === "load:loadx?address=0x80000000&baud=115200",
  `Serial loadx transfer request is wrong: ${JSON.stringify(serialLoadTransfer)}`);
  assert(await page.locator(".transfer-dialog .transfer-list .transfer-row").count() === 1,
    "Serial loadx transfer was not added to the queue");
  const transferNotice = page.locator(".notice-dialog", { hasText: "xmodem queued" });
  await transferNotice.waitFor();
  await transferNotice.getByRole("button", { name: "确定", exact: true }).click();
  await page.locator(".transfer-dialog .utility-actions button", { hasText: "取消" }).click();
  await page.locator(".transfer-dialog").waitFor({ state: "detached" });
  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Edge Router" }).click();

  await page.evaluate(() => { window.__closeSessionError = true; });
  await page.getByRole("button", { name: "断开 Edge Router", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
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
  await page.evaluate(() => {
    window.__clipboardText = "clipboard-before-denial";
    window.__clipboardWriteFailures = 1;
  });
  await edge.click({ button: "right" });
  await page.locator(".context-menu-row", { hasText: "复制会话名称(N)" }).click();
  const sessionNameClipboardError = page.locator(".notice-dialog", { hasText: "复制会话名称失败" });
  await sessionNameClipboardError.waitFor();
  assert(await sessionNameClipboardError.locator(".notice-content").textContent() === "simulated clipboard denial"
    && await page.evaluate(() => window.__clipboardText) === "clipboard-before-denial",
  "resource session name copy did not report a clipboard write failure");
  await sessionNameClipboardError.getByRole("button", { name: "确定", exact: true }).click();
  await edge.click({ button: "right" });
  await page.locator(".context-menu-row", { hasText: "复制会话名称(N)" }).click();
  assert(await page.evaluate(() => window.__clipboardText) === "Edge Router",
    "resource context menu targeted a different session");
  await page.evaluate(() => { window.__clipboardWriteFailures = 1; });
  await edge.click({ button: "right" });
  await page.locator(".context-menu-row", { hasText: "复制会话 URL(U)" }).click();
  const sessionUrlClipboardError = page.locator(".notice-dialog", { hasText: "复制会话 URL失败" });
  await sessionUrlClipboardError.waitFor();
  assert(await sessionUrlClipboardError.locator(".notice-content").textContent() === "simulated clipboard denial",
    "resource session URL copy did not report a clipboard write failure");
  await sessionUrlClipboardError.getByRole("button", { name: "确定", exact: true }).click();

  const activeTerminalHost = page.locator(".terminal-pane.active .terminal-host");
  await page.waitForFunction((expectedTimestamp) => {
    const gutter = document.querySelector(".terminal-pane.active .terminal-timestamp-gutter");
    return gutter?.getAttribute("data-buffer-type") === "normal"
      && [...gutter.querySelectorAll("time")].some((time) => time.getAttribute("datetime") === expectedTimestamp);
  }, isoNow);
  const terminalTimestampLayout = await page.locator(".terminal-pane.active .terminal-terminal-region").evaluate((region) => {
    const gutter = region.querySelector(".terminal-timestamp-gutter");
    const host = region.querySelector(".terminal-host");
    const screen = host?.querySelector(".xterm-screen");
    const firstTimestamp = gutter?.querySelector("time");
    const regionRect = region.getBoundingClientRect();
    const gutterRect = gutter?.getBoundingClientRect();
    const hostRect = host?.getBoundingClientRect();
    const screenRect = screen?.getBoundingClientRect();
    const timestampRect = firstTimestamp?.getBoundingClientRect();
    return {
      bufferType: gutter?.getAttribute("data-buffer-type"),
      count: gutter?.querySelectorAll("time").length ?? 0,
      rowCount: host?.getAttribute("data-terminal-timestamp-rows") ?? "0",
      clock: firstTimestamp?.textContent ?? "",
      regionLeft: regionRect.left,
      gutterLeft: gutterRect?.left ?? 0,
      gutterRight: gutterRect?.right ?? 0,
      gutterWidth: gutterRect?.width ?? 0,
      hostLeft: hostRect?.left ?? 0,
      screenTop: screenRect?.top ?? 0,
      timestampTop: timestampRect?.top ?? 0,
    };
  });
  assert(terminalTimestampLayout.bufferType === "normal"
    && terminalTimestampLayout.count > 1
    && terminalTimestampLayout.count === Number(terminalTimestampLayout.rowCount)
    && /^\d{2}:\d{2}:\d{2}\.\d{6}$/.test(terminalTimestampLayout.clock)
    && Math.abs(terminalTimestampLayout.gutterLeft - terminalTimestampLayout.regionLeft) <= 1
    && Math.abs(terminalTimestampLayout.gutterWidth - 96) <= 1
    && Math.abs(terminalTimestampLayout.gutterRight - terminalTimestampLayout.hostLeft) <= 1
    && Math.abs(terminalTimestampLayout.timestampTop - terminalTimestampLayout.screenTop) <= 1,
  `terminal timestamps were not aligned to the left of XTerm rows: ${JSON.stringify(terminalTimestampLayout)}`);
  const terminalTimestampProbe = new Date(recordedAt + 5_000).toISOString()
    .replace("Z", "000Z");
  await page.evaluate((timestamp) => {
    window.__emitTauriEvent("portmate-session-event", {
      id: "terminal-timestamp-probe",
      sessionId: "edge-router",
      paneId: "edge-router:main",
      ts: timestamp,
      direction: "inbound",
      stream: "stdout",
      bytesRef: null,
      text: "PORTMATE TIMESTAMP PROBE\r\n",
      annotations: {},
    });
  }, terminalTimestampProbe);
  await page.waitForFunction((expectedTimestamp) => [...document.querySelectorAll(
    ".terminal-pane.active .terminal-timestamp-gutter time",
  )].some((time) => time.getAttribute("datetime") === expectedTimestamp), terminalTimestampProbe);
  await page.locator(".terminal-pane.active .terminal-terminal-region")
    .screenshot({ path: `${screenshotPrefix}-terminal-timestamps.png` });
  await activeTerminalHost.click({ button: "right", position: { x: 40, y: 40 } });
  const terminalContextMenu = page.locator(".terminal-context-menu");
  await terminalContextMenu.waitFor();
  assert(await terminalContextMenu.locator(".context-label", { hasText: /^导出终端文本$/ }).count() === 1
    && await terminalContextMenu.locator(".context-label", { hasText: /^导出终端文本到\.\.\.$/ }).count() === 1,
  "terminal context menu does not expose both default and chosen-path text exports");
  const terminalExportCallsBefore = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "export_terminal_text"
  )).length);
  await terminalContextMenu.locator(".context-menu-row", { hasText: "导出终端文本到..." }).click();
  await page.waitForFunction((before) => window.__invokeCalls.filter((call) => (
    call.command === "export_terminal_text"
  )).length === before + 1, terminalExportCallsBefore);
  const chosenTerminalExport = await page.evaluate(() => ({
    save: window.__invokeCalls.filter((call) => call.command === "plugin:dialog|save").at(-1),
    request: window.__invokeCalls.filter((call) => call.command === "export_terminal_text").at(-1)?.args.request,
  }));
  const exportedTerminalLines = chosenTerminalExport.request?.text?.split("\n") ?? [];
  assert(chosenTerminalExport.save?.args.options?.defaultPath?.endsWith("-buffer.txt")
    && chosenTerminalExport.request?.destinationDirectory === null
    && chosenTerminalExport.request?.destinationPath === "/tmp/portmate-picked-terminal.txt"
    && chosenTerminalExport.request?.overwrite === true
    && exportedTerminalLines.length > 1
    && exportedTerminalLines.every((line) => /^\[\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6}Z\] /.test(line))
    && exportedTerminalLines.some((line) => line === `[${terminalTimestampProbe}] PORTMATE TIMESTAMP PROBE`),
  `chosen-path terminal export was not routed through the native save dialog: ${JSON.stringify(chosenTerminalExport)}`);
  await page.getByRole("button", { name: "确定", exact: true }).click();

  const terminalExportOperationBaseline = await page.evaluate(() => {
    window.__deferTerminalTextExports = true;
    window.__pendingTerminalTextExports = [];
    return window.__invokeCalls.filter((call) => call.command === "export_terminal_text").length;
  });
  await activeTerminalHost.click({ button: "right", position: { x: 40, y: 40 } });
  await terminalContextMenu.waitFor();
  await terminalContextMenu.locator(".context-menu-row", { hasText: /^导出终端文本$/ }).evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingTerminalTextExports.length === 1);
  await activeTerminalHost.click({ button: "right", position: { x: 40, y: 40 } });
  await terminalContextMenu.waitFor();
  for (const [label, pattern] of [
    ["导出终端文本", /^导出终端文本$/],
    ["导出终端文本到...", /^导出终端文本到\.\.\.$/],
    ["导出选中文本", /^导出选中文本$/],
  ]) {
    assert(await terminalContextMenu.locator(".context-menu-row", { hasText: pattern }).isDisabled(),
      `pending terminal export left ${label} actionable`);
  }
  await page.evaluate(() => {
    window.__deferTerminalTextExports = false;
    window.__pendingTerminalTextExports.shift().resolve();
  });
  const deferredTerminalExportNotice = page.locator(".notice-dialog", { hasText: "/tmp/portmate-terminal-export.txt" });
  await deferredTerminalExportNotice.waitFor();
  const terminalExportOperationCalls = await page.evaluate((baseline) => (
    window.__invokeCalls.filter((call) => call.command === "export_terminal_text").length - baseline
  ), terminalExportOperationBaseline);
  assert(terminalExportOperationCalls === 1,
    `terminal context action submitted ${terminalExportOperationCalls} duplicate exports`);
  await deferredTerminalExportNotice.getByRole("button", { name: "确定", exact: true }).click();

  await page.evaluate(() => { window.__failNextTerminalTextExport = true; });
  await activeTerminalHost.click({ button: "right", position: { x: 40, y: 40 } });
  await terminalContextMenu.locator(".context-menu-row", { hasText: /^导出终端文本$/ }).click();
  const failedTerminalExportNotice = page.locator(".notice-dialog", { hasText: "simulated terminal text export failure" });
  await failedTerminalExportNotice.waitFor();
  await failedTerminalExportNotice.getByRole("button", { name: "确定", exact: true }).click();

  await activeTerminalHost.click({ button: "right", position: { x: 40, y: 40 } });
  await terminalContextMenu.waitFor();
  await activeTerminalHost.dispatchEvent("scroll");
  assert(await terminalContextMenu.isVisible(),
    "XTerm's internal scroll bookkeeping dismissed the terminal context menu");
  await activeTerminalHost.dispatchEvent("wheel", { deltaY: 120 });
  await terminalContextMenu.waitFor({ state: "detached" });
  const terminalInputProbe = "portmate-input-probe\r";
  const activeTerminalInput = activeTerminalHost.locator(".xterm-helper-textarea");
  assert(await activeTerminalHost.getAttribute("data-terminal-key-mode") === "remote"
    && await activeTerminalHost.getAttribute("data-terminal-cursor-style") === "bar",
  "SSH terminal did not start in Insert mode with a bar cursor");
  await activeTerminalInput.focus();
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => {
    const host = document.querySelector(".terminal-pane.active .terminal-host");
    return host?.getAttribute("data-terminal-key-mode") === "command"
      && host?.getAttribute("data-terminal-cursor-style") === "block";
  });
  await page.keyboard.press("i");
  await page.waitForFunction(() => {
    const host = document.querySelector(".terminal-pane.active .terminal-host");
    return host?.getAttribute("data-terminal-key-mode") === "remote"
      && host?.getAttribute("data-terminal-cursor-style") === "bar";
  });
  assert(await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "send_text").length) === 0,
  "Insert/Normal mode controls leaked into SSH terminal input");
  await page.keyboard.type("portmate-input-probe");
  await page.keyboard.press("Enter");
  await page.waitForFunction((expected) => window.__invokeCalls
    .filter((call) => call.command === "send_text")
    .map((call) => call.args.text)
    .join("") === expected, terminalInputProbe);
  const terminalInputWrites = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "send_text"));
  assert(terminalInputWrites.length > 0
    && terminalInputWrites.every((call) => call.args.sessionId === "edge-router")
    && terminalInputWrites.map((call) => call.args.text).join("") === terminalInputProbe,
  `terminal keyboard input was lost or routed to another session: ${JSON.stringify(terminalInputWrites)}`);

  const terminalCanvas = page.locator(".terminal-pane.active .terminal-canvas");
  assert(await terminalCanvas.getAttribute("data-terminal-display-mode") === "text",
    "terminal byte view did not default to text mode");
  const backgroundByteCapture = await page.evaluate(async () => {
    window.__emitTauriEvent("portmate-terminal-bytes", {
      id: "terminal-byte-background",
      sessionId: "bench-uart",
      ts: new Date().toISOString(),
      direction: "inbound",
      stream: "stdout",
      bytes: [0xde, 0xad, 0xbe, 0xef],
      originalLength: 4,
      truncated: false,
      eventId: null,
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    const state = await import("/src/terminal-byte-state.ts");
    return state.terminalByteCacheSnapshot("bench-uart").capturedBytes;
  });
  assert(backgroundByteCapture === 4,
    "window-level byte capture missed data from an inactive terminal tab");
  await page.evaluate(() => {
    window.__emitTauriEvent("portmate-terminal-bytes", {
      id: "terminal-byte-rx-1",
      sessionId: "edge-router",
      ts: new Date().toISOString(),
      direction: "inbound",
      stream: "stdout",
      bytes: [0x00, 0x41, 0x0d, 0x0a, 0x09, 0x80, 0xff, 0x20, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c],
      originalLength: 20,
      truncated: false,
      eventId: "event-edge-router",
    });
    window.__emitTauriEvent("portmate-terminal-bytes", {
      id: "terminal-byte-tx-1",
      sessionId: "edge-router",
      ts: new Date().toISOString(),
      direction: "outbound",
      stream: "control",
      bytes: [0x1b, 0x5b, 0x41],
      originalLength: 3,
      truncated: false,
      eventId: "event-edge-router-tx",
    });
  });
  const hexModeButton = terminalCanvas.getByRole("button", { name: "Hex", exact: true });
  await hexModeButton.click();
  await terminalCanvas.locator('.terminal-byte-scroll[aria-rowcount="4"]').waitFor();
  assert(await terminalCanvas.getAttribute("data-terminal-display-mode") === "hex"
    && await terminalCanvas.locator(".terminal-byte-inspector").isVisible()
    && await terminalCanvas.locator(".terminal-host .xterm-screen").count() === 1,
  "Hex mode did not retain the mounted XTerm behind the byte inspector");
  const pairedByte = terminalCanvas.locator('[data-byte-key="terminal-byte-rx-1:1"]');
  assert(await pairedByte.count() === 2, "Hex and ASCII did not expose the same byte index");
  await terminalCanvas.locator('[data-byte-key="terminal-byte-rx-1:1"][data-byte-column="hex"]').click();
  assert(await pairedByte.evaluateAll((cells) => cells.every((cell) => (
    cell.classList.contains("selected") && cell.getAttribute("aria-pressed") === "true"
  ))), "selecting Hex did not highlight its corresponding ASCII byte");
  const hoveredAscii = terminalCanvas.locator('[data-byte-key="terminal-byte-rx-1:2"][data-byte-column="ascii"]');
  await hoveredAscii.hover();
  assert(await terminalCanvas.locator('[data-byte-key="terminal-byte-rx-1:2"]').evaluateAll((cells) => (
    cells.length === 2 && cells.every((cell) => cell.classList.contains("linked"))
  )), "hovering ASCII did not highlight its corresponding Hex byte");

  await terminalCanvas.getByRole("button", { name: "对照", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".terminal-pane.active .terminal-canvas")
    ?.getAttribute("data-terminal-display-mode") === "split");
  const splitLayout = await terminalCanvas.evaluate((canvas) => {
    const region = canvas.querySelector(".terminal-terminal-region")?.getBoundingClientRect();
    const inspector = canvas.querySelector(".terminal-byte-inspector")?.getBoundingClientRect();
    return region && inspector ? {
      region: { left: region.left, top: region.top, right: region.right, bottom: region.bottom, width: region.width },
      inspector: { left: inspector.left, top: inspector.top, right: inspector.right, bottom: inspector.bottom, width: inspector.width },
    } : null;
  });
  assert(splitLayout
    && splitLayout.region.width > 200
    && splitLayout.inspector.width > 300
    && splitLayout.region.right <= splitLayout.inspector.left + 1,
  `desktop terminal comparison view was not split horizontally: ${JSON.stringify(splitLayout)}`);
  const splitInputWritesBefore = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "send_text").length);
  const splitTerminalInput = terminalCanvas.locator(".terminal-host .xterm-helper-textarea");
  await splitTerminalInput.focus();
  await page.keyboard.type("split-input-probe");
  await page.keyboard.press("Enter");
  await page.waitForFunction(({ start, expected }) => window.__invokeCalls
    .filter((call) => call.command === "send_text")
    .slice(start)
    .map((call) => call.args.text)
    .join("") === expected, { start: splitInputWritesBefore, expected: "split-input-probe\r" });
  const splitInputWrites = await page.evaluate((start) => window.__invokeCalls
    .filter((call) => call.command === "send_text")
    .slice(start), splitInputWritesBefore);
  assert(splitInputWrites.length > 0
    && splitInputWrites.every((call) => call.args.sessionId === "edge-router")
    && splitInputWrites.map((call) => call.args.text).join("") === "split-input-probe\r",
  `comparison mode duplicated or misrouted terminal input: ${JSON.stringify(splitInputWrites)}`);

  const byteLayoutBeforeUpdate = await terminalCanvas.evaluate((canvas) => {
    const region = canvas.querySelector(".terminal-terminal-region")?.getBoundingClientRect();
    const inspector = canvas.querySelector(".terminal-byte-inspector")?.getBoundingClientRect();
    return { regionWidth: region?.width ?? 0, inspectorWidth: inspector?.width ?? 0 };
  });
  await page.evaluate(() => {
    window.__emitTauriEvent("portmate-terminal-bytes", {
      id: "terminal-byte-rx-bulk",
      sessionId: "edge-router",
      ts: new Date().toISOString(),
      direction: "inbound",
      stream: "stdout",
      bytes: Array.from({ length: 4096 }, (_, index) => index % 256),
      originalLength: 4096,
      truncated: false,
      eventId: null,
    });
  });
  await terminalCanvas.locator('.terminal-byte-scroll[aria-rowcount="517"]').waitFor();
  const byteLayoutAfterUpdate = await terminalCanvas.evaluate((canvas) => {
    const region = canvas.querySelector(".terminal-terminal-region")?.getBoundingClientRect();
    const inspector = canvas.querySelector(".terminal-byte-inspector")?.getBoundingClientRect();
    return {
      regionWidth: region?.width ?? 0,
      inspectorWidth: inspector?.width ?? 0,
      renderedRows: canvas.querySelectorAll(".terminal-byte-row").length,
    };
  });
  assert(Math.abs(byteLayoutBeforeUpdate.regionWidth - byteLayoutAfterUpdate.regionWidth) < 1
    && Math.abs(byteLayoutBeforeUpdate.inspectorWidth - byteLayoutAfterUpdate.inspectorWidth) < 1
    && byteLayoutAfterUpdate.renderedRows < 80,
  `terminal byte updates shifted layout or disabled row virtualization: ${JSON.stringify({ byteLayoutBeforeUpdate, byteLayoutAfterUpdate })}`);
  const byteScroll = terminalCanvas.locator(".terminal-byte-scroll");
  await byteScroll.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll"));
  });
  const followBytesButton = terminalCanvas.getByRole("button", { name: "跟随最新字节", exact: true });
  await page.waitForFunction(() => document.querySelector('[aria-label="跟随最新字节"]')?.getAttribute("aria-pressed") === "false");
  await followBytesButton.click();
  await page.waitForFunction(() => {
    const element = document.querySelector(".terminal-pane.active .terminal-byte-scroll");
    return element && element.scrollHeight - element.scrollTop - element.clientHeight <= 2;
  });
  await terminalCanvas.screenshot({ path: `${screenshotPrefix}-terminal-byte-desktop.png` });

  await page.setViewportSize({ width: 600, height: 800 });
  await page.waitForFunction(() => {
    const canvas = document.querySelector(".terminal-pane.active .terminal-canvas");
    const region = canvas?.querySelector(".terminal-terminal-region")?.getBoundingClientRect();
    const inspector = canvas?.querySelector(".terminal-byte-inspector")?.getBoundingClientRect();
    return region && inspector && region.bottom <= inspector.top + 1;
  });
  const narrowTerminalLayout = await terminalCanvas.evaluate((canvas) => {
    const region = canvas.querySelector(".terminal-terminal-region")?.getBoundingClientRect();
    const inspector = canvas.querySelector(".terminal-byte-inspector")?.getBoundingClientRect();
    const gutter = canvas.querySelector(".terminal-timestamp-gutter")?.getBoundingClientRect();
    const host = canvas.querySelector(".terminal-host")?.getBoundingClientRect();
    return {
      regionWidth: region?.width ?? 0,
      inspectorWidth: inspector?.width ?? 0,
      regionBottom: region?.bottom ?? 0,
      inspectorTop: inspector?.top ?? 0,
      gutterWidth: gutter?.width ?? 0,
      gutterRight: gutter?.right ?? 0,
      hostLeft: host?.left ?? 0,
      hostRight: host?.right ?? 0,
      regionRight: region?.right ?? 0,
      documentWidth: document.documentElement.scrollWidth,
      viewportWidth: window.innerWidth,
    };
  });
  assert(Math.abs(narrowTerminalLayout.regionWidth - narrowTerminalLayout.inspectorWidth) < 1
    && narrowTerminalLayout.regionBottom <= narrowTerminalLayout.inspectorTop + 1
    && Math.abs(narrowTerminalLayout.gutterWidth - 96) <= 1
    && Math.abs(narrowTerminalLayout.gutterRight - narrowTerminalLayout.hostLeft) <= 1
    && narrowTerminalLayout.hostRight <= narrowTerminalLayout.regionRight + 1
    && narrowTerminalLayout.documentWidth === narrowTerminalLayout.viewportWidth,
  `narrow terminal comparison layout overflowed or overlapped: ${JSON.stringify(narrowTerminalLayout)}`);
  await terminalCanvas.screenshot({ path: `${screenshotPrefix}-terminal-byte-narrow.png` });
  await page.setViewportSize({ width: 1440, height: 900 });
  const clearBytesButton = terminalCanvas.getByRole("button", { name: "清空实时字节", exact: true });
  await clearBytesButton.click();
  await terminalCanvas.locator(".terminal-byte-empty").waitFor();
  const clearedByteState = await page.evaluate(async () => {
    const state = await import("/src/terminal-byte-state.ts");
    const snapshot = state.terminalByteCacheSnapshot("edge-router");
    return {
      capturedBytes: snapshot.capturedBytes,
      frames: snapshot.frames.length,
      nextOffset: snapshot.nextOffset,
    };
  });
  assert(clearedByteState.capturedBytes === 0
    && clearedByteState.frames === 0
    && clearedByteState.nextOffset === 0
    && await clearBytesButton.isDisabled()
    && await terminalCanvas.locator(".terminal-byte-summary").innerText() === "RX 0 B\nTX 0 B",
  `clearing live terminal bytes left stale data or controls: ${JSON.stringify(clearedByteState)}`);
  await terminalCanvas.getByRole("button", { name: "文本", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".terminal-pane.active .terminal-canvas")
    ?.getAttribute("data-terminal-display-mode") === "text");
  assert(JSON.parse(await page.evaluate(() => localStorage.getItem("portmate.terminalDisplayModes.v1")))?.["view-edge"] === "text",
    "terminal display mode was not persisted per workspace view");

  await togglePanel("文件管理器");
  const leftDock = page.locator('.workspace-dock[data-dock="left"]');
  await leftDock.locator('.workspace-dock-content[data-panel="fileManager"]').waitFor();
  await leftDock.locator('.file-browser-pane[data-file-pane="local"]').waitFor();
  await page.waitForFunction(() => window.__invokeCalls.some((call) => (
    call.command === "list_files"
      && call.args.request.remote === false
      && call.args.request.path === "~"
  )));
  const initialLocalPath = page.getByRole("textbox", { name: "本地路径", exact: true });
  assert(await initialLocalPath.inputValue() === "~",
    "file manager did not begin from a cross-platform home path");
  const initialLocalPane = leftDock.locator('.file-browser-pane[data-file-pane="local"]');
  const homeCreateCallsBefore = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "create_file").length);
  await page.evaluate(() => {
    window.__originalPrompt = window.prompt;
    window.prompt = () => "home-note.txt";
  });
  await initialLocalPane.getByRole("button", { name: "新建文件", exact: true }).click();
  await page.waitForFunction((count) => window.__invokeCalls.filter((call) => call.command === "create_file").length === count + 1, homeCreateCallsBefore);
  const homeCreateRequest = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "create_file").at(-1)?.args.request);
  await page.evaluate(() => {
    window.prompt = window.__originalPrompt;
    delete window.__originalPrompt;
  });
  assert(homeCreateRequest?.path === "~/home-note.txt" && homeCreateRequest?.remote === false,
    `home file creation did not preserve the portable path: ${JSON.stringify(homeCreateRequest)}`);
  const homeListCallsBefore = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "list_files" && call.args.request.remote === false && call.args.request.path === "~"
  )).length);
  await initialLocalPane.locator(".file-row.up").click();
  await page.waitForFunction((count) => window.__invokeCalls.filter((call) => (
    call.command === "list_files" && call.args.request.remote === false && call.args.request.path === "~"
  )).length === count + 1, homeListCallsBefore);
  const fileDockLayout = await leftDock.evaluate((dock) => ({
    width: dock.getBoundingClientRect().width,
    active: dock.getAttribute("data-active-panel"),
    tabs: [...dock.querySelectorAll(".workspace-dock-tab")].map((tab) => tab.getAttribute("data-panel")),
    panels: [...dock.querySelectorAll(".workspace-dock-content")].map((panel) => panel.getAttribute("data-panel")),
    visiblePanels: [...dock.querySelectorAll(".workspace-dock-content:not([hidden])")].map((panel) => panel.getAttribute("data-panel")),
    panes: [...dock.querySelectorAll(".file-browser-pane")].map((pane) => {
      const rect = pane.getBoundingClientRect();
      return { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right };
    }),
    actions: [...dock.querySelectorAll(".file-actions")].map((actions) => ({
      clientWidth: actions.clientWidth,
      scrollWidth: actions.scrollWidth,
      persistentLabels: [...actions.querySelectorAll(":scope > button")].map((button) => button.getAttribute("aria-label")),
      persistentIcons: actions.querySelectorAll(":scope > button svg").length,
      overflowLabel: actions.querySelector(".file-action-overflow > summary")?.getAttribute("aria-label") ?? "",
      overflowActions: [...actions.querySelectorAll(".file-action-overflow-menu button")].map((button) => button.getAttribute("aria-label")),
    })),
  }));
  assert(fileDockLayout.width >= 350 && fileDockLayout.width <= 366
    && fileDockLayout.active === "fileManager"
    && JSON.stringify(fileDockLayout.tabs) === JSON.stringify(["explorer", "fileManager"])
    && JSON.stringify(fileDockLayout.panels) === JSON.stringify(["explorer", "fileManager"])
    && JSON.stringify(fileDockLayout.visiblePanels) === JSON.stringify(["fileManager"])
    && fileDockLayout.panes.length === 2
    && fileDockLayout.panes[1].top >= fileDockLayout.panes[0].bottom - 1
    && fileDockLayout.actions.length === 2
    && fileDockLayout.actions.every((actions) => (
      actions.scrollWidth <= actions.clientWidth + 1
      && actions.persistentLabels.length === 5
      && actions.persistentLabels.every(Boolean)
      && actions.persistentIcons === 5
      && actions.overflowLabel === "更多文件操作"
      && JSON.stringify(actions.overflowActions) === JSON.stringify(["复制路径", "移动到...", "重命名", "修改权限", "文件属性"])
    )),
  `file manager did not occupy the active left dock view: ${JSON.stringify(fileDockLayout)}`);

  const fileManagerTab = leftDock.getByRole("tab", { name: "文件管理器", exact: true });
  const explorerTab = leftDock.getByRole("tab", { name: "资源管理器", exact: true });
  await fileManagerTab.press("ArrowLeft");
  await page.waitForFunction(() => document.querySelector('.workspace-dock[data-dock="left"]')?.getAttribute("data-active-panel") === "explorer");
  assert(await explorerTab.getAttribute("aria-selected") === "true"
    && await fileManagerTab.getAttribute("aria-selected") === "false",
  "left-arrow did not switch the active dock tab");
  await explorerTab.press("ArrowRight");
  await page.waitForFunction(() => document.querySelector('.workspace-dock[data-dock="left"]')?.getAttribute("data-active-panel") === "fileManager");

  const localFilePath = page.getByRole("textbox", { name: "本地路径", exact: true });
  await page.evaluate(() => { window.__deferFileLoads = true; });
  await localFilePath.fill("/portmate-slow");
  await localFilePath.press("Enter");
  await localFilePath.fill("/portmate-fast");
  await localFilePath.press("Enter");
  await page.waitForFunction(() => window.__pendingFileLoads.length === 2);
  await page.evaluate(() => {
    const pending = window.__pendingFileLoads.find((request) => request.args.request.path === "/portmate-fast");
    pending.resolve([
      { name: "FAST-RESULT", path: "/portmate-fast/FAST-RESULT", isDir: false, size: 4, modified: null },
      { name: "SECOND-RESULT", path: "/portmate-fast/SECOND-RESULT", isDir: false, size: 6, modified: null },
    ]);
  });
  await page.locator('.file-browser-pane[data-file-pane="local"] .file-row', { hasText: "FAST-RESULT" }).waitFor();
  await page.evaluate(() => {
    const pending = window.__pendingFileLoads.find((request) => request.args.request.path === "/portmate-slow");
    pending.resolve([{ name: "STALE-RESULT", path: "/portmate-slow/STALE-RESULT", isDir: false, size: 5, modified: null }]);
    window.__deferFileLoads = false;
  });
  await page.waitForTimeout(100);
  assert(await localFilePath.inputValue() === "/portmate-fast"
    && await page.locator('.file-browser-pane[data-file-pane="local"] .file-row', { hasText: "FAST-RESULT" }).count() === 1
    && await page.locator('.file-browser-pane[data-file-pane="local"] .file-row', { hasText: "STALE-RESULT" }).count() === 0,
  "a stale file listing replaced the latest directory response");

  const localFilePane = page.locator('.file-browser-pane[data-file-pane="local"]');
  const fileActionOverflow = localFilePane.locator(".file-action-overflow");
  async function openFileActionOverflow() {
    if (await fileActionOverflow.getAttribute("open") === null) {
      await fileActionOverflow.locator("summary").click();
    }
  }
  const fileBack = localFilePane.getByRole("button", { name: "本地后退", exact: true });
  const fileForward = localFilePane.getByRole("button", { name: "本地前进", exact: true });
  assert(!await fileBack.isDisabled() && await fileForward.isDisabled(),
    "successful file navigation did not expose the expected history controls");
  await page.evaluate(() => { window.__deferFileLoads = true; });
  await fileBack.click();
  await page.waitForFunction(() => window.__pendingFileLoads.some((request) => request.args.request.path === "~"));
  await page.evaluate(() => {
    const pending = window.__pendingFileLoads.filter((request) => request.args.request.path === "~").at(-1);
    pending.resolve([{ name: "HOME-RESULT", path: "~/HOME-RESULT", isDir: false, size: 4, modified: null }]);
    window.__deferFileLoads = false;
  });
  await page.locator('.file-browser-pane[data-file-pane="local"] .file-row', { hasText: "HOME-RESULT" }).waitFor();
  assert(await localFilePath.inputValue() === "~"
    && await fileBack.isDisabled()
    && !await fileForward.isDisabled(),
  "file history back navigation did not restore the previous loaded path");
  await page.evaluate(() => { window.__deferFileLoads = true; });
  await fileForward.click();
  await page.waitForFunction(() => window.__pendingFileLoads.filter((request) => request.args.request.path === "/portmate-fast").length >= 2);
  await page.evaluate(() => {
    const pending = window.__pendingFileLoads.filter((request) => request.args.request.path === "/portmate-fast").at(-1);
    pending.resolve([
      { name: "FAST-RESULT", path: "/portmate-fast/FAST-RESULT", isDir: false, size: 4, modified: null },
      { name: "SECOND-RESULT", path: "/portmate-fast/SECOND-RESULT", isDir: false, size: 6, modified: null },
    ]);
    window.__deferFileLoads = false;
  });
  await page.locator('.file-browser-pane[data-file-pane="local"] .file-row', { hasText: "FAST-RESULT" }).waitFor();
  assert(await localFilePath.inputValue() === "/portmate-fast"
    && !await fileBack.isDisabled()
    && await fileForward.isDisabled(),
  "file history forward navigation did not restore the latest loaded path");

  const filePropertiesButton = localFilePane.getByRole("button", { name: "文件属性", exact: true });
  await page.evaluate(() => { window.__deferFileProperties = true; });
  await localFilePane.locator(".file-row", { hasText: "FAST-RESULT" }).click();
  await openFileActionOverflow();
  await page.screenshot({ path: `${screenshotPrefix}-file-manager-menu.png`, fullPage: true });
  await filePropertiesButton.click();
  await page.locator(".file-properties-dialog").waitFor();
  await page.locator(".file-properties-dialog .utility-actions").getByRole("button", { name: "关闭", exact: true }).click();
  await localFilePane.locator(".file-row", { hasText: "SECOND-RESULT" }).click();
  await openFileActionOverflow();
  await filePropertiesButton.click();
  await page.waitForFunction(() => window.__pendingFileProperties.length === 2);
  await page.evaluate(() => {
    const pending = window.__pendingFileProperties.find((request) => request.args.request.path.endsWith("/SECOND-RESULT"));
    pending.resolve({
      name: "SECOND-RESULT",
      path: "/portmate-fast/SECOND-RESULT",
      remote: false,
      kind: "file",
      isDir: false,
      isFile: true,
      isSymlink: false,
      size: 6,
    });
  });
  await page.locator(".file-properties-dialog", { hasText: "SECOND-RESULT" }).waitFor();
  await page.evaluate(() => {
    const pending = window.__pendingFileProperties.find((request) => request.args.request.path.endsWith("/FAST-RESULT"));
    pending.resolve({
      name: "FAST-RESULT",
      path: "/portmate-fast/FAST-RESULT",
      remote: false,
      kind: "file",
      isDir: false,
      isFile: true,
      isSymlink: false,
      size: 4,
    });
    window.__deferFileProperties = false;
  });
  await page.waitForTimeout(100);
  const filePropertiesText = await page.locator(".file-properties-dialog").textContent();
  assert(filePropertiesText.includes("SECOND-RESULT") && !filePropertiesText.includes("FAST-RESULT"),
    `a stale properties response replaced the reopened inspector: ${filePropertiesText}`);
  await page.locator(".file-properties-dialog .utility-actions").getByRole("button", { name: "关闭", exact: true }).click();

  await localFilePane.locator(".file-row", { hasText: "FAST-RESULT" }).click();
  await openFileActionOverflow();
  await localFilePane.getByRole("button", { name: "复制路径", exact: true }).click();
  await page.waitForFunction(() => window.__clipboardText === "/portmate-fast/FAST-RESULT");

  const moveCallsBefore = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "move_paths").length);
  await localFilePane.locator(".file-row", { hasText: "FAST-RESULT" }).click();
  await page.evaluate(() => {
    window.__originalPrompt = window.prompt;
    window.prompt = () => "/portmate-target ";
  });
  await openFileActionOverflow();
  await localFilePane.getByRole("button", { name: "移动到...", exact: true }).click();
  await page.waitForFunction((count) => window.__invokeCalls.filter((call) => call.command === "move_paths").length === count + 1, moveCallsBefore);
  const moveRequest = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "move_paths").at(-1)?.args.request);
  await page.evaluate(() => {
    window.prompt = window.__originalPrompt;
    delete window.__originalPrompt;
  });
  assert(moveRequest?.destination === "/portmate-target "
    && JSON.stringify(moveRequest?.paths) === JSON.stringify(["/portmate-fast/FAST-RESULT"])
    && moveRequest?.remote === false
    && moveRequest?.sessionId === "edge-router",
  `Move To did not target the selected local item: ${JSON.stringify(moveRequest)}`);

  const newFileCallsBefore = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "create_file").length);
  await page.evaluate(() => {
    window.__originalPrompt = window.prompt;
    window.prompt = () => " workspace-note.txt ";
  });
  await localFilePane.getByRole("button", { name: "新建文件", exact: true }).click();
  await page.waitForFunction((count) => window.__invokeCalls.filter((call) => call.command === "create_file").length === count + 1, newFileCallsBefore);
  const newFileRequest = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "create_file").at(-1)?.args.request);
  await page.evaluate(() => {
    window.prompt = window.__originalPrompt;
    delete window.__originalPrompt;
  });
  assert(newFileRequest?.path === "/portmate-fast/ workspace-note.txt "
    && newFileRequest?.remote === false
    && newFileRequest?.sessionId === "edge-router",
  `new file did not target the active local directory: ${JSON.stringify(newFileRequest)}`);

  await page.evaluate(() => {
    window.__deferFileLoads = true;
    window.__pendingFileLoads = [];
  });
  await localFilePath.fill("/portmate-delete");
  await localFilePath.press("Enter");
  await page.waitForFunction(() => window.__pendingFileLoads.some((request) => request.args.request.path === "/portmate-delete"));
  await page.evaluate(() => {
    const pending = window.__pendingFileLoads.find((request) => request.args.request.path === "/portmate-delete");
    pending.resolve([
      { name: "DELETE-ONE", path: "/portmate-delete/DELETE-ONE", isDir: false, size: 4, modified: null },
      { name: "DELETE-TWO", path: "/portmate-delete/DELETE-TWO", isDir: false, size: 6, modified: null },
    ]);
    window.__deferFileLoads = false;
  });
  await localFilePane.locator(".file-row", { hasText: "DELETE-ONE" }).waitFor();
  await localFilePane.locator(".file-row", { hasText: "DELETE-ONE" }).click();
  await localFilePane.locator(".file-row", { hasText: "DELETE-TWO" }).click({ modifiers: ["Control"] });
  const deleteCallsBefore = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "delete_paths").length);
  await page.evaluate(() => {
    window.__originalConfirm = window.confirm;
    window.__fileDeleteConfirmCalls = 0;
    window.__deferFileMutations = true;
    window.confirm = () => {
      window.__fileDeleteConfirmCalls += 1;
      return true;
    };
  });
  const deleteButton = localFilePane.getByRole("button", { name: "删除", exact: true });
  await deleteButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction((count) => window.__invokeCalls.filter((call) => call.command === "delete_paths").length === count + 1, deleteCallsBefore);
  await page.waitForFunction(() => window.__pendingFileMutations.length === 1);
  const pendingDeleteState = await page.evaluate(() => ({
    confirms: window.__fileDeleteConfirmCalls,
    pending: window.__pendingFileMutations.map((request) => request.command),
    request: window.__pendingFileMutations[0]?.args.request,
  }));
  assert(pendingDeleteState.confirms === 1
    && JSON.stringify(pendingDeleteState.pending) === JSON.stringify(["delete_paths"])
    && await deleteButton.isDisabled()
    && await localFilePath.isDisabled()
    && await localFilePane.getByRole("button", { name: "新建文件", exact: true }).isDisabled()
    && await localFilePane.getByRole("combobox", { name: "文件冲突策略", exact: true }).isDisabled()
    && await localFilePane.locator('.file-row[role="option"]').first().getAttribute("aria-disabled") === "true",
  `a pending file delete did not lock duplicate or stale pane operations: ${JSON.stringify(pendingDeleteState)}`);
  await page.evaluate(() => {
    window.__pendingFileMutations[0].resolve();
    window.__deferFileMutations = false;
  });
  await localFilePane.getByRole("button", { name: "新建文件", exact: true }).waitFor({ state: "visible" });
  await page.waitForFunction(() => {
    const pane = document.querySelector('.file-browser-pane[data-file-pane="local"]');
    return pane?.querySelector('[aria-label="新建文件"]')?.disabled === false
      && pane?.querySelector('[aria-label="本地路径"]')?.disabled === false;
  });
  const deleteRequest = pendingDeleteState.request;
  await page.evaluate(() => {
    window.confirm = window.__originalConfirm;
    delete window.__originalConfirm;
  });
  assert(JSON.stringify(deleteRequest?.paths) === JSON.stringify([
    "/portmate-delete/DELETE-ONE",
    "/portmate-delete/DELETE-TWO",
  ]) && deleteRequest?.remote === false
    && deleteRequest?.sessionId === "edge-router",
  `batch delete did not target the selected local items: ${JSON.stringify(deleteRequest)}`);

  await page.evaluate(() => {
    window.__deferFileLoads = true;
    window.__pendingFileLoads = [];
  });
  await localFilePath.fill("/portmate-transfer");
  await localFilePath.press("Enter");
  await page.waitForFunction(() => window.__pendingFileLoads.some((request) => request.args.request.path === "/portmate-transfer"));
  await page.evaluate(() => {
    const pending = window.__pendingFileLoads.find((request) => request.args.request.path === "/portmate-transfer");
    pending.resolve([{
      name: "SESSION-SWITCH.bin",
      path: "/portmate-transfer/SESSION-SWITCH.bin",
      isDir: false,
      size: 32,
      modified: null,
    }]);
    window.__deferFileLoads = false;
  });
  const transferSwitchRow = localFilePane.locator(".file-row", { hasText: "SESSION-SWITCH.bin" });
  await transferSwitchRow.waitFor();
  await transferSwitchRow.click();
  await page.evaluate(() => {
    window.__deferFileBatches = true;
    window.__pendingFileBatches = [];
  });
  const remoteFilePane = page.locator('.file-browser-pane[data-file-pane="remote"]');
  const uploadButton = localFilePane.getByRole("button", { name: "上传", exact: true });
  await uploadButton.click();
  await page.waitForFunction(() => window.__pendingFileBatches.length === 1);
  const pendingFileTransfer = await page.evaluate(() => window.__pendingFileBatches[0]?.args.request);
  assert(pendingFileTransfer?.sessionId === "edge-router"
    && JSON.stringify(pendingFileTransfer?.paths) === JSON.stringify(["/portmate-transfer/SESSION-SWITCH.bin"])
    && pendingFileTransfer?.sourceRemote === false
    && pendingFileTransfer?.destinationRemote === true
    && pendingFileTransfer?.destination === "."
    && await uploadButton.isDisabled()
    && await localFilePath.isDisabled()
    && await remoteFilePane.getByRole("textbox", { name: "远端路径", exact: true }).isDisabled(),
  `a batch transfer did not own both file panes: ${JSON.stringify(pendingFileTransfer)}`);

  await explorerTab.click();
  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" }).click();
  await page.waitForFunction(() => (
    document.querySelector(".workspace-dock-content.panel-explorer .tree-session.active")?.textContent?.includes("Bench UART")
  ));
  await page.waitForFunction(() => {
    const pane = document.querySelector('.file-browser-pane[data-file-pane="local"]');
    return pane?.querySelector('[aria-label="新建文件"]')?.disabled === false
      && pane?.querySelector('[aria-label="本地路径"]')?.disabled === false;
  });
  const releasedFilePaneState = await page.evaluate(() => {
    const pane = document.querySelector('.file-browser-pane[data-file-pane="local"]');
    return {
      pathDisabled: pane?.querySelector('[aria-label="本地路径"]')?.disabled ?? null,
      createDisabled: pane?.querySelector('[aria-label="新建文件"]')?.disabled ?? null,
      busy: pane?.getAttribute("aria-busy") ?? null,
    };
  });
  assert(releasedFilePaneState.pathDisabled === false
    && releasedFilePaneState.createDisabled === false
    && releasedFilePaneState.busy === "false",
  `switching away from an SSH session left the local half of a cross-pane operation locked: ${JSON.stringify(releasedFilePaneState)}`);
  await page.evaluate(() => {
    const pending = window.__pendingFileBatches[0];
    pending.resolve();
    window.__pendingFileBatches = [];
    window.__deferFileBatches = false;
  });
  await page.waitForTimeout(100);
  assert(await page.locator(".notice-dialog").count() === 0,
    "a transfer response from the previous SSH session produced a stale notice");
  await togglePanel("历史命令");
  const serialMonitor = page.locator(".serial-monitor");
  await serialMonitor.waitFor();
  await page.evaluate(() => {
    window.__serialCaptureFrames = [{
      id: "main-rx-before-clear",
      ts: new Date().toISOString(),
      direction: "inbound",
      bytes: [0x31, 0x32, 0x33],
      originalLength: 3,
      truncated: false,
    }];
  });
  await serialMonitor.locator(".serial-monitor-row", { hasText: "31 32 33" }).waitFor();
  await page.evaluate(() => {
    window.__serialCaptureFrames = [{
      id: "main-stale-read",
      ts: new Date().toISOString(),
      direction: "inbound",
      bytes: [0x53, 0x54, 0x41, 0x4c, 0x45],
      originalLength: 5,
      truncated: false,
    }];
    window.__deferSerialCaptureReads = true;
    window.__pendingSerialCaptureReads = [];
    window.__deferSerialCaptureOperations = true;
    window.__pendingSerialCaptureOperations = [];
  });
  await page.waitForFunction(() => window.__pendingSerialCaptureReads.length === 1);
  await serialMonitor.getByRole("button", { name: "清空串口捕获", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingSerialCaptureOperations.length === 1);
  const mainSerialClearState = await page.evaluate(() => ({
    clearCalls: window.__invokeCalls.filter((call) => call.command === "clear_serial_capture").length,
    pendingReads: window.__pendingSerialCaptureReads.length,
    pendingActions: window.__pendingSerialCaptureOperations.length,
    busy: document.querySelector(".serial-monitor")?.getAttribute("aria-busy"),
  }));
  assert(mainSerialClearState.clearCalls === 1
    && mainSerialClearState.pendingReads === 1
    && mainSerialClearState.pendingActions === 1
    && mainSerialClearState.busy === "true"
    && await serialMonitor.getByRole("button", { name: "清空串口捕获", exact: true }).isDisabled()
    && await serialMonitor.getByRole("button", { name: "导出可见串口帧", exact: true }).isDisabled(),
  `main serial clear duplicated or left conflicting controls enabled: ${JSON.stringify(mainSerialClearState)}`);
  await page.evaluate(() => {
    window.__pendingSerialCaptureOperations.shift().resolve();
  });
  await serialMonitor.locator(".serial-monitor-row").waitFor({ state: "detached" });
  await page.evaluate(() => {
    const pending = window.__pendingSerialCaptureReads.shift();
    pending.resolve(pending.result);
  });
  await page.waitForTimeout(100);
  assert(await serialMonitor.locator(".serial-monitor-row").count() === 0,
    "a stale main-window serial read restored frames after clearing");
  await page.evaluate(() => {
    window.__deferSerialCaptureReads = false;
    window.__serialCaptureFrames = [{
      id: "main-tx-after-clear",
      ts: new Date().toISOString(),
      direction: "outbound",
      bytes: [0x41, 0x46, 0x54, 0x45, 0x52],
      originalLength: 5,
      truncated: false,
    }];
  });
  await serialMonitor.locator(".serial-monitor-row", { hasText: "41 46 54 45 52" }).waitFor();
  assert(await serialMonitor.getAttribute("aria-busy") === "false",
    "main serial monitor did not resume polling after clearing");
  await serialMonitor.getByRole("button", { name: "导出可见串口帧", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingSerialCaptureOperations.length === 1);
  const mainSerialExportState = await page.evaluate(() => ({
    exportCalls: window.__invokeCalls.filter((call) => call.command === "export_serial_capture").length,
    pendingActions: window.__pendingSerialCaptureOperations.length,
    busy: document.querySelector(".serial-monitor")?.getAttribute("aria-busy"),
  }));
  assert(mainSerialExportState.exportCalls === 1
    && mainSerialExportState.pendingActions === 1
    && mainSerialExportState.busy === "true",
  `main serial export duplicated: ${JSON.stringify(mainSerialExportState)}`);
  await page.evaluate(() => {
    window.__pendingSerialCaptureOperations.shift().resolve();
    window.__deferSerialCaptureOperations = false;
    window.__serialCaptureFrames = [];
  });
  const mainSerialExportNotice = page.locator(".notice-dialog", { hasText: "串口捕获已导出" });
  await mainSerialExportNotice.waitFor();
  await mainSerialExportNotice.getByRole("button", { name: "确定", exact: true }).click();
  await togglePanel("历史命令");
  await serialMonitor.waitFor({ state: "detached" });
  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Edge Router" }).click();
  await page.waitForFunction(() => (
    document.querySelector(".workspace-dock-content.panel-explorer .tree-session.active")?.textContent?.includes("Edge Router")
  ));
  await fileManagerTab.click();
  await page.waitForFunction(() => document.querySelector('.workspace-dock[data-dock="left"]')?.getAttribute("data-active-panel") === "fileManager");
  await page.locator('.file-browser-pane[data-file-pane="remote"]').waitFor();
  await page.screenshot({ path: `${screenshotPrefix}-file-manager.png`, fullPage: true });

  const fileTitle = leftDock.locator('.workspace-dock-tab[data-panel="fileManager"]');
  const explorerTitle = leftDock.locator('.workspace-dock-tab[data-panel="explorer"]');
  const explorerTitleBox = await explorerTitle.boundingBox();
  assert(explorerTitleBox, "explorer title geometry is unavailable for same-dock reorder");
  const reorderTransfer = await page.evaluateHandle(() => new DataTransfer());
  await fileTitle.dispatchEvent("dragstart", { dataTransfer: reorderTransfer });
  await explorerTitle.dispatchEvent("dragover", {
    dataTransfer: reorderTransfer,
    clientX: explorerTitleBox.x + 2,
  });
  await explorerTitle.dispatchEvent("drop", {
    dataTransfer: reorderTransfer,
    clientX: explorerTitleBox.x + 2,
  });
  await page.waitForFunction(() => {
    const snapshot = JSON.parse(localStorage.getItem("portmate.workspacePanels.v2") || "null");
    return JSON.stringify(snapshot?.docks?.left) === JSON.stringify(["fileManager", "explorer"]);
  });
  assert(JSON.stringify(await leftDock.locator(".workspace-dock-tab").evaluateAll(
    (tabs) => tabs.map((tab) => tab.getAttribute("data-panel")),
  )) === JSON.stringify(["fileManager", "explorer"]), "same-dock title drag did not reorder dock tabs");
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
    return snapshot?.version === 7
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
  const advancedSendButton = sender.getByRole("button", { name: "高级发送选项", exact: true });
  assert(await sender.locator(".send-toolbar-primary > button").count() === 2
    && await sender.locator(".send-toolbar > svg").count() === 0
    && await advancedSendButton.getAttribute("aria-expanded") === "false"
    && await sender.locator(".send-advanced-controls").count() === 0,
  "sender did not keep low-frequency controls collapsed by default");
  await advancedSendButton.click();
  const advancedSendControls = sender.getByRole("group", { name: "高级发送选项", exact: true });
  await advancedSendControls.waitFor();
  assert(await advancedSendButton.getAttribute("aria-expanded") === "true"
    && await sender.getByRole("spinbutton", { name: "发送次数", exact: true }).inputValue() === "1"
    && await sender.getByRole("spinbutton", { name: "发送间隔（毫秒）", exact: true }).inputValue() === "1000",
  "sender advanced controls did not expose the existing default values");
  await page.screenshot({ path: `${screenshotPrefix}-sender-advanced.png`, fullPage: true });
  await sender.getByRole("combobox", { name: "发送目标", exact: true }).selectOption("connected");
  await advancedSendButton.click();
  assert(await advancedSendButton.getAttribute("data-active") === "true"
    && await sender.locator(".send-advanced-controls").count() === 0,
  "sender did not retain an active advanced-settings indicator after collapsing");
  await advancedSendButton.click();
  await sender.getByRole("combobox", { name: "发送目标", exact: true }).selectOption("active");
  await advancedSendButton.click();
  assert(await advancedSendButton.getAttribute("data-active") === "false",
    "sender advanced-settings indicator did not clear after restoring defaults");
  await page.screenshot({ path: `${screenshotPrefix}-sender.png`, fullPage: true });
  await sender.getByRole("button", { name: "发送", exact: true }).click();
  await sender.getByRole("textbox", { name: "send text", exact: true }).fill("uname -a");
  const senderLifecycleStart = await page.evaluate(() => {
    window.__deferTerminalSends = true;
    window.__pendingTerminalSends = [];
    return window.__invokeCalls.filter((call) => call.command === "send_text").length;
  });
  await activeTerminalInput.focus();
  await page.keyboard.press("q");
  await page.waitForFunction(() => window.__pendingTerminalSends.length === 1);
  const senderSendButton = sender.getByRole("button", { name: "发送", exact: true });
  await senderSendButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingTerminalSends.length === 1);
  const pendingSenderLifecycle = await page.evaluate((start) => ({
    calls: window.__invokeCalls.filter((call) => call.command === "send_text").length - start,
    pending: window.__pendingTerminalSends.length,
  }), senderLifecycleStart);
  assert(pendingSenderLifecycle.calls === 1
    && pendingSenderLifecycle.pending === 1
    && await senderSendButton.isDisabled(),
  `sender bypassed pending terminal input or did not lock: ${JSON.stringify(pendingSenderLifecycle)}`);
  await page.evaluate(() => {
    window.__deferTerminalSends = false;
    window.__pendingTerminalSends.shift().resolve(null);
  });
  await page.waitForFunction((start) => window.__invokeCalls
    .filter((call) => call.command === "send_text").length === start + 2, senderLifecycleStart);
  const orderedSenderWrites = await page.evaluate((start) => window.__invokeCalls
    .filter((call) => call.command === "send_text").slice(start)
    .map((call) => call.args.text), senderLifecycleStart);
  assert(JSON.stringify(orderedSenderWrites) === JSON.stringify(["q", "uname -a"]),
    `sender changed terminal input order: ${JSON.stringify(orderedSenderWrites)}`);
  await page.waitForFunction(() => window.__invokeCalls.filter((call) => call.command === "record_command_history").length === 2);
  await page.waitForFunction(() => JSON.parse(localStorage.getItem("portmate.commandHistory") || "null")?.entries?.[0]?.command === "uname -a");
  const recordedCommandHistory = await page.evaluate(() => ({
    commands: window.__commandHistory.entries.map((entry) => entry.command),
    calls: window.__invokeCalls.filter((call) => call.command === "record_command_history")
      .map((call) => call.args.command),
    revision: window.__commandHistory.revision,
  }));
  assert(JSON.stringify(recordedCommandHistory.calls) === JSON.stringify(["docker compose\nup -d", "uname -a"])
    && JSON.stringify(recordedCommandHistory.commands.slice(0, 4)) === JSON.stringify(["uname -a", "docker compose\nup -d", "cross-window during startup", "git status --short"])
    && recordedCommandHistory.revision === 4,
  `rapid commands were not serialized into canonical history: ${JSON.stringify(recordedCommandHistory)}`);

  await historyFilter.fill("");
  const invalidHistoryBaseline = await page.evaluate(() => ({
    revision: window.__commandHistory.revision,
    recordCalls: window.__invokeCalls.filter((call) => call.command === "record_command_history").length,
    local: localStorage.getItem("portmate.commandHistory"),
  }));
  const historyRowsBeforeInvalid = await page.locator(".history-list button").allTextContents();
  for (const invalidCommand of ["bad\0command", "界".repeat(8_193)]) {
    await sender.getByRole("textbox", { name: "send text", exact: true }).evaluate((textarea, value) => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
      setter?.call(textarea, value);
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    }, invalidCommand);
    await sender.getByRole("button", { name: "发送", exact: true }).click();
    await sender.getByRole("button", { name: "发送", exact: true }).waitFor({ state: "visible" });
  }
  await page.waitForTimeout(50);
  const invalidHistoryResult = await page.evaluate(() => ({
    revision: window.__commandHistory.revision,
    recordCalls: window.__invokeCalls.filter((call) => call.command === "record_command_history").length,
    local: localStorage.getItem("portmate.commandHistory"),
  }));
  const historyRowsAfterInvalid = await page.locator(".history-list button").allTextContents();
  assert(JSON.stringify(invalidHistoryResult) === JSON.stringify(invalidHistoryBaseline)
    && JSON.stringify(historyRowsAfterInvalid) === JSON.stringify(historyRowsBeforeInvalid),
  `invalid commands reached command history state or persistence: ${JSON.stringify({
    baseline: invalidHistoryBaseline,
    result: invalidHistoryResult,
    rowsBefore: historyRowsBeforeInvalid.length,
    rowsAfter: historyRowsAfterInvalid.length,
  })}`);

  await leftDock.locator('.workspace-dock-tab[data-panel="explorer"] .workspace-dock-tab-label').click();
  await leftDock.locator('.workspace-dock-content[data-panel="explorer"]').waitFor();
  const focusedDockPanels = await leftDock.evaluate((dock) => Object.fromEntries(
    [...dock.querySelectorAll(".workspace-dock-content")].map((panel) => [panel.getAttribute("data-panel"), panel.hasAttribute("hidden")]),
  ));
  assert(await leftDock.getAttribute("data-active-panel") === "explorer"
    && focusedDockPanels.explorer === false
    && focusedDockPanels.fileManager === true,
  `dock tab switch did not collapse inactive content: ${JSON.stringify(focusedDockPanels)}`);
  const uart = page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" });
  await uart.click();
  await page.waitForFunction(() => document.querySelector(".workspace-dock-content.panel-explorer .tree-session.active")?.textContent?.includes("Bench UART"));
  const serialControls = page.locator(".pane-serial-tools");
  assert(await serialControls.count() === 1,
    "serial line controls were lost while consolidating the workspace toolbar");
  const dtrButton = serialControls.getByRole("button", { name: "DTR", exact: true });
  const breakButton = serialControls.getByRole("button", { name: "BRK", exact: true });
  const serialControlStart = await page.evaluate(() => {
    window.__deferSerialControls = true;
    window.__pendingSerialControls = [];
    return window.__invokeCalls.length;
  });
  await dtrButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingSerialControls.length === 1);
  const pendingDtrControl = await page.evaluate((start) => ({
    calls: window.__invokeCalls.slice(start).filter((call) => call.command === "serial_set_lines").length,
    pending: window.__pendingSerialControls.map((request) => request.command),
  }), serialControlStart);
  assert(pendingDtrControl.calls === 1
    && JSON.stringify(pendingDtrControl.pending) === JSON.stringify(["serial_set_lines"])
    && await serialControls.getAttribute("aria-busy") === "true"
    && await serialControls.locator("button:disabled").count() === 3,
  `serial DTR control was duplicated or left sibling controls active: ${JSON.stringify(pendingDtrControl)}`);
  await page.evaluate(() => window.__pendingSerialControls.shift().resolve());
  await page.waitForFunction(() => document.querySelector('.pane-serial-tools button[aria-pressed="true"]')?.textContent === "DTR");
  await dtrButton.waitFor({ state: "visible" });
  assert(!await dtrButton.isDisabled(), "serial controls did not unlock after the DTR response");
  const breakControlStart = await page.evaluate(() => window.__invokeCalls.length);
  await breakButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingSerialControls.length === 1);
  const pendingBreakControl = await page.evaluate((start) => ({
    calls: window.__invokeCalls.slice(start).filter((call) => call.command === "serial_send_break").length,
    pending: window.__pendingSerialControls.map((request) => request.command),
  }), breakControlStart);
  assert(pendingBreakControl.calls === 1
    && JSON.stringify(pendingBreakControl.pending) === JSON.stringify(["serial_send_break"])
    && await serialControls.locator("button:disabled").count() === 3,
  `serial Break control was duplicated or left sibling controls active: ${JSON.stringify(pendingBreakControl)}`);
  await page.evaluate(() => {
    window.__deferSerialControls = false;
    window.__pendingSerialControls.shift().resolve();
  });
  await page.waitForFunction(() => document.querySelectorAll(".pane-serial-tools button:disabled").length === 0);

  const terminalInputBeforeModal = page.locator(".terminal-pane.active .xterm-helper-textarea");
  await terminalInputBeforeModal.focus();
  const modalBoundaryBaseline = await page.evaluate(() => ({
    activePaneId: document.querySelector(".terminal-pane.active")?.getAttribute("data-pane-id"),
    focusMode: document.querySelector('[aria-label="进入专注模式"], [aria-label="退出专注模式"]')?.getAttribute("aria-pressed"),
    sendTextCalls: window.__invokeCalls.filter((call) => call.command === "send_text").length,
  }));
  await page.evaluate(() => document.querySelector('[aria-label="搜索会话"]')?.click());
  await page.locator(".search-dialog").waitFor();
  await page.waitForFunction(() => Boolean(document.activeElement?.closest(".search-dialog")));
  await page.waitForFunction(() => Boolean(
    document.querySelector(".terminal-pane.active .xterm-helper-textarea")?.closest("[inert]"),
  ));
  const modalBoundaryResult = await page.evaluate(async () => {
    const terminalInput = document.querySelector(".terminal-pane.active .xterm-helper-textarea");
    const terminalCanvas = document.querySelector(".terminal-pane.active .terminal-canvas");
    const backgroundTarget = terminalInput;
    const backgroundInert = Boolean(terminalInput?.closest("[inert]"));
    terminalInput?.dispatchEvent(new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      code: "Enter",
      key: "Enter",
      altKey: true,
    }));
    terminalInput?.dispatchEvent(new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      code: "KeyA",
      key: "a",
      ctrlKey: true,
      shiftKey: true,
    }));
    backgroundTarget?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true }));
    backgroundTarget?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    const blockedPointerEvents = Object.fromEntries([
      ["mouseup", new MouseEvent("mouseup", { bubbles: true, cancelable: true })],
      ["auxclick", new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 })],
      ["contextmenu", new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 })],
      ["wheel", new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: 40 })],
    ].map(([name, event]) => {
      backgroundTarget?.dispatchEvent(event);
      return [name, event.defaultPrevented];
    }));
    const selectionEvent = new Event("selectstart", { bubbles: true, cancelable: true });
    terminalInput?.dispatchEvent(selectionEvent);
    const terminalTarget = {
      sessionId: terminalCanvas?.getAttribute("data-terminal-session-id"),
      viewId: terminalCanvas?.getAttribute("data-terminal-view-id"),
    };
    const commandResponses = {};
    for (const [type, detail] of [
      ["portmate-terminal-selection", { ...terminalTarget, action: "select-all" }],
      ["portmate-terminal-buffer-action", { ...terminalTarget, action: "clear-all" }],
      ["portmate-terminal-text-export", { ...terminalTarget, source: "buffer" }],
    ]) {
      window.dispatchEvent(new CustomEvent(type, {
        detail: { ...detail, respond: (response) => { commandResponses[type] = response; } },
      }));
    }
    for (const type of [
      "portmate-terminal-search",
      "portmate-terminal-goto-line",
      "portmate-terminal-free-input",
    ]) window.dispatchEvent(new Event(type));
    (terminalInput instanceof HTMLElement ? terminalInput : null)?.focus();
    await new Promise((resolve) => setTimeout(resolve, 40));
    return {
      activePaneId: document.querySelector(".terminal-pane.active")?.getAttribute("data-pane-id"),
      backgroundInert,
      focusMode: document.querySelector('[aria-label="进入专注模式"], [aria-label="退出专注模式"]')?.getAttribute("aria-pressed"),
      focusInsideModal: Boolean(document.activeElement?.closest(".search-dialog")),
      blockedPointerEvents,
      selectionBlocked: selectionEvent.defaultPrevented,
      commandResponses,
      terminalToolsOpened: Boolean(document.querySelector(
        ".terminal-pane.active .terminal-search-bar, .terminal-pane.active .terminal-goto-line, .terminal-pane.active .terminal-free-input",
      )),
      sendTextCalls: window.__invokeCalls.filter((call) => call.command === "send_text").length,
    };
  });
  assert(modalBoundaryResult.activePaneId === modalBoundaryBaseline.activePaneId
    && modalBoundaryResult.backgroundInert
    && modalBoundaryResult.focusMode === modalBoundaryBaseline.focusMode
    && modalBoundaryResult.focusInsideModal
    && Object.values(modalBoundaryResult.blockedPointerEvents).every(Boolean)
    && modalBoundaryResult.selectionBlocked
    && Object.keys(modalBoundaryResult.commandResponses).length === 3
    && Object.values(modalBoundaryResult.commandResponses).every((response) => response?.ok === false && response.error.includes("顶层对话框"))
    && !modalBoundaryResult.terminalToolsOpened
    && modalBoundaryResult.sendTextCalls === modalBoundaryBaseline.sendTextCalls,
  `modal interaction leaked into the terminal workspace: ${JSON.stringify({ modalBoundaryBaseline, modalBoundaryResult })}`);
  await page.getByRole("combobox", { name: "搜索会话和日志", exact: true }).press("Escape");
  await page.locator(".search-dialog").waitFor({ state: "detached" });
  await page.waitForFunction(() => document.activeElement?.classList.contains("xterm-helper-textarea"));
  assert(!await page.evaluate(() => Boolean(document.querySelector(".wind-root > [inert]"))),
    "closing a modal left the workspace inert");

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
  const activePaneBeforeSwitch = page.locator(".terminal-pane.active");
  const activeCanvasBeforeSwitch = activePaneBeforeSwitch.locator(".terminal-canvas");
  const activeSelectionTarget = {
    sessionId: await activeCanvasBeforeSwitch.getAttribute("data-terminal-session-id"),
    viewId: await activeCanvasBeforeSwitch.getAttribute("data-terminal-view-id"),
  };
  assert(activeSelectionTarget.sessionId && activeSelectionTarget.viewId,
    `active pane did not expose its terminal identity: ${JSON.stringify(activeSelectionTarget)}`);
  const activeSelectionResult = await page.evaluate(({ sessionId, viewId }) => new Promise((resolve) => {
    window.dispatchEvent(new CustomEvent("portmate-terminal-selection", {
      detail: { sessionId, viewId, action: "select-all", respond: resolve },
    }));
  }), activeSelectionTarget);
  assert(activeSelectionResult?.ok, `active pane could not establish a local selection: ${JSON.stringify(activeSelectionResult)}`);
  const previousActivePaneId = await activePaneBeforeSwitch.getAttribute("data-pane-id");
  const nextPane = page.locator(".terminal-pane:not(.active)");
  const nextPaneId = await nextPane.getAttribute("data-pane-id");
  const nextPaneHostBounds = await nextPane.locator(".terminal-host").boundingBox();
  assert(nextPaneHostBounds, "inactive pane terminal geometry is unavailable");
  await page.mouse.click(nextPaneHostBounds.x + 40, nextPaneHostBounds.y + 40);
  await page.waitForFunction((paneId) => document.querySelector(".terminal-pane.active")?.getAttribute("data-pane-id") === paneId, nextPaneId);
  const paneInteractionBoundary = await page.evaluate(async (oldPaneId) => {
    const activePane = document.querySelector(".terminal-pane.active");
    const inactivePane = [...document.querySelectorAll(".terminal-pane")]
      .find((pane) => pane.getAttribute("data-pane-id") === oldPaneId);
    const activeCanvas = activePane?.querySelector(".terminal-canvas");
    const inactiveCanvas = inactivePane?.querySelector(".terminal-canvas");
    const inactiveInput = inactivePane?.querySelector(".xterm-helper-textarea");
    const sendCallsBefore = window.__invokeCalls.filter((call) => call.command === "send_text").length;
    inactiveInput?.focus();
    inactiveInput?.dispatchEvent(new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      key: "x",
      code: "KeyX",
    }));
    inactiveInput?.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      cancelable: true,
      data: "inactive-pane-probe",
      inputType: "insertText",
    }));
    await new Promise((resolve) => setTimeout(resolve, 40));
    const staleSelection = inactivePane?.querySelector(".terminal-host")?.getAttribute("data-terminal-has-selection");
    activePane?.querySelector(".xterm-helper-textarea")?.focus();
    const activeText = activePane?.querySelector(".workspace-pane-tab-label span:last-child")?.firstChild;
    const inactiveText = inactivePane?.querySelector(".workspace-pane-tab-label span:last-child")?.firstChild;
    const selection = document.getSelection();
    selection?.removeAllRanges();
    if (activeText && inactiveText && selection) {
      selection.setBaseAndExtent(activeText, 0, inactiveText, inactiveText.textContent?.length ?? 0);
      document.dispatchEvent(new Event("selectionchange"));
    }
    return {
      activeCanvasFocused: activeCanvas?.getAttribute("data-terminal-focused"),
      inactiveCanvasFocused: inactiveCanvas?.getAttribute("data-terminal-focused"),
      inactiveCanvasInert: inactiveCanvas?.hasAttribute("inert"),
      inactiveReceivedFocus: document.activeElement === inactiveInput,
      staleSelection,
      selectionAfterCrossPaneDrag: selection?.toString() ?? "",
      sendTextCalls: window.__invokeCalls.filter((call) => call.command === "send_text").length - sendCallsBefore,
    };
  }, previousActivePaneId);
  assert(paneInteractionBoundary.activeCanvasFocused === "true"
    && paneInteractionBoundary.inactiveCanvasFocused === "false"
    && paneInteractionBoundary.inactiveCanvasInert
    && !paneInteractionBoundary.inactiveReceivedFocus
    && paneInteractionBoundary.staleSelection !== "true"
    && paneInteractionBoundary.selectionAfterCrossPaneDrag === ""
    && paneInteractionBoundary.sendTextCalls === 0,
  `interaction crossed active/inactive pane boundaries: ${JSON.stringify(paneInteractionBoundary)}`);
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
  const terminalExportDirectory = page.getByRole("textbox", { name: "终端文本默认导出目录", exact: true });
  assert(await page.getByRole("combobox", { name: "会话 1:", exact: true }).count() === 1
    && await page.getByRole("combobox", { name: "会话 2:", exact: true }).count() === 1,
  "startup session selectors do not expose stable accessible names");
  assert(await terminalExportDirectory.count() === 1
    && await page.getByRole("button", { name: "选择终端文本导出目录", exact: true }).count() === 1,
  "terminal export directory setting is incomplete");
  const terminalExportDirectoryPickerBaseline = await page.evaluate(() => {
    window.__deferTerminalExportDirectoryPicker = true;
    window.__pendingTerminalExportDirectoryPickers = [];
    return window.__invokeCalls.filter((call) => call.command === "plugin:dialog|open").length;
  });
  const chooseTerminalExportDirectoryButton = page.getByRole("button", {
    name: "选择终端文本导出目录",
    exact: true,
  });
  await chooseTerminalExportDirectoryButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingTerminalExportDirectoryPickers.length === 1);
  assert(await chooseTerminalExportDirectoryButton.isDisabled(),
    "a pending terminal export directory picker remained actionable");
  await page.evaluate(() => {
    window.__deferTerminalExportDirectoryPicker = false;
    window.__pendingTerminalExportDirectoryPickers.shift().resolve("/tmp/picked-terminal-text");
  });
  await page.waitForFunction(() => document.querySelector('[aria-label="终端文本默认导出目录"]')?.value === "/tmp/picked-terminal-text");
  const terminalExportDirectoryPickerCalls = await page.evaluate((baseline) => (
    window.__invokeCalls.filter((call) => call.command === "plugin:dialog|open").length - baseline
  ), terminalExportDirectoryPickerBaseline);
  assert(terminalExportDirectoryPickerCalls === 1,
    `terminal export directory picker opened ${terminalExportDirectoryPickerCalls} times`);
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
  await terminalExportDirectory.fill("/tmp/portmate-terminal-text");
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
    return {
      mode: prefs.startupMode,
      sessions: prefs.startupSessions,
      terminalTextExportDirectory: prefs.terminalTextExportDirectory,
      legacyKeys,
    };
  });
  assert(startupSettings.mode === "specific"
    && JSON.stringify(startupSettings.sessions) === JSON.stringify(["bench-uart", "local-shell", "", ""])
    && startupSettings.terminalTextExportDirectory === "/tmp/portmate-terminal-text"
    && startupSettings.legacyKeys.length === 0,
  `startup settings did not persist exact profile IDs: ${JSON.stringify(startupSettings)}`);
  await openTerminalSettings();
  assert(await startupSelect(1).inputValue() === "bench-uart"
    && await startupSelect(2).inputValue() === "local-shell"
    && await page.getByRole("textbox", { name: "终端文本默认导出目录", exact: true }).inputValue() === "/tmp/portmate-terminal-text",
  "saved startup or terminal export preferences did not restore in the settings dialog");
  await page.setViewportSize({ width: 390, height: 844 });
  const mobileTerminalExportSetting = await page.locator(".terminal-export-path-setting").evaluate((setting) => {
    const dialog = setting.closest(".terminal-settings-dialog");
    const control = setting.querySelector(".setting-path-control");
    const input = setting.querySelector("input");
    const button = setting.querySelector("button");
    const dialogRect = dialog.getBoundingClientRect();
    const controlRect = control.getBoundingClientRect();
    const inputRect = input.getBoundingClientRect();
    const buttonRect = button.getBoundingClientRect();
    return {
      dialogLeft: dialogRect.left,
      dialogRight: dialogRect.right,
      dialogWidth: dialogRect.width,
      dialogScrollWidth: dialog.scrollWidth,
      controlLeft: controlRect.left,
      controlRight: controlRect.right,
      inputWidth: inputRect.width,
      buttonWidth: buttonRect.width,
    };
  });
  assert(mobileTerminalExportSetting.dialogLeft >= 0
    && mobileTerminalExportSetting.dialogRight <= 390
    && mobileTerminalExportSetting.dialogScrollWidth <= mobileTerminalExportSetting.dialogWidth
    && mobileTerminalExportSetting.controlLeft >= mobileTerminalExportSetting.dialogLeft
    && mobileTerminalExportSetting.controlRight <= mobileTerminalExportSetting.dialogRight
    && mobileTerminalExportSetting.inputWidth >= 100
    && mobileTerminalExportSetting.buttonWidth === 34,
  `terminal export directory controls overflow on mobile: ${JSON.stringify(mobileTerminalExportSetting)}`);
  await page.screenshot({ path: `${screenshotPrefix}-terminal-export-settings-mobile.png`, fullPage: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await terminalExportDirectory.fill("/tmp/private-unsaved-terminal-export");
  await page.evaluate(() => {
    window.__terminalSettingsDiscardPrompts = [];
    window.__originalTerminalSettingsConfirm = window.confirm;
    window.confirm = (message) => {
      window.__terminalSettingsDiscardPrompts.push(String(message));
      return false;
    };
  });
  await page.locator(".terminal-settings-dialog .dialog-actions button", { hasText: "取消" }).click();
  assert(await page.locator(".terminal-settings-dialog").isVisible()
    && await terminalExportDirectory.inputValue() === "/tmp/private-unsaved-terminal-export",
  "terminal settings discarded an unsaved path after close cancellation");
  await page.evaluate(() => {
    window.confirm = (message) => {
      window.__terminalSettingsDiscardPrompts.push(String(message));
      return true;
    };
  });
  await page.getByRole("button", { name: "关闭终端设置", exact: true }).click();
  await page.locator(".terminal-settings-dialog").waitFor({ state: "detached" });
  const terminalSettingsDiscardState = await page.evaluate(() => {
    const prompts = window.__terminalSettingsDiscardPrompts;
    window.confirm = window.__originalTerminalSettingsConfirm;
    return {
      prompts,
      savedPath: JSON.parse(localStorage.getItem("portmate.terminalPrefs")).terminalTextExportDirectory,
    };
  });
  assert(terminalSettingsDiscardState.savedPath === "/tmp/portmate-terminal-text"
    && terminalSettingsDiscardState.prompts.length === 2
    && terminalSettingsDiscardState.prompts.every((prompt) => prompt.includes("未保存的更改"))
    && terminalSettingsDiscardState.prompts.every((prompt) => !prompt.includes("private-unsaved-terminal-export")),
  `terminal settings draft was persisted or exposed without confirmation: ${JSON.stringify(terminalSettingsDiscardState)}`);

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
  const createSessionDialog = page.locator(".session-settings-dialog.quick");
  const quickProtocolLabels = await createSessionDialog.getByRole("tab").allTextContents();
  assert(JSON.stringify(quickProtocolLabels.map((label) => label.trim())) === JSON.stringify([
    "Shell", "SSH", "Tmux", "Telnet", "TCP", "Serial",
  ]), `quick session protocols are incomplete: ${JSON.stringify(quickProtocolLabels)}`);
  assert(await createSessionDialog.getByRole("tab", { name: "Serial", exact: true }).getAttribute("aria-selected") === "true",
    "new session dialog did not select the draft protocol");
  await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "串口");
  const quickConnectButton = createSessionDialog.getByRole("button", { name: "连接", exact: true });
  const quickSaveButton = createSessionDialog.getByRole("button", { name: "仅保存", exact: true });
  assert(await quickConnectButton.isDisabled() && !await quickSaveButton.isDisabled(),
    "incomplete quick session draft blocks saving or allows connecting");
  const quickSerialPort = createSessionDialog.getByRole("combobox", { name: "串口", exact: true });
  await quickSerialPort.selectOption("/dev/ttyUSB0");
  assert(!await quickConnectButton.isDisabled(), "a valid serial target did not enable quick connect");

  await createSessionDialog.getByRole("tab", { name: "Shell", exact: true }).click();
  const addShellArgument = createSessionDialog.getByRole("button", { name: "添加参数", exact: true });
  for (let index = 0; index < 4; index += 1) await addShellArgument.click();
  const shellArguments = createSessionDialog.getByRole("group", { name: "Shell 参数列表", exact: true }).locator("input");
  const shellScript = " printf '%s\\n' \"hello world\" ";
  await createSessionDialog.getByRole("textbox", { name: "Shell 参数 1", exact: true }).fill("-c");
  await createSessionDialog.getByRole("textbox", { name: "Shell 参数 2", exact: true }).fill(shellScript);
  await createSessionDialog.getByRole("textbox", { name: "Shell 参数 4", exact: true }).fill("remove-me");
  assert(JSON.stringify(await shellArguments.evaluateAll((inputs) => inputs.map((input) => input.value)))
    === JSON.stringify(["-c", shellScript, "", "remove-me"]),
  "quick Shell arguments did not preserve spaces, quotes, or an empty argv entry");
  await createSessionDialog.getByRole("button", { name: "上移 Shell 参数 2", exact: true }).click();
  assert(JSON.stringify(await shellArguments.evaluateAll((inputs) => inputs.map((input) => input.value)))
    === JSON.stringify([shellScript, "-c", "", "remove-me"]),
  "quick Shell argument move did not preserve exact entries");
  await createSessionDialog.getByRole("button", { name: "下移 Shell 参数 1", exact: true }).click();
  await createSessionDialog.getByRole("button", { name: "删除 Shell 参数 4", exact: true }).click();
  assert(JSON.stringify(await shellArguments.evaluateAll((inputs) => inputs.map((input) => input.value)))
    === JSON.stringify(["-c", shellScript, ""]),
  "quick Shell argument delete changed a retained argv entry");
  await page.screenshot({ path: `${screenshotPrefix}-session-create-shell.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForFunction(() => (document.querySelector(".session-settings-dialog.quick")?.getBoundingClientRect().height ?? 0) >= 695);
  const mobileShellCreateBounds = await createSessionDialog.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    const formRect = dialog.querySelector(".session-quick-form")?.getBoundingClientRect();
    const cwdRect = dialog.querySelector('[aria-label="Shell 工作目录"]')?.getBoundingClientRect();
    return {
      left: rect.left,
      right: rect.right,
      width: rect.width,
      scrollWidth: dialog.scrollWidth,
      formBottom: formRect?.bottom ?? 0,
      cwdBottom: cwdRect?.bottom ?? Number.POSITIVE_INFINITY,
    };
  });
  assert(mobileShellCreateBounds.left >= 8 && mobileShellCreateBounds.right <= 382
    && mobileShellCreateBounds.scrollWidth <= mobileShellCreateBounds.width
    && mobileShellCreateBounds.cwdBottom <= mobileShellCreateBounds.formBottom,
  `quick Shell argument editor overflows on mobile: ${JSON.stringify(mobileShellCreateBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-session-create-shell-mobile.png`, fullPage: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.waitForFunction(() => (document.querySelector(".session-settings-dialog.quick")?.getBoundingClientRect().height ?? Number.POSITIVE_INFINITY) <= 645);

  await createSessionDialog.getByRole("tab", { name: "SSH", exact: true }).click();
  await createSessionDialog.getByRole("textbox", { name: "SSH 主机或 IP", exact: true }).fill("router.local");
  await createSessionDialog.getByRole("textbox", { name: "SSH 用户名", exact: true }).fill("root");
  assert(!await quickConnectButton.isDisabled(), "a valid SSH target did not enable quick connect");
  await createSessionDialog.getByRole("tab", { name: "TCP", exact: true }).click();
  await createSessionDialog.getByRole("textbox", { name: "TCP 主机", exact: true }).fill("10.0.0.8");
  await createSessionDialog.getByRole("spinbutton", { name: "TCP 端口", exact: true }).fill("443");
  const quickTlsToggle = createSessionDialog.getByRole("button", { name: "TLS", exact: true });
  await quickTlsToggle.click();
  assert(await quickTlsToggle.getAttribute("aria-pressed") === "true",
    "quick TCP TLS toggle did not update the draft");
  await createSessionDialog.getByRole("tab", { name: "Serial", exact: true }).click();
  assert(await createSessionDialog.getByRole("combobox", { name: "串口", exact: true }).inputValue() === "/dev/ttyUSB0",
    "switching quick protocols discarded the serial draft");
  await createSessionDialog.getByRole("tab", { name: "SSH", exact: true }).click();
  assert(await createSessionDialog.getByRole("textbox", { name: "SSH 主机或 IP", exact: true }).inputValue() === "router.local"
    && await createSessionDialog.getByRole("textbox", { name: "SSH 用户名", exact: true }).inputValue() === "root",
    "switching quick protocols discarded the SSH host or username draft");
  await createSessionDialog.getByRole("tab", { name: "Shell", exact: true }).click();
  assert(JSON.stringify(await createSessionDialog.getByRole("group", { name: "Shell 参数列表", exact: true })
    .locator("input").evaluateAll((inputs) => inputs.map((input) => input.value)))
    === JSON.stringify(["-c", shellScript, ""]),
  "switching quick protocols discarded or reparsed the Shell argv draft");
  await createSessionDialog.getByRole("tab", { name: "SSH", exact: true }).click();

  assert(await createSessionDialog.getByRole("heading", { name: "会话信息", exact: true }).count() === 1,
    "quick session metadata is still collapsed or uses the old label");
  const profileNameInput = createSessionDialog.getByRole("textbox", { name: "会话名称", exact: true });
  const profileGroupInput = createSessionDialog.getByRole("textbox", { name: "会话分组", exact: true });
  const profileTagsInput = createSessionDialog.getByRole("textbox", { name: "会话标签", exact: true });
  assert(await profileNameInput.getAttribute("placeholder") === null
    && await profileGroupInput.getAttribute("placeholder") === null,
    "session name or group still shows redundant placeholder guidance");
  await profileNameInput.fill("😀".repeat(129));
  await profileGroupInput.fill("g".repeat(257));
  await profileTagsInput.fill("alpha");
  await profileTagsInput.press("End");
  await profileTagsInput.pressSequentially(", beta");
  assert(Array.from(await profileNameInput.inputValue()).length === 128
    && Array.from(await profileGroupInput.inputValue()).length === 256
    && await profileTagsInput.inputValue() === "alpha, beta",
  "session metadata bounds or incremental comma-separated tag editing failed");
  const quickSessionBounds = await createSessionDialog.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return { width: rect.width, height: rect.height, scrollWidth: dialog.scrollWidth, scrollHeight: dialog.scrollHeight };
  });
  assert(quickSessionBounds.width <= 720 && quickSessionBounds.height <= 640
    && quickSessionBounds.scrollWidth <= quickSessionBounds.width
    && quickSessionBounds.scrollHeight <= quickSessionBounds.height,
  `quick session dialog is not compact: ${JSON.stringify(quickSessionBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-session-create.png`, fullPage: true });
  await createSessionDialog.getByRole("button", { name: "高级设置", exact: true }).click();
  const protocolSelect = page.getByRole("combobox", { name: "会话类型", exact: true });
  const sectionSelect = page.getByRole("combobox", { name: "会话配置项", exact: true });
  await protocolSelect.waitFor();
  await page.getByRole("button", { name: "快速设置", exact: true }).click();
  const returnedQuickDialog = page.locator(".session-settings-dialog.quick");
  assert(await returnedQuickDialog.getByRole("textbox", { name: "SSH 主机或 IP", exact: true }).inputValue() === "router.local"
    && await returnedQuickDialog.getByRole("textbox", { name: "SSH 用户名", exact: true }).inputValue() === "root",
    "returning from advanced session settings discarded the SSH host or username draft");
  await returnedQuickDialog.getByRole("button", { name: "高级设置", exact: true }).click();
  await protocolSelect.waitFor();
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
  await sectionSelect.selectOption("SSH");
  assert(await page.getByLabel("主机:(H)", { exact: true }).inputValue() === "router.local"
    && await page.getByLabel("用户名:(U)", { exact: true }).inputValue() === "root",
    "advanced SSH settings recombined or discarded the separate host and username fields");
  await sectionSelect.selectOption("公钥");
  const authOrderSelect = page.locator(".session-settings-dialog .dialog-field", { hasText: "顺序:(O)" }).locator("select");
  const authOrderOptions = await authOrderSelect.locator("option").evaluateAll((options) => options.map((option) => option.value));
  assert(authOrderOptions.length === 15
    && new Set(authOrderOptions).size === 15
    && authOrderOptions.includes("keyboard-interactive>public-key")
    && authOrderOptions.includes("password>keyboard-interactive>public-key"),
  `SSH authentication orders are incomplete: ${JSON.stringify(authOrderOptions)}`);
  await authOrderSelect.selectOption("keyboard-interactive>public-key");
  assert(await authOrderSelect.inputValue() === "keyboard-interactive>public-key",
    "SSH authentication order selector did not retain a valid non-default order");
  const recordAuthSuccess = page.locator(".session-settings-dialog .dialog-field", { hasText: "记住成功方式:(R)" }).getByRole("button");
  assert(await recordAuthSuccess.getAttribute("aria-pressed") === "true", "SSH successful-auth recording is not enabled by default");
  await recordAuthSuccess.click();
  assert(await recordAuthSuccess.getAttribute("aria-pressed") === "false", "SSH successful-auth recording cannot be disabled from Session Settings");
  await sectionSelect.selectOption("验证");
  await page.waitForFunction(() => (
    document.querySelector('select[aria-label="会话类型"]')?.value === "SSH"
      && document.querySelector('select[aria-label="会话配置项"]')?.value === "验证"
  ));
  const sshHealthButton = page.locator(".session-settings-dialog .ssh-health-check")
    .getByRole("button", { name: "检查 SSH 健康", exact: true });
  assert(await sshHealthButton.count() === 1, `SSH health action is unavailable: ${JSON.stringify({
    protocol: await protocolSelect.inputValue(),
    section: await sectionSelect.inputValue(),
    form: await page.locator(".session-settings-dialog .session-form").textContent(),
  })}`);
  await sshHealthButton.click();
  await page.getByText("健康 · russh · 公钥 · SSH 7 ms · Channel 11 ms · SFTP 13 ms", { exact: true }).waitFor();
  const sshHealthCall = await page.evaluate(() => window.__invokeCalls.findLast((call) => call.command === "check_ssh_health"));
  assert(sshHealthCall?.args?.probeSftp === true,
    `SSH health UI omitted the SFTP probe: ${JSON.stringify(sshHealthCall)}`);
  await page.screenshot({ path: `${screenshotPrefix}-ssh-health.png`, fullPage: true });
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
  await sectionSelect.selectOption("串口");
  const baudRateInput = page.locator('.session-settings-dialog input[list="serial-baud-rate-options"]');
  const baudRateOptions = await page.locator("#serial-baud-rate-options option")
    .evaluateAll((options) => options.map((option) => Number(option.value)));
  assert(JSON.stringify(baudRateOptions) === JSON.stringify([
    110, 300, 600, 1200, 2400, 4800, 9600, 14400, 19200, 38400, 57600, 115200, 230400, 460800, 921600,
    1000000, 1500000,
  ]), `Serial baud-rate suggestions are incomplete: ${JSON.stringify(baudRateOptions)}`);
  await baudRateInput.fill("250000");
  assert(await baudRateInput.inputValue() === "250000",
    "Serial baud-rate suggestions prevent a custom adapter rate");
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

  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" }).click();
  await page.waitForFunction(() => (
    document.querySelector(".workspace-dock-content.panel-explorer .tree-session.active")?.textContent?.includes("Bench UART")
  ));
  const serialProfileSaveStart = await page.evaluate(() => window.__invokeCalls.length);
  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "会话设置" }).click();
  const serialProfileDialog = page.locator(".session-settings-dialog");
  await serialProfileDialog.waitFor();
  await serialProfileDialog.getByRole("combobox", { name: "会话配置项", exact: true }).selectOption("串口");
  assert(await serialProfileDialog.locator(".dialog-field", { hasText: "串口:(S)" }).locator("select").inputValue() === "/dev/ttyUSB0 ",
    "Session Settings did not preserve the exact serial device path");
  await serialProfileDialog.getByRole("button", { name: "保存", exact: true }).click();
  await serialProfileDialog.waitFor({ state: "detached" });
  const savedSerialPath = await page.evaluate((start) => (
    window.__invokeCalls.slice(start)
      .find((call) => call.command === "save_session_profile")
      ?.args.profile.connection.port ?? null
  ), serialProfileSaveStart);
  assert(savedSerialPath === "/dev/ttyUSB0 ",
    `saving a Serial Profile changed its exact device path: ${JSON.stringify(savedSerialPath)}`);

  const sessionPreferenceKeys = await page.evaluate(() => (
    Object.keys(localStorage).filter((key) => key.startsWith("portmate.sessionPrefs."))
  ));
  assert(sessionPreferenceKeys.length === 0,
    `closing session settings persisted non-runtime preferences: ${JSON.stringify(sessionPreferenceKeys)}`);

  async function setActiveSessionTheme(theme, backgroundOpacity = 100) {
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
      "背景不透明度:(O)": { min: "20", max: "100", maxLength: null },
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
    await dialog.locator(".dialog-field", { hasText: "背景不透明度:" }).locator('input[type="range"]').fill(String(backgroundOpacity));
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
  await setActiveSessionTheme("portmate-light", 55);
  await page.locator('.terminal-pane.active .terminal-host[data-terminal-theme="portmate-light"][data-terminal-opacity="55"]').waitFor();
  const lightThemeState = await page.locator(".terminal-pane.active .terminal-canvas").evaluate((canvas) => ({
    background: getComputedStyle(canvas).backgroundColor,
    retained: canvas.querySelector(".xterm")?.dataset.themeTestIdentity ?? "",
    xtermBackground: getComputedStyle(canvas.querySelector(".xterm-viewport")).backgroundColor,
    renderer: canvas.querySelector(".terminal-host")?.getAttribute("data-terminal-renderer"),
  }));
  const lightTerminalPixel = await samplePngCenter(browser, await page.locator(".terminal-pane.active .terminal-canvas").screenshot());
  assert(lightThemeState.background === "rgba(0, 0, 0, 0)"
    && lightThemeState.xtermBackground === "rgba(0, 0, 0, 0)"
    && lightThemeState.renderer === "dom"
    && lightThemeState.retained === "retained"
    && lightTerminalPixel[0] >= 138 && lightTerminalPixel[0] <= 146
    && lightTerminalPixel[1] >= 140 && lightTerminalPixel[1] <= 148
    && lightTerminalPixel[2] >= 144 && lightTerminalPixel[2] <= 152,
    `terminal theme did not update in place: ${JSON.stringify({ lightThemeState, lightTerminalPixel })}`);
  await page.screenshot({ path: `${screenshotPrefix}-terminal-light-theme.png`, fullPage: true });
  await setActiveSessionTheme("portmate-dark");
  await page.locator('.terminal-pane.active .terminal-host[data-terminal-theme="portmate-dark"][data-terminal-renderer="webgl"]').waitFor();
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
  await detachedPage.locator('.detached-pane-terminal .terminal-host[data-terminal-theme="portmate-dark"][data-terminal-ready="true"]').waitFor();
  const detachedConnectedIndicator = await detachedPage.locator(".detached-pane-toolbar .session-status-dot").evaluate((indicator) => ({
    color: getComputedStyle(indicator).backgroundColor,
    width: getComputedStyle(indicator).width,
    height: getComputedStyle(indicator).height,
    status: indicator.getAttribute("role"),
    label: indicator.getAttribute("aria-label"),
  }));
  assert(detachedConnectedIndicator.color === "rgb(55, 214, 122)"
    && detachedConnectedIndicator.width === "7px"
    && detachedConnectedIndicator.height === "7px"
    && detachedConnectedIndicator.status === "status"
    && detachedConnectedIndicator.label?.startsWith("已连接"),
    `detached terminal did not expose its connected status indicator: ${JSON.stringify(detachedConnectedIndicator)}`);
  await detachedPage.waitForFunction(() => (
    window.__tauriEventListeners.get("portmate-session-profile-updated")?.length ?? 0
  ) > 0);
  const detachedTerminalInstanceId = await detachedPage
    .locator(".detached-pane-terminal .terminal-host")
    .getAttribute("data-terminal-instance-id");
  assert(detachedTerminalInstanceId, "detached terminal did not expose its mounted XTerm instance");
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
    instanceId: canvas.querySelector(".terminal-host")?.getAttribute("data-terminal-instance-id"),
  }));
  assert(detachedThemeState.background === "rgb(247, 248, 250)"
    && detachedThemeState.instanceId === detachedTerminalInstanceId
    && detachedPageErrors.length === 0,
  `detached profile update did not apply in place: ${JSON.stringify({ detachedThemeState, detachedPageErrors })}`);
  await detachedPage.evaluate(() => { window.__deferSessionLists = true; });
  await detachedPage.waitForFunction(() => window.__pendingSessionLists.length > 0);
  await detachedPage.evaluate(() => {
    const index = window.__sessions.findIndex((session) => session.profile.id === "local-shell");
    const updated = structuredClone(window.__sessions[index]);
    updated.profile.terminal.theme = "graphite";
    window.__sessions[index] = updated;
    window.__emitTauriEvent("portmate-session-profile-updated", updated);
  });
  await detachedPage.locator('.detached-pane-terminal .terminal-host[data-terminal-theme="graphite"]').waitFor();
  await detachedPage.evaluate(() => {
    for (const pending of window.__pendingSessionLists) pending.resolve(pending.result);
    window.__pendingSessionLists = [];
    window.__deferSessionLists = false;
  });
  await detachedPage.waitForTimeout(100);
  const detachedStaleRefreshState = await detachedPage.locator(".detached-pane-terminal .terminal-canvas").evaluate((canvas) => ({
    theme: canvas.querySelector(".terminal-host")?.getAttribute("data-terminal-theme"),
    background: getComputedStyle(canvas).backgroundColor,
    instanceId: canvas.querySelector(".terminal-host")?.getAttribute("data-terminal-instance-id"),
  }));
  assert(detachedStaleRefreshState.theme === "graphite"
    && detachedStaleRefreshState.background === "rgb(23, 23, 23)"
    && detachedStaleRefreshState.instanceId === detachedTerminalInstanceId,
  `stale detached session refresh replaced a newer profile event: ${JSON.stringify({ detachedTerminalInstanceId, detachedStaleRefreshState })}`);
  await detachedPage.evaluate(() => {
    const index = window.__sessions.findIndex((session) => session.profile.id === "local-shell");
    const updated = structuredClone(window.__sessions[index]);
    updated.profile.terminal.theme = "portmate-light";
    window.__sessions[index] = updated;
    window.__emitTauriEvent("portmate-session-profile-updated", updated);
  });
  await detachedPage.locator('.detached-pane-terminal .terminal-host[data-terminal-theme="portmate-light"]').waitFor();
  const detachedHealth = await detachedPage.evaluate(() => {
    const index = window.__sessions.findIndex((session) => session.profile.id === "local-shell");
    const updated = structuredClone(window.__sessions[index]);
    updated.runtime.status = "reconnecting";
    updated.runtime.lastDisconnect = "invalid";
    updated.runtime.lastDisconnectReason = ` transport\n  stalled ${"x".repeat(300)} `;
    window.__sessions[index] = updated;
    window.__emitTauriEvent("portmate-session-profile-updated", updated);
    return updated;
  });
  await detachedPage.getByRole("button", { name: "断开会话", exact: true }).waitFor();
  const detachedRuntimeHealth = await detachedPage.evaluate(() => {
    const status = document.querySelector(".detached-pane-status > span");
    const indicator = document.querySelector(".detached-pane-toolbar .session-status-dot");
    return {
      text: status?.textContent ?? "",
      title: status?.getAttribute("title") ?? "",
      live: status?.getAttribute("aria-live") ?? "",
      indicatorTitle: indicator?.getAttribute("title") ?? "",
      indicatorLabel: indicator?.getAttribute("aria-label") ?? "",
      indicatorColor: indicator ? getComputedStyle(indicator).backgroundColor : "missing",
      indicatorWidth: indicator ? getComputedStyle(indicator).width : "missing",
      indicatorHeight: indicator ? getComputedStyle(indicator).height : "missing",
      connectButtons: document.querySelectorAll('button[aria-label="连接会话"]').length,
      disconnectButtons: document.querySelectorAll('button[aria-label="断开会话"]').length,
    };
  });
  assert(detachedRuntimeHealth.text === detachedRuntimeHealth.title
    && detachedRuntimeHealth.text === detachedRuntimeHealth.indicatorTitle
    && detachedRuntimeHealth.text === detachedRuntimeHealth.indicatorLabel
    && detachedRuntimeHealth.text.startsWith("正在重连 · 原因: transport stalled ")
    && !detachedRuntimeHealth.text.includes("Invalid Date")
    && !detachedRuntimeHealth.text.includes("\n")
    && detachedRuntimeHealth.live === "polite"
    && detachedRuntimeHealth.indicatorColor === "rgb(248, 113, 113)"
    && detachedRuntimeHealth.indicatorWidth === "7px"
    && detachedRuntimeHealth.indicatorHeight === "7px",
  `detached terminal did not normalize its runtime health: ${JSON.stringify({ detachedHealth, detachedRuntimeHealth })}`);
  assert(Array.from(detachedRuntimeHealth.text.split("原因: ")[1]).length === 256
    && detachedRuntimeHealth.text.endsWith("...")
    && detachedRuntimeHealth.connectButtons === 0
    && detachedRuntimeHealth.disconnectButtons === 1,
  `detached reconnect action or diagnostic boundary is wrong: ${JSON.stringify(detachedRuntimeHealth)}`);
  const detachedEmitCallsBefore = await detachedPage.evaluate(() => {
    window.__deferDetachedOwnerCommands = true;
    window.__pendingDetachedOwnerCommands = [];
    return window.__invokeCalls.length;
  });
  await detachedPage.getByRole("button", { name: "断开会话", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await detachedPage.waitForFunction(() => window.__pendingDetachedOwnerCommands.length === 1);
  const detachedOwnerBusyState = await detachedPage.locator(".detached-pane-root").evaluate((root) => ({
    action: root.getAttribute("data-owner-command-busy"),
    disconnectDisabled: document.querySelector('button[aria-label="断开会话"]')?.disabled,
    reattachDisabled: document.querySelector('button[aria-label="返回工作区"]')?.disabled,
    calls: window.__invokeCalls.filter((call) => call.command === "plugin:event|emit_to"
      && call.args.event === "portmate-detached-pane-command").length,
  }));
  assert(detachedOwnerBusyState.action === "disconnect"
    && detachedOwnerBusyState.disconnectDisabled
    && detachedOwnerBusyState.reattachDisabled
    && detachedOwnerBusyState.calls === 1,
  `detached owner command did not enter a single-flight state: ${JSON.stringify(detachedOwnerBusyState)}`);
  await detachedPage.evaluate(() => {
    window.__pendingDetachedOwnerCommands.shift().reject(new Error("simulated detached owner command failure"));
  });
  await detachedPage.locator(".detached-pane-status", { hasText: "simulated detached owner command failure" }).waitFor();
  await detachedPage.waitForFunction(() => !document.querySelector('button[aria-label="断开会话"]')?.disabled);
  await detachedPage.getByRole("button", { name: "断开会话", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await detachedPage.waitForFunction(() => window.__pendingDetachedOwnerCommands.length === 1);
  const detachedDisconnectCommands = await detachedPage.evaluate((start) => window.__invokeCalls.slice(start)
    .filter((call) => call.command === "plugin:event|emit_to"), detachedEmitCallsBefore);
  assert(detachedDisconnectCommands.length === 2
    && detachedDisconnectCommands.every((call) => call.args?.target?.label === "main"
      && call.args.event === "portmate-detached-pane-command"
      && call.args.payload?.action === "disconnect"
      && call.args.payload?.sessionId === "local-shell"),
  `detached reconnect control emitted duplicate or malformed commands: ${JSON.stringify(detachedDisconnectCommands)}`);
  await detachedPage.evaluate(() => {
    window.__pendingDetachedOwnerCommands.shift().resolve();
    window.__deferDetachedOwnerCommands = false;
  });
  await detachedPage.waitForFunction(() => (
    document.querySelector(".detached-pane-root")?.getAttribute("data-owner-command-busy") === ""
      && !document.querySelector('button[aria-label="断开会话"]')?.disabled
      && !document.querySelector(".detached-pane-status")?.textContent?.includes("simulated detached owner command failure")
  ));
  await detachedPage.screenshot({ path: `${screenshotPrefix}-detached-health.png`, fullPage: true });
  await detachedPage.evaluate(() => {
    const index = window.__sessions.findIndex((session) => session.profile.id === "local-shell");
    const updated = structuredClone(window.__sessions[index]);
    updated.runtime.status = "connected";
    window.__sessions[index] = updated;
    window.__emitTauriEvent("portmate-session-profile-updated", updated);
  });
  await detachedPage.getByRole("button", { name: "断开会话", exact: true }).waitFor();
  const detachedCatchupMarker = "DETACHED-POLL-CATCHUP";
  const detachedTailCallsBefore = await detachedPage.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "tail_log" && call.args.sessionId === "local-shell"
  )).length);
  await detachedPage.evaluate((marker) => {
    window.__events.push({
      id: "detached-poll-catchup",
      sessionId: "local-shell",
      paneId: "local-shell:main",
      ts: new Date().toISOString(),
      direction: "inbound",
      stream: "stdout",
      bytesRef: null,
      text: `${marker}\r\n`,
      annotations: {},
    });
  }, detachedCatchupMarker);
  await detachedPage.waitForFunction(({ previousCalls }) => window.__invokeCalls.filter((call) => (
    call.command === "tail_log" && call.args.sessionId === "local-shell"
  )).length > previousCalls, { previousCalls: detachedTailCallsBefore });
  await detachedPage.evaluate(() => window.dispatchEvent(new Event("portmate-terminal-search")));
  const detachedSearchInput = detachedPage.locator(".detached-pane-terminal .terminal-search-bar input");
  await detachedSearchInput.fill(detachedCatchupMarker);
  let detachedCatchupSearch = "0/0";
  for (let attempt = 0; attempt < 50 && detachedCatchupSearch === "0/0"; attempt += 1) {
    await detachedSearchInput.press("Enter");
    await detachedPage.waitForTimeout(50);
    detachedCatchupSearch = await detachedPage.locator(".detached-pane-terminal .terminal-search-status").textContent() ?? "0/0";
  }
  assert(detachedCatchupSearch !== "0/0", "detached terminal did not render its polled log catch-up event");
  await detachedPage.getByRole("button", { name: "关闭查找", exact: true }).click();
  const detachedInputStart = await detachedPage.evaluate(() => {
    window.__deferTerminalSends = true;
    window.__pendingTerminalSends = [];
    return window.__invokeCalls.filter((call) => call.command === "send_text").length;
  });
  const detachedTerminalInput = detachedPage.locator(".detached-pane-terminal .xterm-helper-textarea");
  await detachedTerminalInput.focus();
  await detachedPage.keyboard.press("a");
  await detachedPage.waitForFunction(() => window.__pendingTerminalSends.length === 1);
  await detachedPage.keyboard.press("b");
  await detachedPage.waitForTimeout(50);
  const detachedFirstInput = await detachedPage.evaluate((start) => window.__invokeCalls
    .filter((call) => call.command === "send_text").slice(start), detachedInputStart);
  assert(detachedFirstInput.length === 1 && detachedFirstInput[0].args.text === "a",
    `detached terminal dispatched overlapping input: ${JSON.stringify(detachedFirstInput)}`);
  await detachedPage.evaluate(() => window.__pendingTerminalSends.shift().resolve(null));
  await detachedPage.waitForFunction(() => window.__pendingTerminalSends.length === 1);
  const detachedOrderedInput = await detachedPage.evaluate((start) => window.__invokeCalls
    .filter((call) => call.command === "send_text").slice(start), detachedInputStart);
  assert(detachedOrderedInput.length === 2
    && detachedOrderedInput.map((call) => call.args.text).join("") === "ab",
  `detached terminal input order changed: ${JSON.stringify(detachedOrderedInput)}`);
  await detachedPage.evaluate(() => {
    window.__deferTerminalSends = false;
    window.__pendingTerminalSends.shift().resolve(null);
  });
  const detachedOneKeyCallsBefore = await detachedPage.evaluate(() => {
    window.__oneKeys = [{
      id: "detached-one-key",
      label: "Detached test",
      kind: "account",
      username: "tester",
      hasPassword: true,
      hasPassphrase: false,
      identity: null,
      sessionIds: ["local-shell"],
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    }];
    return window.__invokeCalls.filter((call) => call.command === "list_one_keys").length;
  });
  await detachedPage.waitForFunction((previousCalls) => (
    window.__invokeCalls.filter((call) => call.command === "list_one_keys").length > previousCalls
    && document.querySelector(".detached-pane-root")?.getAttribute("data-one-key-count") === "1"
  ), detachedOneKeyCallsBefore);
  const detachedStableCounts = await detachedPage.locator(".detached-pane-root").evaluate((root) => ({
    events: root.getAttribute("data-event-count"),
    oneKeys: root.getAttribute("data-one-key-count"),
  }));
  const detachedFailureCalls = await detachedPage.evaluate(() => {
    window.__failTailLogs = 1;
    window.__failOneKeyLists = 1;
    return {
      tail: window.__invokeCalls.filter((call) => call.command === "tail_log").length,
      oneKeys: window.__invokeCalls.filter((call) => call.command === "list_one_keys").length,
    };
  });
  await detachedPage.waitForFunction((previousCalls) => (
    window.__invokeCalls.filter((call) => call.command === "tail_log").length > previousCalls.tail
    && window.__invokeCalls.filter((call) => call.command === "list_one_keys").length > previousCalls.oneKeys
  ), detachedFailureCalls);
  await detachedPage.waitForTimeout(100);
  const detachedCountsAfterFailure = await detachedPage.locator(".detached-pane-root").evaluate((root) => ({
    events: root.getAttribute("data-event-count"),
    oneKeys: root.getAttribute("data-one-key-count"),
  }));
  assert(JSON.stringify(detachedCountsAfterFailure) === JSON.stringify(detachedStableCounts),
    `detached terminal cleared valid state after a polling failure: ${JSON.stringify({ detachedStableCounts, detachedCountsAfterFailure })}`);
  const detachedTerminalCanvas = detachedPage.locator(".detached-pane-terminal .terminal-canvas");
  const detachedSelectionTarget = {
    sessionId: await detachedTerminalCanvas.getAttribute("data-terminal-session-id"),
    viewId: await detachedTerminalCanvas.getAttribute("data-terminal-view-id"),
  };
  assert(detachedSelectionTarget.sessionId && detachedSelectionTarget.viewId,
    `detached terminal did not expose its interaction identity: ${JSON.stringify(detachedSelectionTarget)}`);
  const detachedSelectionResult = await detachedPage.evaluate(({ sessionId, viewId }) => new Promise((resolve) => {
    window.dispatchEvent(new CustomEvent("portmate-terminal-selection", {
      detail: { sessionId, viewId, action: "select-all", respond: resolve },
    }));
  }), detachedSelectionTarget);
  assert(detachedSelectionResult?.ok, `detached terminal could not establish a selection: ${JSON.stringify(detachedSelectionResult)}`);
  const detachedInputBeforeLock = await detachedPage.evaluate(() => window.__invokeCalls.filter((call) => call.command === "send_text").length);
  await detachedPage.evaluate(() => {
    const marker = { version: 1, reason: "manual", lockedAt: Date.now() };
    localStorage.setItem("portmate.screenLock.v1", JSON.stringify(marker));
    window.dispatchEvent(new StorageEvent("storage", {
      key: "portmate.screenLock.v1",
      newValue: JSON.stringify(marker),
      storageArea: localStorage,
    }));
  });
  const detachedLockOverlay = detachedPage.locator(".screen-lock-overlay");
  await detachedLockOverlay.waitFor();
  await detachedPage.waitForFunction(() => document.activeElement?.closest(".screen-lock-overlay"));
  const detachedLockBoundary = await detachedPage.evaluate(async () => {
    const terminalInput = document.querySelector(".detached-pane-terminal .xterm-helper-textarea");
    const terminalHost = document.querySelector(".detached-pane-terminal .terminal-host");
    const terminalCanvas = document.querySelector(".detached-pane-terminal .terminal-canvas");
    const rootInert = Boolean(terminalInput?.closest("[inert]"));
    terminalInput?.dispatchEvent(new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      key: "x",
      code: "KeyX",
    }));
    const inputEvent = new InputEvent("input", {
      bubbles: true,
      cancelable: true,
      data: "locked-detached-probe",
      inputType: "insertText",
    });
    terminalInput?.dispatchEvent(inputEvent);
    const pointerEvent = new PointerEvent("pointerdown", { bubbles: true, cancelable: true });
    terminalInput?.dispatchEvent(pointerEvent);
    const wheelEvent = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: 40 });
    terminalInput?.dispatchEvent(wheelEvent);
    const terminalTarget = {
      sessionId: terminalCanvas?.getAttribute("data-terminal-session-id"),
      viewId: terminalCanvas?.getAttribute("data-terminal-view-id"),
    };
    const commandResponses = {};
    for (const [type, detail] of [
      ["portmate-terminal-selection", { ...terminalTarget, action: "select-all" }],
      ["portmate-terminal-buffer-action", { ...terminalTarget, action: "clear-all" }],
      ["portmate-terminal-text-export", { ...terminalTarget, source: "buffer" }],
    ]) {
      window.dispatchEvent(new CustomEvent(type, {
        detail: { ...detail, respond: (response) => { commandResponses[type] = response; } },
      }));
    }
    for (const type of [
      "portmate-terminal-search",
      "portmate-terminal-goto-line",
      "portmate-terminal-free-input",
    ]) window.dispatchEvent(new Event(type));
    terminalInput?.focus();
    await new Promise((resolve) => setTimeout(resolve, 40));
    return {
      rootInert,
      focusInsideLock: Boolean(document.activeElement?.closest(".screen-lock-overlay")),
      inputBlocked: inputEvent.defaultPrevented,
      pointerBlocked: pointerEvent.defaultPrevented,
      wheelBlocked: wheelEvent.defaultPrevented,
      hasSelection: terminalHost?.getAttribute("data-terminal-has-selection"),
      commandResponses,
      terminalToolsOpened: Boolean(document.querySelector(
        ".detached-pane-terminal .terminal-search-bar, .detached-pane-terminal .terminal-goto-line, .detached-pane-terminal .terminal-free-input",
      )),
    };
  });
  const detachedInputAfterLock = await detachedPage.evaluate(() => window.__invokeCalls.filter((call) => call.command === "send_text").length);
  assert(detachedLockBoundary.rootInert
    && detachedLockBoundary.focusInsideLock
    && detachedLockBoundary.inputBlocked
    && detachedLockBoundary.pointerBlocked
    && detachedLockBoundary.wheelBlocked
    && detachedLockBoundary.hasSelection !== "true"
    && Object.keys(detachedLockBoundary.commandResponses).length === 3
    && Object.values(detachedLockBoundary.commandResponses).every((response) => response?.ok === false && response.error.includes("顶层对话框"))
    && !detachedLockBoundary.terminalToolsOpened
    && detachedInputAfterLock === detachedInputBeforeLock,
  `interaction crossed the detached-window lock layer: ${JSON.stringify({
    boundary: detachedLockBoundary,
    before: detachedInputBeforeLock,
    after: detachedInputAfterLock,
  })}`);
  await detachedPage.evaluate(() => localStorage.removeItem("portmate.screenLock.v1"));
  await detachedLockOverlay.waitFor({ state: "detached" });
  const detachedCloseCallsBefore = await detachedPage.evaluate(() => {
    window.__detachedReattachResult = { ok: false, error: "simulated owner reattach rejection" };
    return window.__invokeCalls.filter((call) => call.command === "plugin:window|close").length;
  });
  await detachedPage.getByRole("button", { name: "返回工作区", exact: true }).click();
  await detachedPage.locator(".detached-pane-status", { hasText: "simulated owner reattach rejection" }).waitFor();
  const detachedRejectedReturnState = await detachedPage.evaluate((closeCallsBefore) => ({
    busy: document.querySelector(".detached-pane-root")?.getAttribute("data-owner-command-busy"),
    closeCalls: window.__invokeCalls.filter((call) => call.command === "plugin:window|close").length - closeCallsBefore,
    resultListeners: window.__invokeCalls.filter((call) => call.command === "plugin:event|listen"
      && call.args.event === "portmate-detached-pane-result").length,
  }), detachedCloseCallsBefore);
  assert(detachedRejectedReturnState.busy === ""
    && detachedRejectedReturnState.closeCalls === 0
    && detachedRejectedReturnState.resultListeners >= 1
    && !detachedPage.isClosed(),
  `detached window closed without an accepted owner acknowledgement: ${JSON.stringify(detachedRejectedReturnState)}`);
  await detachedPage.evaluate(() => {
    window.__detachedReattachResult = { ok: true, error: "" };
  });
  await detachedPage.getByRole("button", { name: "返回工作区", exact: true }).click();
  await detachedPage.waitForFunction((closeCallsBefore) => (
    window.__invokeCalls.filter((call) => call.command === "plugin:window|close").length > closeCallsBefore
  ), detachedCloseCallsBefore);
  await detachedPage.screenshot({ path: `${screenshotPrefix}-detached-theme.png`, fullPage: true });
  await detachedPage.close();

  const serialAnalyzerUrl = new URL(appUrl);
  serialAnalyzerUrl.searchParams.set("serialAnalyzer", "1");
  serialAnalyzerUrl.searchParams.set("windowId", "serial-refresh-gate");
  serialAnalyzerUrl.searchParams.set("sessionId", "bench-uart");
  const serialAnalyzerPage = await context.newPage();
  const serialAnalyzerPageErrors = [];
  serialAnalyzerPage.on("pageerror", (error) => serialAnalyzerPageErrors.push(error.message));
  await serialAnalyzerPage.goto(serialAnalyzerUrl.toString());
  await serialAnalyzerPage.locator(".serial-analyzer-root").waitFor({ timeout: 30_000 }).catch(async (error) => {
    throw new Error(`serial analyzer did not open at ${serialAnalyzerPage.url()}: ${JSON.stringify({
      pageErrors: serialAnalyzerPageErrors,
      body: (await serialAnalyzerPage.locator("body").textContent())?.slice(0, 500),
      cause: error.message,
    })}`);
  });
  await serialAnalyzerPage.evaluate(() => {
    const serial = window.__sessions.find((item) => item.profile.id === "bench-uart");
    serial.runtime.status = "disconnected";
    serial.runtime.connectedSince = null;
  });
  await serialAnalyzerPage.locator(".serial-analyzer-connection", { hasText: "已断开" }).waitFor();
  await serialAnalyzerPage.evaluate(() => {
    const serial = window.__sessions.find((item) => item.profile.id === "bench-uart");
    serial.runtime.status = "connected";
    serial.runtime.connectedSince = new Date().toISOString();
  });
  await serialAnalyzerPage.locator(".serial-analyzer-connection", { hasText: "已连接" }).waitFor();
  const serialAnalyzerHealth = await serialAnalyzerPage.evaluate(() => {
    const status = document.querySelector(".serial-analyzer-connection");
    const disconnect = document.querySelector(".serial-analyzer-last-disconnect");
    return {
      status: status?.textContent ?? "",
      title: status?.getAttribute("title") ?? "",
      description: status?.getAttribute("aria-description") ?? "",
      statusWidth: status?.getBoundingClientRect().width ?? 0,
      disconnect: disconnect?.textContent ?? "",
      disconnectTitle: disconnect?.getAttribute("title") ?? "",
    };
  });
  assert(serialAnalyzerHealth.status === "已连接",
    `serial analyzer did not refresh a connected backend session after a disconnect: ${JSON.stringify(serialAnalyzerHealth)}`);
  assert(serialAnalyzerHealth.title === serialAnalyzerHealth.description
    && serialAnalyzerHealth.title.startsWith("已连接 · 原因: serial cable ")
    && !serialAnalyzerHealth.title.includes("Invalid Date")
    && !serialAnalyzerHealth.title.includes("\n"),
  `serial analyzer did not normalize its accessible health diagnostic: ${JSON.stringify(serialAnalyzerHealth)}`);
  assert(serialAnalyzerHealth.disconnect === serialAnalyzerHealth.disconnectTitle
    && serialAnalyzerHealth.disconnect.startsWith("原因: serial cable ")
    && serialAnalyzerHealth.disconnect.endsWith("...")
    && Array.from(serialAnalyzerHealth.disconnect.slice("原因: ".length)).length === 256,
  `serial analyzer did not bound its disconnect diagnostic: ${JSON.stringify(serialAnalyzerHealth)}`);
  assert(serialAnalyzerHealth.statusWidth === 60,
    `serial analyzer runtime status width is unstable: ${JSON.stringify(serialAnalyzerHealth)}`);
  await serialAnalyzerPage.screenshot({ path: `${screenshotPrefix}-serial-analyzer.png`, fullPage: true });
  await serialAnalyzerPage.evaluate(() => {
    const marker = { version: 1, reason: "manual", lockedAt: Date.now() };
    localStorage.setItem("portmate.screenLock.v1", JSON.stringify(marker));
    window.dispatchEvent(new StorageEvent("storage", {
      key: "portmate.screenLock.v1",
      newValue: JSON.stringify(marker),
      storageArea: localStorage,
    }));
  });
  const serialAnalyzerLockOverlay = serialAnalyzerPage.locator(".screen-lock-overlay");
  await serialAnalyzerLockOverlay.waitFor();
  await serialAnalyzerPage.waitForFunction(() => document.activeElement?.closest(".screen-lock-overlay"));
  const serialAnalyzerLockBoundary = await serialAnalyzerPage.evaluate(async () => {
    const parserButton = document.querySelector('.serial-analyzer-segmented[aria-label="帧解析方式"] button');
    const filterInput = document.querySelector('[aria-label="筛选分析帧"]');
    const statusText = document.querySelector(".serial-analyzer-status-strip span");
    const clickEvent = new MouseEvent("click", { bubbles: true, cancelable: true });
    parserButton?.dispatchEvent(clickEvent);
    const inputEvent = new InputEvent("input", {
      bubbles: true,
      cancelable: true,
      data: "locked-analyzer-probe",
      inputType: "insertText",
    });
    filterInput?.dispatchEvent(inputEvent);
    const pointerEvent = new PointerEvent("pointerdown", { bubbles: true, cancelable: true });
    filterInput?.dispatchEvent(pointerEvent);
    const wheelEvent = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: 40 });
    filterInput?.dispatchEvent(wheelEvent);
    const selection = document.getSelection();
    selection?.removeAllRanges();
    if (statusText?.firstChild && selection) {
      const range = document.createRange();
      range.selectNodeContents(statusText.firstChild);
      selection.addRange(range);
      document.dispatchEvent(new Event("selectionchange"));
    }
    filterInput?.focus();
    await new Promise((resolve) => setTimeout(resolve, 40));
    return {
      contentInert: Boolean(filterInput?.closest("[inert]")),
      clickBlocked: clickEvent.defaultPrevented,
      inputBlocked: inputEvent.defaultPrevented,
      pointerBlocked: pointerEvent.defaultPrevented,
      wheelBlocked: wheelEvent.defaultPrevented,
      selection: selection?.toString() ?? "",
      focusInsideLock: Boolean(document.activeElement?.closest(".screen-lock-overlay")),
    };
  });
  assert(serialAnalyzerLockBoundary.contentInert
    && serialAnalyzerLockBoundary.clickBlocked
    && serialAnalyzerLockBoundary.inputBlocked
    && serialAnalyzerLockBoundary.pointerBlocked
    && serialAnalyzerLockBoundary.wheelBlocked
    && serialAnalyzerLockBoundary.selection === ""
    && serialAnalyzerLockBoundary.focusInsideLock,
  `interaction crossed the serial-analyzer lock layer: ${JSON.stringify(serialAnalyzerLockBoundary)}`);
  await serialAnalyzerPage.evaluate(() => localStorage.removeItem("portmate.screenLock.v1"));
  await serialAnalyzerLockOverlay.waitFor({ state: "detached" });
  await serialAnalyzerPage.getByLabel("筛选分析帧", { exact: true }).fill("post-lock-probe");
  assert(await serialAnalyzerPage.getByLabel("筛选分析帧", { exact: true }).inputValue() === "post-lock-probe",
    "serial analyzer remained inert after the shared screen lock closed");
  await serialAnalyzerPage.getByLabel("筛选分析帧", { exact: true }).fill("");
  await serialAnalyzerPage.evaluate(() => {
    window.__serialCaptureFrames = [{
      id: "rx-after-lock",
      ts: new Date().toISOString(),
      direction: "inbound",
      bytes: [0x41, 0x42, 0x43],
      originalLength: 3,
      truncated: false,
    }];
  });
  await serialAnalyzerPage.locator(".serial-analyzer-status-strip", { hasText: "捕获 1" }).waitFor();
  const serialClearButton = serialAnalyzerPage.getByRole("button", { name: "清空串口捕获", exact: true });
  const serialRefreshButton = serialAnalyzerPage.getByRole("button", { name: "刷新串口捕获", exact: true });
  const serialExportButton = serialAnalyzerPage.getByRole("button", { name: "导出筛选串口帧", exact: true });
  await serialAnalyzerPage.evaluate(() => {
    window.__deferSerialCaptureOperations = true;
    window.__serialAnalyzerOriginalConfirm = window.confirm;
    window.confirm = () => true;
    const button = document.querySelector('[aria-label="清空串口捕获"]');
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
  await serialAnalyzerPage.waitForFunction(() => window.__pendingSerialCaptureOperations.length === 1);
  const serialListCallsDuringClear = await serialAnalyzerPage.evaluate(() => (
    window.__invokeCalls.filter((call) => call.command === "list_serial_capture").length
  ));
  await serialAnalyzerPage.waitForTimeout(900);
  const serialClearBusyState = await serialAnalyzerPage.evaluate(() => ({
    pending: window.__pendingSerialCaptureOperations.length,
    clearCalls: window.__invokeCalls.filter((call) => call.command === "clear_serial_capture").length,
    listCalls: window.__invokeCalls.filter((call) => call.command === "list_serial_capture").length,
    toolbarBusy: document.querySelector(".serial-analyzer-toolbar")?.getAttribute("aria-busy"),
  }));
  assert(serialClearBusyState.pending === 1
    && serialClearBusyState.clearCalls === 1
    && serialClearBusyState.listCalls === serialListCallsDuringClear
    && serialClearBusyState.toolbarBusy === "true"
    && await serialClearButton.isDisabled()
    && await serialRefreshButton.isDisabled()
    && await serialExportButton.isDisabled(),
  `serial analyzer clear overlapped polling or duplicated: ${JSON.stringify(serialClearBusyState)}`);
  await serialAnalyzerPage.evaluate(() => {
    window.__pendingSerialCaptureOperations.shift().resolve();
  });
  await serialAnalyzerPage.locator(".serial-analyzer-footer", { hasText: "捕获已清空" }).waitFor();
  await serialAnalyzerPage.evaluate(() => {
    window.__serialCaptureFrames = [{
      id: "tx-after-clear",
      ts: new Date().toISOString(),
      direction: "outbound",
      bytes: [0x44, 0x45, 0x46],
      originalLength: 3,
      truncated: false,
    }];
  });
  await serialAnalyzerPage.locator(".serial-analyzer-status-strip", { hasText: "捕获 1" }).waitFor();
  await serialAnalyzerPage.waitForFunction((previousCalls) => (
    window.__invokeCalls.filter((call) => call.command === "list_serial_capture").length > previousCalls
  ), serialListCallsDuringClear);
  assert(await serialAnalyzerPage.locator(".serial-analyzer-toolbar").getAttribute("aria-busy") === "false"
    && await serialClearButton.isEnabled()
    && await serialRefreshButton.isEnabled()
    && await serialExportButton.isEnabled(),
  "serial analyzer did not resume capture actions after clearing");
  await serialAnalyzerPage.evaluate(() => {
    const button = document.querySelector('[aria-label="导出筛选串口帧"]');
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
  await serialAnalyzerPage.waitForFunction(() => window.__pendingSerialCaptureOperations.length === 1);
  const serialExportBusyState = await serialAnalyzerPage.evaluate(() => ({
    pending: window.__pendingSerialCaptureOperations.length,
    exportCalls: window.__invokeCalls.filter((call) => call.command === "export_serial_capture").length,
    toolbarBusy: document.querySelector(".serial-analyzer-toolbar")?.getAttribute("aria-busy"),
  }));
  assert(serialExportBusyState.pending === 1
    && serialExportBusyState.exportCalls === 1
    && serialExportBusyState.toolbarBusy === "true"
    && await serialClearButton.isDisabled()
    && await serialRefreshButton.isDisabled()
    && await serialExportButton.isDisabled(),
  `serial analyzer export duplicated or left conflicting actions enabled: ${JSON.stringify(serialExportBusyState)}`);
  await serialAnalyzerPage.evaluate(() => {
    window.__pendingSerialCaptureOperations.shift().resolve();
    window.__deferSerialCaptureOperations = false;
    window.confirm = window.__serialAnalyzerOriginalConfirm;
  });
  await serialAnalyzerPage.locator(".serial-analyzer-footer", { hasText: "/tmp/portmate-serial-capture.txt" }).waitFor();
  await serialAnalyzerPage.evaluate(() => { window.__deferSessionLists = true; });
  await serialAnalyzerPage.waitForFunction(() => window.__pendingSessionLists.length === 1);
  await serialAnalyzerPage.waitForTimeout(3_200);
  assert(await serialAnalyzerPage.evaluate(() => window.__pendingSessionLists.length) === 1,
    "serial analyzer session polling started overlapping requests");
  await serialAnalyzerPage.getByRole("button", { name: "刷新串口捕获", exact: true }).click();
  assert(await serialAnalyzerPage.evaluate(() => window.__pendingSessionLists.length) === 1,
    "serial analyzer manual refresh bypassed the session request gate");
  await serialAnalyzerPage.evaluate(() => {
    for (const pending of window.__pendingSessionLists) pending.resolve(pending.result);
    window.__pendingSessionLists = [];
    window.__deferSessionLists = false;
  });
  assert(serialAnalyzerPageErrors.length === 0,
    `serial analyzer refresh gate raised browser errors: ${JSON.stringify(serialAnalyzerPageErrors)}`);
  await serialAnalyzerPage.close();

  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.getByRole("button", { name: "自定义脚本", exact: true }).click();
  const customScriptDialog = page.locator(".custom-script-dialog");
  await customScriptDialog.waitFor();
  const scriptBody = customScriptDialog.getByRole("textbox", { name: "脚本正文", exact: true });
  assert(await scriptBody.inputValue() === "systemctl status portmate"
    && await customScriptDialog.locator(".custom-script-list i", { hasText: "MCP" }).count() === 1,
  "custom script manager did not load the persisted MCP-enabled script");
  await scriptBody.fill("systemctl status portmate\nwhoami");
  assert(await customScriptDialog.getByRole("button", { name: "运行自定义脚本", exact: true }).isDisabled(),
    "custom script manager can run a stale saved body while the editor has unsaved changes");
  assert(await customScriptDialog.getByRole("button", { name: "刷新自定义脚本", exact: true }).isDisabled(),
    "custom script refresh can silently discard unsaved editor changes");
  await page.evaluate(() => {
    window.__customScriptDiscardPrompts = [];
    window.__originalCustomScriptConfirm = window.confirm;
    window.confirm = (message) => {
      window.__customScriptDiscardPrompts.push(String(message));
      return false;
    };
  });
  await customScriptDialog.getByRole("button", { name: "关闭自定义脚本", exact: true }).click();
  await customScriptDialog.getByRole("button", { name: "添加自定义脚本", exact: true }).click();
  await customScriptDialog.getByRole("option", { name: /Inspect service/ }).click();
  const retainedCustomScriptDraft = await page.evaluate(() => ({
    prompts: window.__customScriptDiscardPrompts,
    dialogVisible: Boolean(document.querySelector(".custom-script-dialog")),
    body: document.querySelector('[aria-label="脚本正文"]')?.value,
  }));
  assert(retainedCustomScriptDraft.dialogVisible
    && retainedCustomScriptDraft.body === "systemctl status portmate\nwhoami"
    && retainedCustomScriptDraft.prompts.length === 3
    && retainedCustomScriptDraft.prompts.some((prompt) => prompt.includes("关闭窗口"))
    && retainedCustomScriptDraft.prompts.some((prompt) => prompt.includes("新建脚本"))
    && retainedCustomScriptDraft.prompts.some((prompt) => prompt.includes("切换脚本")),
  `custom script draft was discarded without confirmation: ${JSON.stringify(retainedCustomScriptDraft)}`);
  await page.evaluate(() => {
    window.confirm = window.__originalCustomScriptConfirm;
  });
  await customScriptDialog.getByRole("button", { name: "保存自定义脚本", exact: true }).click();
  await page.waitForFunction(() => window.__invokeCalls.filter((call) => call.command === "save_custom_script").length === 1);
  assert(!await customScriptDialog.getByRole("button", { name: "运行自定义脚本", exact: true }).isDisabled(),
    "saved custom script did not become runnable");
  assert(!await customScriptDialog.getByRole("button", { name: "刷新自定义脚本", exact: true }).isDisabled(),
    "custom script refresh remained disabled after saving editor changes");

  await customScriptDialog.getByRole("button", { name: "添加自定义脚本", exact: true }).click();
  await page.evaluate(() => { window.__injectConcurrentCustomScriptBeforeSave = true; });
  await customScriptDialog.getByRole("textbox", { name: "脚本名称", exact: true }).fill("Collect diagnostics");
  await customScriptDialog.getByRole("textbox", { name: "脚本说明", exact: true }).fill("Capture runtime state");
  await customScriptDialog.getByRole("textbox", { name: "脚本正文", exact: true }).fill("uptime\ndf -h");
  await customScriptDialog.getByRole("checkbox", { name: "开放给 MCP", exact: true }).check();
  await customScriptDialog.getByRole("button", { name: "保存自定义脚本", exact: true }).click();
  await page.waitForFunction(() => window.__customScripts.length === 3);
  assert(await customScriptDialog.getByRole("textbox", { name: "脚本名称", exact: true }).inputValue() === "Collect diagnostics",
    "a concurrently created script replaced the script selected by the save response");
  const customScriptRunButton = customScriptDialog.getByRole("button", { name: "运行自定义脚本", exact: true });
  const customScriptOperationBaseline = await page.evaluate(() => ({
    runCalls: window.__invokeCalls.filter((item) => item.command === "run_custom_script").length,
    events: window.__events.length,
  }));
  await page.evaluate(() => {
    window.__deferCustomScriptRuns = true;
    const runButton = document.querySelector('[aria-label="运行自定义脚本"]');
    const closeButton = document.querySelector('[aria-label="关闭自定义脚本"]');
    runButton.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    runButton.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    closeButton.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
  await page.waitForFunction(() => window.__pendingCustomScriptRuns.length === 1);
  const customScriptOperationState = await page.evaluate((baseline) => ({
    runCalls: window.__invokeCalls.filter((item) => item.command === "run_custom_script").length - baseline.runCalls,
    events: window.__events.length - baseline.events,
    pending: window.__pendingCustomScriptRuns.length,
    dialogVisible: Boolean(document.querySelector(".custom-script-dialog")),
  }), customScriptOperationBaseline);
  assert(customScriptOperationState.runCalls === 1
    && customScriptOperationState.events === 1
    && customScriptOperationState.pending === 1
    && customScriptOperationState.dialogVisible
    && await customScriptRunButton.isDisabled()
    && await customScriptDialog.getByRole("button", { name: "保存自定义脚本", exact: true }).isDisabled()
    && await customScriptDialog.getByRole("button", { name: "删除自定义脚本", exact: true }).isDisabled()
    && await customScriptDialog.getByRole("button", { name: "刷新自定义脚本", exact: true }).isDisabled()
    && await customScriptDialog.getByRole("button", { name: "关闭自定义脚本", exact: true }).isDisabled(),
  `custom script operation was duplicated or did not lock conflicting controls: ${JSON.stringify(customScriptOperationState)}`);
  await page.evaluate(() => {
    const pending = window.__pendingCustomScriptRuns.shift();
    pending.resolve(pending.result);
    window.__deferCustomScriptRuns = false;
  });
  const scriptNotice = page.locator(".notice-dialog", { hasText: "Collect diagnostics" });
  await scriptNotice.waitFor();
  const scriptInvocation = await page.evaluate(() => {
    const call = window.__invokeCalls.filter((item) => item.command === "run_custom_script").at(-1);
    const script = window.__customScripts.find((item) => item.id === call?.args.request.scriptId);
    return {
      call,
      targetAllowed: Boolean(script && (script.allowAllSessions || script.allowedSessionIds.includes(call.args.request.sessionId))),
    };
  });
  assert(scriptInvocation.targetAllowed
    && typeof scriptInvocation.call?.args.request.scriptId === "string"
    && scriptInvocation.call.args.request.expectedUpdatedAt
      === (await page.evaluate((scriptId) => window.__customScripts.find((item) => item.id === scriptId)?.updatedAt,
        scriptInvocation.call.args.request.scriptId))
    && !Object.hasOwn(scriptInvocation.call.args.request, "content"),
  `custom script execution did not select a saved script safely: ${JSON.stringify(scriptInvocation)}`);
  await scriptNotice.getByRole("button", { name: "确定", exact: true }).click();
  const customScriptBounds = await customScriptDialog.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    const editor = dialog.querySelector(".custom-script-editor")?.getBoundingClientRect();
    const actions = dialog.querySelector(".custom-script-actions")?.getBoundingClientRect();
    return {
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      scrollWidth: dialog.scrollWidth,
      width: rect.width,
      actionsInsideEditor: Boolean(editor && actions && actions.top >= editor.top && actions.bottom <= editor.bottom),
    };
  });
  assert(customScriptBounds.left >= 0 && customScriptBounds.right <= 1440
    && customScriptBounds.top >= 0 && customScriptBounds.bottom <= 900
    && customScriptBounds.scrollWidth <= customScriptBounds.width
    && customScriptBounds.actionsInsideEditor,
  `custom script desktop workspace overflows or clips actions: ${JSON.stringify(customScriptBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-custom-scripts.png`, fullPage: true });
  const staleScriptRun = await page.evaluate((scriptId) => {
    const script = window.__customScripts.find((item) => item.id === scriptId);
    if (!script) throw new Error("custom script fixture disappeared");
    script.content = "echo changed-in-another-window";
    script.updatedAt = new Date(Date.now() + 60_000).toISOString();
    return { scriptId, eventCount: window.__events.length };
  }, scriptInvocation.call.args.request.scriptId);
  await customScriptDialog.getByRole("button", { name: "运行自定义脚本", exact: true }).click();
  const staleRunError = customScriptDialog.getByRole("alert");
  await staleRunError.waitFor();
  assert((await staleRunError.textContent())?.includes("changed in another window")
    && await page.evaluate(() => window.__events.length) === staleScriptRun.eventCount,
  "desktop custom script execution accepted a body changed in another window");
  await customScriptDialog.getByRole("button", { name: "刷新自定义脚本", exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[aria-label="脚本正文"]')?.value
    === "echo changed-in-another-window");
  assert(await customScriptDialog.getByRole("textbox", { name: "脚本名称", exact: true }).inputValue()
      === "Collect diagnostics"
    && await customScriptDialog.getByRole("alert").count() === 0,
  "custom script refresh did not preserve the selected script or clear its stale conflict error");
  await page.evaluate(() => {
    window.__customScriptDeletePrompts = [];
    window.__originalCustomScriptConfirm = window.confirm;
    window.confirm = (message) => {
      window.__customScriptDeletePrompts.push(String(message));
      return true;
    };
  });
  await customScriptDialog.getByRole("button", { name: "删除自定义脚本", exact: true }).click();
  await page.waitForFunction(() => window.__customScripts.length === 2);
  await customScriptDialog.getByRole("option", { name: /Concurrent window script/ }).click();
  await customScriptDialog.getByRole("button", { name: "删除自定义脚本", exact: true }).click();
  await page.waitForFunction(() => window.__customScripts.length === 1);
  const customScriptDeletePrompts = await page.evaluate(() => {
    window.confirm = window.__originalCustomScriptConfirm;
    return window.__customScriptDeletePrompts;
  });
  assert(customScriptDeletePrompts.length === 2
    && customScriptDeletePrompts[0].includes("Collect diagnostics")
    && customScriptDeletePrompts[1].includes("Concurrent window script"),
  `custom script deletion confirmation omitted its target: ${JSON.stringify(customScriptDeletePrompts)}`);
  await customScriptDialog.getByRole("button", { name: "关闭自定义脚本", exact: true }).click();
  await customScriptDialog.waitFor({ state: "detached" });

  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.getByRole("button", { name: "日志管理", exact: true }).click();
  const logManager = page.locator(".log-manager-dialog");
  await logManager.waitFor();
  await logManager.getByRole("button", { name: /logs\/a\.txt/ }).waitFor();
  await page.evaluate(() => { window.__deferLogPreviews = true; });
  await logManager.getByRole("button", { name: /logs\/a\.txt/ }).click();
  await logManager.getByRole("button", { name: /logs\/b\.jsonl/ }).click();
  await page.waitForFunction(() => window.__pendingLogPreviews.length === 2);
  await page.evaluate(() => {
    const pending = window.__pendingLogPreviews[1];
    pending.resolve({ path: pending.args.path, content: "newer preview", encoding: "utf8", bytesRead: 13, truncated: false });
  });
  await logManager.locator(".log-preview > header strong", { hasText: "logs/b.jsonl" }).waitFor();
  await page.evaluate(() => {
    const pending = window.__pendingLogPreviews[0];
    pending.resolve({ path: pending.args.path, content: "stale preview", encoding: "utf8", bytesRead: 13, truncated: false });
    window.__pendingLogPreviews = [];
    window.__deferLogPreviews = false;
  });
  await page.waitForTimeout(100);
  assert(await logManager.locator(".log-preview > header strong").textContent() === "logs/b.jsonl",
    "a stale log preview response replaced the latest selected shard");

  await logManager.getByRole("checkbox", { name: "选择 logs/a.txt", exact: true }).check();
  await page.evaluate(() => {
    window.__deferLogShardLists = true;
    window.__pendingLogShardLists = [];
    window.__deferLogPreviews = true;
    window.__pendingLogPreviews = [];
  });
  await logManager.getByRole("button", { name: "刷新日志分片", exact: true }).click();
  await logManager.getByRole("button", { name: /logs\/b\.jsonl/ }).click();
  await page.waitForFunction(() => window.__pendingLogShardLists.length === 1
    && window.__pendingLogPreviews.length === 1);
  await page.evaluate(() => {
    const pending = window.__pendingLogPreviews.shift();
    pending.resolve({ path: pending.args.path, content: "concurrent preview", encoding: "utf8", bytesRead: 18, truncated: false });
    window.__deferLogPreviews = false;
  });
  await logManager.locator(".log-preview", { hasText: "concurrent preview" }).waitFor();
  assert(await logManager.getByRole("button", { name: "刷新日志分片", exact: true }).isDisabled()
    && await logManager.getByRole("button", { name: "归档选中分片", exact: true }).isDisabled()
    && await logManager.getByRole("button", { name: "删除选中分片", exact: true }).isDisabled()
    && await logManager.getByRole("button", { name: "导出会话包", exact: true }).isDisabled(),
  "a completed log preview released controls while the shard refresh was still pending");
  await page.evaluate(() => {
    window.__pendingLogShardLists.shift().resolve();
    window.__deferLogShardLists = false;
  });
  await page.waitForFunction(() => !document.querySelector('button[aria-label="刷新日志分片"]')?.disabled);
  assert(await logManager.getByRole("button", { name: "归档选中分片", exact: true }).isEnabled()
    && await logManager.getByRole("button", { name: "删除选中分片", exact: true }).isEnabled()
    && await logManager.getByRole("button", { name: "导出会话包", exact: true }).isEnabled(),
  "log controls did not recover after both concurrent reads completed");
  await page.evaluate(() => {
    window.__deferLogMutations = true;
    window.__pendingLogMutations = [];
  });
  const archiveLogsButton = logManager.getByRole("button", { name: "归档选中分片", exact: true });
  await archiveLogsButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingLogMutations.length === 1);
  const pendingArchiveRequests = await page.evaluate(() => window.__pendingLogMutations.map((pending) => ({
    command: pending.command,
    paths: pending.args.request?.paths,
  })));
  assert(JSON.stringify(pendingArchiveRequests) === JSON.stringify([
    { command: "archive_log_shards", paths: ["logs/a.txt"] },
  ]), `log archive was submitted more than once: ${JSON.stringify(pendingArchiveRequests)}`);
  assert(await archiveLogsButton.isDisabled()
    && await logManager.getByRole("button", { name: "删除选中分片", exact: true }).isDisabled()
    && await logManager.getByRole("button", { name: "导出中", exact: true }).count() === 0
    && await logManager.getByRole("button", { name: "导出会话包", exact: true }).isDisabled(),
  "a pending log archive did not lock conflicting write operations");
  await logManager.getByRole("button", { name: "关闭", exact: true }).last().click();
  await logManager.waitFor({ state: "detached" });
  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.getByRole("button", { name: "日志管理", exact: true }).click();
  await logManager.waitFor();
  await logManager.getByRole("button", { name: /logs\/a\.txt/ }).waitFor();
  await page.evaluate(() => {
    window.__deferLogMutations = false;
    window.__pendingLogMutations.shift().resolve();
    window.__pendingLogMutations = [];
  });
  await page.waitForTimeout(100);
  assert(await logManager.count() === 1
    && await logManager.locator(".log-archive-result").count() === 0
    && await page.locator(".notice-dialog").count() === 0,
  "a late log archive response mutated or obscured the replacement dialog");

  await logManager.getByRole("checkbox", { name: "选择 logs/a.txt", exact: true }).check();
  await page.evaluate(() => {
    window.__deferLogMutations = true;
    window.__pendingLogMutations = [];
  });
  const exportLogBundleButton = logManager.getByRole("button", { name: "导出会话包", exact: true });
  await exportLogBundleButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingLogMutations.length === 1);
  assert(await logManager.getByRole("button", { name: "导出中", exact: true }).isDisabled()
    && await logManager.getByRole("button", { name: "归档选中分片", exact: true }).isDisabled()
    && await logManager.getByRole("button", { name: "删除选中分片", exact: true }).isDisabled(),
  "a pending session bundle did not lock conflicting log writes");
  const pendingBundleRequests = await page.evaluate(() => window.__pendingLogMutations.map((pending) => pending.command));
  assert(JSON.stringify(pendingBundleRequests) === JSON.stringify(["export_session_bundle_archive"]),
    `session bundle export was submitted more than once: ${JSON.stringify(pendingBundleRequests)}`);
  await page.evaluate(() => {
    window.__deferLogMutations = false;
    window.__pendingLogMutations.shift().resolve();
    window.__pendingLogMutations = [];
  });
  await logManager.locator(".log-bundle-result", { hasText: "/tmp/portmate-session.tar.gz" }).waitFor();
  const bundleNotice = page.locator(".notice-dialog", { hasText: "会话包已导出" });
  await bundleNotice.waitFor();
  await bundleNotice.getByRole("button", { name: "确定", exact: true }).click();

  await page.evaluate(() => {
    window.__deferLogMutations = true;
    window.__pendingLogMutations = [];
    window.__logDeletePrompts = [];
    window.__originalLogConfirm = window.confirm;
    window.confirm = (message) => {
      window.__logDeletePrompts.push(String(message));
      return true;
    };
  });
  const deleteLogsButton = logManager.getByRole("button", { name: "删除选中分片", exact: true });
  await deleteLogsButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingLogMutations.length === 1);
  const pendingDeleteRequests = await page.evaluate(() => ({
    commands: window.__pendingLogMutations.map((pending) => pending.command),
    prompts: window.__logDeletePrompts,
  }));
  assert(JSON.stringify(pendingDeleteRequests.commands) === JSON.stringify(["delete_log_shards"])
    && pendingDeleteRequests.prompts.length === 1
    && pendingDeleteRequests.prompts[0].includes("1 个日志分片"),
  `log deletion was confirmed or submitted more than once: ${JSON.stringify(pendingDeleteRequests)}`);
  assert(await deleteLogsButton.isDisabled()
    && await logManager.getByRole("button", { name: "归档选中分片", exact: true }).isDisabled()
    && await logManager.getByRole("button", { name: "导出会话包", exact: true }).isDisabled(),
  "a pending log deletion did not lock conflicting write operations");
  await page.evaluate(() => {
    window.__deferLogMutations = false;
    window.__pendingLogMutations.shift().resolve();
    window.__pendingLogMutations = [];
    window.confirm = window.__originalLogConfirm;
  });
  await logManager.getByRole("button", { name: /logs\/a\.txt/ }).waitFor({ state: "detached" });
  assert(await logManager.getByRole("button", { name: /logs\/a\.txt/ }).count() === 0,
    "deleted log shard remained visible after the authoritative refresh");
  const deleteLogsNotice = page.locator(".notice-dialog", { hasText: "已删除 1 个分片" });
  await deleteLogsNotice.waitFor();
  await deleteLogsNotice.getByRole("button", { name: "确定", exact: true }).click();
  await logManager.getByRole("button", { name: "关闭", exact: true }).last().click();
  await logManager.waitFor({ state: "detached" });
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
  const visibleMcpScopes = await mcpDialog.locator(".mcp-check-grid label").allTextContents();
  assert(JSON.stringify(visibleMcpScopes.map((scope) => scope.trim())) === JSON.stringify([
    "read-sessions", "read-logs", "read-transfers", "read-tunnels", "read-scripts",
    "write-input", "transfer", "tunnel", "manage-sessions", "run-scripts",
  ]), `MCP grant editor omitted transfer/route scopes: ${JSON.stringify(visibleMcpScopes)}`);
  await mcpDialog.locator(".mcp-new").click();
  const newGrantClientId = mcpDialog.locator(".dialog-field", { hasText: "Client ID:" }).locator("input");
  await page.waitForFunction(() => document.activeElement?.matches(".mcp-editor .dialog-field input"));
  assert(await newGrantClientId.inputValue() === ""
    && await newGrantClientId.evaluate((input) => input === document.activeElement)
    && await mcpDialog.locator(".mcp-grant-draft.active", { hasText: "新授权" }).count() === 1
    && await mcpDialog.getByRole("button", { name: "保存", exact: true }).isDisabled(),
  "MCP new grant action did not create and focus an explicit blank draft");
  await mcpDialog.getByRole("button", { name: "随机生成 Client ID", exact: true }).click();
  const generatedClientId = await newGrantClientId.inputValue();
  assert(/^client-[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(generatedClientId)
    && await newGrantClientId.evaluate((input) => input === document.activeElement),
  `MCP random Client ID action produced an invalid value: ${generatedClientId}`);
  await page.evaluate(() => {
    window.__mcpGrantDiscardPrompts = [];
    window.__originalMcpConfirm = window.confirm;
    window.confirm = (message) => {
      window.__mcpGrantDiscardPrompts.push(String(message));
      return false;
    };
  });
  await mcpDialog.locator(".mcp-grants > button", { hasText: mcpGrants[0].name }).click();
  assert(await newGrantClientId.inputValue() === generatedClientId
    && await mcpDialog.locator(".mcp-grant-draft.active").count() === 1,
  "MCP grant switch discarded a new unsaved draft after cancellation");
  await page.evaluate(() => {
    window.confirm = (message) => {
      window.__mcpGrantDiscardPrompts.push(String(message));
      return true;
    };
  });
  await mcpDialog.locator(".mcp-grants > button", { hasText: mcpGrants[0].name }).click();
  const mcpGrantDiscardPrompts = await page.evaluate(() => {
    window.confirm = window.__originalMcpConfirm;
    return window.__mcpGrantDiscardPrompts;
  });
  assert(mcpGrantDiscardPrompts.length === 2
    && mcpGrantDiscardPrompts.every((prompt) => prompt.includes("切换授权"))
    && mcpGrantDiscardPrompts.every((prompt) => !prompt.includes(generatedClientId)),
  `MCP grant discard confirmation was missing or exposed draft values: ${JSON.stringify(mcpGrantDiscardPrompts)}`);
  const mcpGrantEditorBounds = await mcpDialog.locator(".mcp-editor").evaluate((editor) => {
    const editorRect = editor.getBoundingClientRect();
    const actionsRect = editor.querySelector(".mcp-actions").getBoundingClientRect();
    return {
      editorTop: editorRect.top,
      editorBottom: editorRect.bottom,
      actionsTop: actionsRect.top,
      actionsBottom: actionsRect.bottom,
    };
  });
  assert(mcpGrantEditorBounds.actionsTop >= mcpGrantEditorBounds.editorTop
    && mcpGrantEditorBounds.actionsBottom <= mcpGrantEditorBounds.editorBottom,
  `MCP grant actions are clipped by the editor viewport: ${JSON.stringify(mcpGrantEditorBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-mcp-grants.png`, fullPage: true });

  await page.evaluate(() => {
    window.__mcpRevokePrompts = [];
    window.__originalMcpConfirm = window.confirm;
    window.confirm = (message) => {
      window.__mcpRevokePrompts.push(String(message));
      return true;
    };
  });
  for (const grant of mcpGrants) {
    const grantRow = mcpDialog.locator(".mcp-grants > button", { hasText: grant.name });
    await grantRow.click();
    await mcpDialog.locator(".mcp-actions").getByRole("button", { name: "撤销", exact: true }).click();
    await grantRow.waitFor({ state: "detached" });
  }
  const mcpRevokePrompts = await page.evaluate(() => {
    window.confirm = window.__originalMcpConfirm;
    return window.__mcpRevokePrompts;
  });
  assert(mcpRevokePrompts.length === mcpGrants.length
    && mcpGrants.every((grant) => mcpRevokePrompts.some((prompt) => (
      prompt.includes(grant.clientId) && prompt.includes(grant.name)
    ))),
  `MCP revocation confirmation omitted an exact target: ${JSON.stringify(mcpRevokePrompts)}`);
  await mcpDialog.locator(".mcp-editor-empty").waitFor();
  assert(await mcpDialog.locator(".mcp-grant-draft").count() === 0
    && await mcpDialog.locator(".mcp-editor .dialog-field").count() === 0,
  "an empty MCP grant store still presented an implicit draft");
  await mcpDialog.locator(".mcp-editor-empty").getByRole("button", { name: "新建授权", exact: true }).click();
  await page.waitForFunction(() => document.activeElement?.matches(".mcp-editor .dialog-field input"));
  await newGrantClientId.fill("empty-store-client");
  await mcpDialog.locator(".dialog-field", { hasText: "名称:" }).locator("input").fill("Empty Store Client");
  assert(await mcpDialog.locator(".mcp-check-grid label", { hasText: "read-transfers" }).locator("input").isChecked()
    && await mcpDialog.locator(".mcp-check-grid label", { hasText: "read-tunnels" }).locator("input").isChecked()
    && await mcpDialog.locator(".mcp-check-grid label", { hasText: "read-scripts" }).locator("input").isChecked(),
  "new MCP grants do not default to complete read-only visibility");
  const grantExpiry = mcpDialog.getByLabel("MCP 授权到期时间", { exact: true });
  await grantExpiry.click();
  const grantExpiryEditor = mcpDialog.locator(".mcp-expiry-editor");
  await grantExpiryEditor.getByLabel("MCP 授权到期日期", { exact: true }).fill("2031-04-05");
  await grantExpiryEditor.getByLabel("MCP 授权到期时刻", { exact: true }).fill("06:07");
  const grantExpiryEditorBounds = await grantExpiryEditor.evaluate((editor) => {
    const rect = editor.getBoundingClientRect();
    return {
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
    };
  });
  assert(grantExpiryEditorBounds.left >= 0
    && grantExpiryEditorBounds.right <= grantExpiryEditorBounds.viewportWidth
    && grantExpiryEditorBounds.top >= 0
    && grantExpiryEditorBounds.bottom <= grantExpiryEditorBounds.viewportHeight,
  `MCP grant expiry editor exceeds the viewport: ${JSON.stringify(grantExpiryEditorBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-mcp-grant-expiry.png`, fullPage: true });
  await grantExpiryEditor.getByRole("button", { name: "取消", exact: true }).click();
  assert(await grantExpiry.inputValue() === "" && await grantExpiryEditor.count() === 0,
    "cancelling the MCP grant expiry editor changed the grant");
  await grantExpiry.click();
  await grantExpiryEditor.getByLabel("MCP 授权到期日期", { exact: true }).fill("2031-04-05");
  await grantExpiryEditor.getByLabel("MCP 授权到期时刻", { exact: true }).fill("06:07");
  await grantExpiryEditor.getByRole("button", { name: "确定", exact: true }).click();
  await mcpDialog.locator(".mcp-actions").getByRole("button", { name: "保存", exact: true }).click();
  await mcpDialog.locator(".mcp-grants > button", { hasText: "Empty Store Client" }).waitFor();
  const emptyStoreGrantSave = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "save_mcp_grant" && call.args.grant.clientId === "empty-store-client")
    .at(-1));
  assert(emptyStoreGrantSave?.args.grant.name === "Empty Store Client"
    && Number.isFinite(Date.parse(emptyStoreGrantSave?.args.grant.expiresAt))
    && await grantExpiry.inputValue() === "2031-04-05T06:07",
    `MCP new grant action did not save from an empty store: ${JSON.stringify(emptyStoreGrantSave)}`);

  await page.evaluate(() => {
    window.__deferMcpHttpConfig = true;
    window.__deferMcpHttpRuntimeStatus = true;
  });
  await mcpDialog.getByRole("tab", { name: "HTTP", exact: true }).click();
  await mcpDialog.locator(".mcp-http-view").waitFor();
  assert(await mcpDialog.locator(".mcp-content").count() === 0
    && await mcpDialog.locator(".mcp-audit-view").count() === 0,
  "MCP HTTP page renders inactive task content");
  await page.waitForFunction(() => window.__pendingMcpHttpConfig.length === 1
    && window.__pendingMcpHttpRuntimeStatuses.length === 1);
  assert(await mcpDialog.getByRole("button", { name: "生成 Token", exact: true }).isDisabled(),
    "MCP token generation stayed enabled while HTTP configuration was loading");
  await mcpDialog.getByRole("tab", { name: "审计", exact: true }).click();
  await mcpDialog.getByRole("tab", { name: "HTTP", exact: true }).click();
  await page.waitForFunction(() => window.__pendingMcpHttpRuntimeStatuses.length === 2);
  assert(await page.evaluate(() => window.__pendingMcpHttpConfig.length) === 1,
    "switching back to MCP HTTP started an overlapping configuration request");
  await page.evaluate((config) => {
    for (const pending of window.__pendingMcpHttpConfig) pending.resolve(config);
    window.__pendingMcpHttpConfig = [];
    window.__deferMcpHttpConfig = false;
  }, mcpHttpConfig);
  assert(await mcpDialog.getByRole("button", { name: "启动服务", exact: true }).isDisabled(),
    "MCP HTTP start was enabled before the managed runtime status loaded");
  await page.evaluate(() => {
    for (const pending of window.__pendingMcpHttpRuntimeStatuses) pending.resolve(pending.result);
    window.__pendingMcpHttpRuntimeStatuses = [];
    window.__deferMcpHttpRuntimeStatus = false;
  });
  await mcpDialog.getByRole("button", { name: "轮换 Token", exact: true }).waitFor();
  await page.waitForFunction(() => ![...document.querySelectorAll(".mcp-actions button")]
    .find((button) => button.textContent?.includes("启动服务"))?.disabled);
  const mcpHttpText = await mcpDialog.locator(".mcp-http-panel").textContent();
  assert(mcpHttpText.includes(mcpHttpConfig.endpoint)
    && mcpHttpText.includes(mcpHttpConfig.executable)
    && mcpHttpText.includes(mcpHttpConfig.storePath)
    && await mcpDialog.getByRole("textbox", { name: "MCP HTTP 启动命令", exact: true }).inputValue() === mcpHttpConfig.startCommand
    && !mcpHttpText.includes("cargo run"),
  "MCP HTTP packaged executable/store configuration did not load");
  const mcpListenHost = mcpDialog.getByLabel("MCP HTTP 监听 IP", { exact: true });
  const mcpListenPreset = mcpDialog.getByRole("combobox", { name: "MCP HTTP 监听范围", exact: true });
  await mcpListenPreset.selectOption("0.0.0.0");
  assert(await mcpListenHost.inputValue() === "0.0.0.0",
    "MCP HTTP all-IPv4 listener preset did not update the explicit bind address");
  assert(await mcpDialog.getByRole("button", { name: "保存配置", exact: true }).isDisabled(),
    "MCP HTTP remote listener could be saved without explicit remote approval");
  const mcpRemoteAccess = mcpDialog.getByRole("checkbox", { name: "允许非本机监听", exact: true });
  await mcpRemoteAccess.check();
  await mcpListenHost.fill("127.0.0.1");
  assert(await mcpRemoteAccess.isDisabled() && !await mcpRemoteAccess.isChecked()
    && await mcpListenPreset.inputValue() === "127.0.0.1",
    "MCP HTTP loopback listener retained a stale remote-access approval");
  await mcpListenHost.fill("0.0.0.0");
  assert(!await mcpRemoteAccess.isChecked()
    && await mcpDialog.getByRole("button", { name: "保存配置", exact: true }).isDisabled(),
  "MCP HTTP remote listener reused approval after returning from loopback");
  await mcpRemoteAccess.check();
  const mcpClientHost = mcpDialog.getByLabel("MCP HTTP 客户端地址", { exact: true });
  await mcpClientHost.fill("0.0.0.0");
  assert(await mcpDialog.getByRole("button", { name: "保存配置", exact: true }).isDisabled()
    && await mcpDialog.getByRole("button", { name: "复制 CC Switch JSON", exact: true }).isDisabled(),
  "MCP HTTP wildcard client address produced a connectable client configuration");
  await mcpClientHost.fill("192.168.33.222");
  await mcpDialog.getByRole("spinbutton", { name: "MCP HTTP 端口", exact: true }).fill("9088");
  await mcpDialog.getByLabel("MCP HTTP Client ID", { exact: true }).fill("remote-automation");
  await mcpDialog.getByRole("textbox", { name: "MCP HTTP Allowed Origins", exact: true }).fill("https://console.example.test");
  await mcpDialog.getByRole("checkbox", { name: "授权为空时允许写操作", exact: true }).check();
  await page.waitForFunction(() => document.querySelector(".mcp-http-command")?.value.includes("PORTMATE_MCP_HTTP_ADDR='0.0.0.0:9088'"));
  assert(await mcpDialog.getByRole("button", { name: "复制命令", exact: true }).isEnabled(),
    "MCP HTTP valid unsaved settings did not produce a copyable command preview");
  await page.evaluate(() => { window.__clipboardWriteFailures = 1; });
  await mcpDialog.getByRole("button", { name: "复制命令", exact: true }).click();
  await mcpDialog.locator(".utility-error", { hasText: "simulated clipboard denial" }).waitFor();
  await mcpDialog.getByRole("button", { name: "复制命令", exact: true }).click();
  await mcpDialog.getByRole("button", { name: "已复制", exact: true }).waitFor();
  assert((await page.evaluate(() => window.__clipboardText)).includes("PORTMATE_MCP_HTTP_ADDR='0.0.0.0:9088'"),
    "MCP HTTP command copy did not recover after a clipboard write failure");
  await page.evaluate(() => {
    window.__mcpHttpDiscardPrompts = [];
    window.__originalMcpConfirm = window.confirm;
    window.confirm = (message) => {
      window.__mcpHttpDiscardPrompts.push(String(message));
      return false;
    };
  });
  await mcpDialog.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
  const retainedMcpHttpDraft = await page.evaluate(() => ({
    prompts: window.__mcpHttpDiscardPrompts,
    dialogVisible: Boolean(document.querySelector(".mcp-dialog")),
    clientHost: document.querySelector('[aria-label="MCP HTTP 客户端地址"]')?.value,
  }));
  assert(retainedMcpHttpDraft.dialogVisible
    && retainedMcpHttpDraft.clientHost === "192.168.33.222"
    && retainedMcpHttpDraft.prompts.length === 1
    && retainedMcpHttpDraft.prompts[0].includes("HTTP 配置")
    && !retainedMcpHttpDraft.prompts[0].includes("192.168.33.222"),
  `MCP HTTP draft was discarded or exposed without confirmation: ${JSON.stringify(retainedMcpHttpDraft)}`);
  await page.evaluate(() => {
    window.confirm = window.__originalMcpConfirm;
    window.__deferMcpHttpMutations = true;
  });
  await mcpDialog.getByRole("button", { name: "保存配置", exact: true }).click();
  await page.waitForFunction(() => window.__pendingMcpHttpMutations.length === 1);
  assert(await mcpListenHost.isDisabled()
    && await mcpClientHost.isDisabled()
    && await mcpDialog.getByRole("textbox", { name: "MCP HTTP Allowed Origins", exact: true }).isDisabled(),
  "MCP HTTP settings remained editable while a save request was pending");
  await page.evaluate(() => {
    for (const pending of window.__pendingMcpHttpMutations) pending.resolve(pending.result);
    window.__pendingMcpHttpMutations = [];
    window.__deferMcpHttpMutations = false;
  });
  await mcpDialog.locator(".mcp-http-row", { hasText: "http://0.0.0.0:9088/mcp" }).waitFor();
  await mcpDialog.locator(".mcp-http-row", { hasText: "http://192.168.33.222:9088/mcp" }).waitFor();
  const remoteCommand = await mcpDialog.getByRole("textbox", { name: "MCP HTTP 启动命令", exact: true }).inputValue();
  assert(await mcpDialog.getByRole("textbox", { name: "CC Switch MCP JSON", exact: true }).inputValue() === ""
    && await mcpDialog.getByRole("button", { name: "复制 CC Switch JSON", exact: true }).isDisabled(),
  "CC Switch JSON exposed a stored Token before an explicit generation or rotation action");
  await mcpDialog.getByRole("button", { name: "轮换 Token", exact: true }).click();
  await mcpDialog.getByText("portmate-test-token", { exact: true }).waitFor();
  const ccSwitchJson = await mcpDialog.getByRole("textbox", { name: "CC Switch MCP JSON", exact: true }).inputValue();
  const parsedCcSwitchJson = JSON.parse(ccSwitchJson);
  await mcpDialog.getByRole("button", { name: "复制 CC Switch JSON", exact: true }).click();
  const copiedCcSwitchJson = await page.evaluate(() => window.__clipboardText);
  const savedMcpHttpSettings = await page.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "save_mcp_http_settings").at(-1)?.args.settings);
  assert(remoteCommand.includes("PORTMATE_MCP_HTTP_ADDR='0.0.0.0:9088'")
    && remoteCommand.includes("PORTMATE_MCP_HTTP_ALLOW_REMOTE=1")
    && remoteCommand.includes("PORTMATE_MCP_TRUSTED=1")
    && remoteCommand.includes("PORTMATE_MCP_CLIENT_ID='remote-automation'")
    && remoteCommand.includes("PORTMATE_MCP_HTTP_ORIGINS='https://console.example.test'")
    && savedMcpHttpSettings.listenHost === "0.0.0.0"
    && savedMcpHttpSettings.clientHost === "192.168.33.222"
    && savedMcpHttpSettings.port === 9088
    && savedMcpHttpSettings.allowRemote === true,
  "MCP HTTP remote listener settings were not persisted into the generated command");
  assert(JSON.stringify(parsedCcSwitchJson) === JSON.stringify({
    portmate: {
      type: "http",
      url: "http://192.168.33.222:9088/mcp",
      headers: {
        Authorization: "Bearer portmate-test-token",
      },
      tool_timeout_sec: 180,
    },
  })
    && copiedCcSwitchJson === ccSwitchJson
    && !ccSwitchJson.includes("mcpServers")
    && ccSwitchJson.includes("portmate-test-token")
    && !ccSwitchJson.includes("bearer_token_env_var")
    && !ccSwitchJson.includes("bearer_token\""),
  `CC Switch MCP JSON is not directly importable or missing its inline token: ${ccSwitchJson}`);
  await page.evaluate(() => { window.__deferMcpHttpRuntimeAction = true; });
  await mcpDialog.getByRole("button", { name: "启动服务", exact: true }).click();
  await page.waitForFunction(() => window.__pendingMcpHttpRuntimeActions.length === 1);
  await mcpDialog.getByRole("tab", { name: "审计", exact: true }).click();
  await mcpDialog.getByRole("tab", { name: "HTTP", exact: true }).click();
  assert(await mcpDialog.getByRole("button", { name: "停止服务", exact: true }).isDisabled(),
    "MCP HTTP runtime action lost its busy state after switching tasks");
  assert(await mcpListenHost.isDisabled()
    && await mcpDialog.getByRole("button", { name: "轮换 Token", exact: true }).isDisabled(),
  "MCP HTTP configuration stayed editable while the managed service was starting");
  await page.evaluate(() => {
    for (const pending of window.__pendingMcpHttpRuntimeActions) pending.resolve(pending.result);
    window.__pendingMcpHttpRuntimeActions = [];
    window.__deferMcpHttpRuntimeAction = false;
  });
  const mcpRuntime = mcpDialog.locator(".mcp-http-runtime");
  await mcpRuntime.filter({ hasText: "运行中" }).waitFor();
  await mcpDialog.getByRole("button", { name: "停止服务", exact: true }).waitFor({ state: "visible" });
  assert(await mcpDialog.getByRole("button", { name: "停止服务", exact: true }).isEnabled(),
    "MCP HTTP runtime action stayed busy after its deferred response completed");
  const mcpRuntimeText = await mcpRuntime.textContent();
  assert(mcpRuntimeText?.includes("PID 4242")
    && await mcpListenHost.isDisabled()
    && await mcpDialog.getByRole("button", { name: "保存配置", exact: true }).isDisabled()
    && await mcpDialog.getByRole("button", { name: "轮换 Token", exact: true }).isDisabled(),
  "managed MCP HTTP runtime did not lock its live configuration or expose the process state");
  await page.screenshot({ path: `${screenshotPrefix}-mcp-http.png`, fullPage: true });
  await page.evaluate(() => { window.__deferMcpHttpRuntimeStatus = true; });
  await page.waitForFunction(() => window.__pendingMcpHttpRuntimeStatuses.length === 1);
  await mcpDialog.getByRole("button", { name: "停止服务", exact: true }).click();
  await mcpRuntime.filter({ hasText: "未运行" }).waitFor();
  await page.waitForFunction(() => window.__pendingMcpHttpRuntimeStatuses.length === 2);
  await page.evaluate(() => {
    const stale = window.__pendingMcpHttpRuntimeStatuses.shift();
    stale.resolve(stale.result);
  });
  await page.waitForTimeout(100);
  assert((await mcpRuntime.textContent()).includes("未运行"),
    "a stale MCP HTTP status poll overwrote the completed stop action");
  await page.evaluate(() => {
    for (const pending of window.__pendingMcpHttpRuntimeStatuses) pending.resolve(pending.result);
    window.__pendingMcpHttpRuntimeStatuses = [];
    window.__deferMcpHttpRuntimeStatus = false;
  });
  const managedMcpHttpCalls = await page.evaluate(() => ({
    started: window.__invokeCalls.some((call) => call.command === "start_mcp_http"),
    stopped: window.__invokeCalls.some((call) => call.command === "stop_mcp_http"),
  }));
  assert(!await mcpListenHost.isDisabled()
    && managedMcpHttpCalls.started
    && managedMcpHttpCalls.stopped,
  "managed MCP HTTP runtime did not stop and release its configuration controls");

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
  await page.evaluate(() => {
    window.__mcpHttpRuntime = {
      phase: "running",
      endpoint: window.__mcpHttpConfig.endpoint,
      pid: 4343,
      startedAt: new Date().toISOString(),
      message: null,
    };
  });
  await mcpDialog.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
  await mcpDialog.waitFor({ state: "detached" });
  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.locator(".menu-popover button", { hasText: "MCP Bridge" }).click();
  const reopenedMcpRuntimeDialog = page.locator(".mcp-dialog");
  await reopenedMcpRuntimeDialog.getByRole("tab", { name: "HTTP", exact: true }).click();
  await reopenedMcpRuntimeDialog.locator(".mcp-http-runtime", { hasText: "运行中" }).waitFor();
  assert((await reopenedMcpRuntimeDialog.locator(".mcp-http-runtime").textContent()).includes("PID 4343"),
    "reopening MCP Bridge did not reload the managed runtime process state");
  await reopenedMcpRuntimeDialog.getByRole("button", { name: "停止服务", exact: true }).click();
  await reopenedMcpRuntimeDialog.locator(".mcp-http-runtime", { hasText: "未运行" }).waitFor();
  await reopenedMcpRuntimeDialog.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
  await reopenedMcpRuntimeDialog.waitFor({ state: "detached" });

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
  await page.evaluate(() => {
    window.__deferMcpApprovalResponses = true;
    window.__pendingMcpApprovalResponses = [];
  });
  await approvalDialog.getByRole("button", { name: "本次允许", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingMcpApprovalResponses.length === 1);
  assert(await approvalDialog.getByRole("button", { name: "本次允许", exact: true }).isDisabled()
    && await approvalDialog.getByRole("button", { name: "拒绝", exact: true }).isDisabled(),
  "MCP approval controls stayed enabled while a response was pending");
  await page.evaluate(() => window.__pendingMcpApprovalResponses.shift().resolve());
  await page.waitForFunction(() => document.querySelector(".mcp-approval-dialog")?.textContent?.includes("断开会话"));
  await page.waitForFunction(() => document.activeElement?.textContent?.includes("拒绝"));
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => window.__pendingMcpApprovalResponses.length === 1);
  await page.evaluate(() => {
    window.__pendingMcpApprovalResponses.shift().resolve();
    window.__deferMcpApprovalResponses = false;
  });
  await approvalDialog.waitFor({ state: "detached" });
  const approvalCalls = await page.evaluate((expectedIds) => window.__invokeCalls
    .filter((call) => call.command === "respond_mcp_approval")
    .map((call) => call.args)
    .filter((args) => expectedIds.includes(args.approvalId)), approvalIds);
  assert(JSON.stringify(approvalCalls) === JSON.stringify([
    { approvalId: approvalIds[0], approved: true },
    { approvalId: approvalIds[1], approved: false },
  ]), `MCP approval responses are wrong: ${JSON.stringify(approvalCalls)}`);

  const expiringApprovalId = await page.evaluate(() => {
    const now = Date.now();
    const id = "44444444-4444-4444-8444-444444444444";
    window.__deferMcpApprovalResponses = true;
    window.__pendingMcpApprovalResponses = [];
    window.__emitTauriEvent("portmate-mcp-approval", {
      id,
      clientId: "expiring-client",
      action: "run_command",
      sessionId: "edge-router",
      scope: "write-input",
      createdAt: new Date(now).toISOString(),
      expiresAt: new Date(now + 2_000).toISOString(),
    });
    return id;
  });
  await approvalDialog.waitFor();
  await approvalDialog.getByRole("button", { name: "本次允许", exact: true }).click();
  await page.waitForFunction(() => window.__pendingMcpApprovalResponses.length === 1);
  await page.waitForTimeout(2_200);
  assert(await approvalDialog.count() === 1
    && await approvalDialog.getByRole("button", { name: "本次允许", exact: true }).isDisabled(),
  "an in-flight MCP approval expired before its backend decision completed");
  await page.evaluate(() => {
    window.__pendingMcpApprovalResponses.shift().reject(new Error("approval expired during response"));
    window.__deferMcpApprovalResponses = false;
  });
  await approvalDialog.waitFor({ state: "detached" });
  const expiringApprovalCalls = await page.evaluate((approvalId) => window.__invokeCalls
    .filter((call) => call.command === "respond_mcp_approval" && call.args.approvalId === approvalId)
    .length, expiringApprovalId);
  assert(expiringApprovalCalls === 1,
    `expiring MCP approval submitted ${expiringApprovalCalls} responses`);

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
    return snapshot?.version === 7
      && JSON.stringify(snapshot.docks?.right) === JSON.stringify(["sysmon", "history", "sender"])
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
      rightVisiblePanels: [...document.querySelectorAll('.workspace-dock[data-dock="right"] .workspace-dock-content:not([hidden])')]
        .map((panel) => panel.getAttribute("data-panel")),
      bottomDock: document.querySelector('.workspace-dock[data-dock="bottom"]') !== null,
      docks: snapshot.docks,
    };
  });
  assert(movedDockLayout.dockCount === 2
    && JSON.stringify(movedDockLayout.rightTabs) === JSON.stringify(["history", "sender"])
    && JSON.stringify(movedDockLayout.rightPanels) === JSON.stringify(["history", "sender"])
    && JSON.stringify(movedDockLayout.rightVisiblePanels) === JSON.stringify(["sender"])
    && !movedDockLayout.bottomDock
    && movedDockLayout.docks.active.right === "sender",
  `cross-dock drag did not preserve the right dock tabs: ${JSON.stringify(movedDockLayout)}`);

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
  await togglePanel("Sysmon 侧栏");
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

  await page.locator(".menu-trigger", { hasText: "工具" }).click();
  await page.getByRole("button", { name: "自定义脚本", exact: true }).click();
  const mobileCustomScripts = page.locator(".custom-script-dialog");
  await mobileCustomScripts.waitFor();
  const mobileCustomScriptBounds = await mobileCustomScripts.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    const content = dialog.querySelector(".custom-script-content");
    const actions = dialog.querySelector(".custom-script-actions")?.getBoundingClientRect();
    return {
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      scrollWidth: dialog.scrollWidth,
      width: rect.width,
      contentScrollWidth: content?.scrollWidth ?? 0,
      contentWidth: content?.clientWidth ?? 0,
      actionsReachable: Boolean(actions && actions.top >= rect.top && actions.bottom <= rect.bottom),
    };
  });
  assert(mobileCustomScriptBounds.left >= 0 && mobileCustomScriptBounds.right <= mobile.viewportWidth
    && mobileCustomScriptBounds.top >= 0 && mobileCustomScriptBounds.bottom <= mobile.viewportHeight
    && mobileCustomScriptBounds.scrollWidth <= mobileCustomScriptBounds.width
    && mobileCustomScriptBounds.contentScrollWidth <= mobileCustomScriptBounds.contentWidth
    && mobileCustomScriptBounds.actionsReachable,
  `mobile custom script workspace exceeds the viewport: ${JSON.stringify(mobileCustomScriptBounds)}`);
  assert(await mobileCustomScripts.getByRole("textbox", { name: "脚本正文", exact: true }).isVisible()
    && await mobileCustomScripts.getByRole("button", { name: "刷新自定义脚本", exact: true }).isVisible()
    && await mobileCustomScripts.getByRole("button", { name: "运行自定义脚本", exact: true }).isVisible()
    && await mobileCustomScripts.getByRole("button", { name: "保存自定义脚本", exact: true }).isVisible(),
  "mobile custom script editor has unreachable controls");
  await page.screenshot({ path: `${screenshotPrefix}-custom-scripts-mobile.png`, fullPage: true });
  await mobileCustomScripts.getByRole("button", { name: "关闭自定义脚本", exact: true }).click();
  await mobileCustomScripts.waitFor({ state: "detached" });

  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "导入会话" }).click();
  const mobileOpenSshImport = page.locator(".session-config-import-dialog");
  await mobileOpenSshImport.waitFor();
  await mobileOpenSshImport.getByRole("textbox", { name: "OpenSSH 配置内容", exact: true }).fill(`Host mobile
  HostName mobile.example.test
  User operator
  IdentityFile ~/.ssh/id_mobile`);
  await mobileOpenSshImport.locator(".session-import-row").waitFor();
  const mobileOpenSshBounds = await mobileOpenSshImport.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    const content = dialog.querySelector(".session-config-import-content");
    return {
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      height: rect.height,
      scrollWidth: dialog.scrollWidth,
      clientWidth: dialog.clientWidth,
      contentScrollWidth: content?.scrollWidth ?? 0,
      contentClientWidth: content?.clientWidth ?? 0,
      modeScrollWidth: dialog.querySelector(".session-import-mode-switch")?.scrollWidth ?? 0,
      modeClientWidth: dialog.querySelector(".session-import-mode-switch")?.clientWidth ?? 0,
      modeWidth: dialog.querySelector(".session-import-mode-switch")?.getBoundingClientRect().width ?? 0,
      modeHeight: dialog.querySelector(".session-import-mode-switch")?.getBoundingClientRect().height ?? 0,
    };
  });
  assert(mobileOpenSshBounds.left >= 0 && mobileOpenSshBounds.right <= 390
    && mobileOpenSshBounds.top >= 0 && mobileOpenSshBounds.bottom <= 844
    && mobileOpenSshBounds.height <= 600
    && mobileOpenSshBounds.scrollWidth <= mobileOpenSshBounds.clientWidth
    && mobileOpenSshBounds.contentScrollWidth <= mobileOpenSshBounds.contentClientWidth
    && mobileOpenSshBounds.modeScrollWidth <= mobileOpenSshBounds.modeClientWidth
    && mobileOpenSshBounds.modeWidth > 0
    && mobileOpenSshBounds.modeHeight > 0,
  `mobile OpenSSH import dialog overflows: ${JSON.stringify(mobileOpenSshBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-openssh-import-mobile.png`, fullPage: true });
  await page.evaluate(() => {
    window.__mobileImportPrompts = [];
    window.__originalMobileImportConfirm = window.confirm;
    window.confirm = (message) => {
      window.__mobileImportPrompts.push(String(message));
      return true;
    };
  });
  await mobileOpenSshImport.getByRole("button", { name: "取消", exact: true }).click();
  await mobileOpenSshImport.waitFor({ state: "detached" });
  const mobileImportPrompts = await page.evaluate(() => {
    window.confirm = window.__originalMobileImportConfirm;
    return window.__mobileImportPrompts;
  });
  assert(mobileImportPrompts.length === 1
    && mobileImportPrompts[0].includes("关闭窗口")
    && !mobileImportPrompts[0].includes("mobile.example.test"),
  `mobile Session import did not safely confirm draft disposal: ${JSON.stringify(mobileImportPrompts)}`);

  await page.evaluate(() => {
    const now = Date.now();
    window.__emitTauriEvent("portmate-mcp-approval", {
      id: "33333333-3333-4333-8333-333333333333",
      clientId: "mobile-ops",
      action: "run_custom_script",
      sessionId: "edge-router",
      scope: "run-scripts",
      target: {
        kind: "custom-script",
        id: "69c06a07-dc48-4d4e-9498-6f42b6deab21",
        label: "Inspect service",
      },
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
  const mobileApprovalText = await mobileApproval.textContent();
  assert(mobileApprovalText.includes("运行自定义脚本")
    && mobileApprovalText.includes("Inspect service")
    && mobileApprovalText.includes("69c06a07-dc48-4d4e-9498-6f42b6deab21"),
  "MCP custom-script approval does not identify the exact saved script");
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
  await page.locator(".mcp-new").click();
  await page.getByLabel("MCP 授权到期时间", { exact: true }).click();
  const mobileMcpExpiryEditor = page.locator(".mcp-expiry-editor");
  await mobileMcpExpiryEditor.scrollIntoViewIfNeeded();
  const mobileMcpExpiryBounds = await mobileMcpExpiryEditor.evaluate((editor) => {
    const rect = editor.getBoundingClientRect();
    return { left: rect.left, right: rect.right, scrollWidth: editor.scrollWidth, width: rect.width };
  });
  assert(mobileMcpExpiryBounds.left >= 0
    && mobileMcpExpiryBounds.right <= mobile.viewportWidth
    && mobileMcpExpiryBounds.scrollWidth <= mobileMcpExpiryBounds.width,
  `mobile MCP grant expiry editor overflows horizontally: ${JSON.stringify(mobileMcpExpiryBounds)}`);
  assert(await mobileMcpExpiryEditor.getByRole("button", { name: "清除", exact: true }).isVisible()
    && await mobileMcpExpiryEditor.getByRole("button", { name: "取消", exact: true }).isVisible()
    && await mobileMcpExpiryEditor.getByRole("button", { name: "确定", exact: true }).isVisible(),
  "mobile MCP grant expiry actions are unreachable");
  await page.screenshot({ path: `${screenshotPrefix}-mcp-grant-expiry-mobile.png`, fullPage: true });
  await mobileMcpExpiryEditor.getByLabel("MCP 授权到期日期", { exact: true }).press("Escape");
  await mobileMcpExpiryEditor.waitFor({ state: "detached" });
  await page.getByRole("tab", { name: "HTTP", exact: true }).click();
  await page.getByLabel("MCP HTTP 监听 IP", { exact: true }).waitFor();
  const mobileMcpHttpBounds = await page.locator(".mcp-http-view").evaluate((view) => ({
    scrollWidth: view.scrollWidth,
    clientWidth: view.clientWidth,
  }));
  assert(mobileMcpHttpBounds.scrollWidth <= mobileMcpHttpBounds.clientWidth,
    `mobile MCP HTTP settings overflow horizontally: ${JSON.stringify(mobileMcpHttpBounds)}`);
  await page.screenshot({ path: `${screenshotPrefix}-mcp-http-mobile.png`, fullPage: true });
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
  const localShellWorkspaceTab = page.locator(".workspace-pane-tab", { hasText: "Local Shell" });
  if (await localShellWorkspaceTab.count()) {
    await localShellWorkspaceTab.click({ button: "right" });
    await page.locator(".workspace-view-context-menu").getByRole("button", { name: "关闭视图", exact: true }).click();
    await localShellWorkspaceTab.waitFor({ state: "detached" });
  }
  const deleteTarget = page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" });
  await page.evaluate(() => {
    window.__deferTailLogs = true;
    window.__pendingTailLogs = [];
    window.__deferSessionLists = true;
    window.__pendingSessionLists = [];
    window.__deferSessionProfileDeletes = true;
    window.__pendingSessionProfileDeletes = [];
    window.__emitSessionProfileDeleteBeforeResolve = true;
  });
  await deleteTarget.click();
  await page.locator(".workspace-pane-tab", { hasText: "Bench UART" }).waitFor();
  await page.waitForFunction(() => window.__pendingTailLogs.some((request) => request.args.sessionId === "bench-uart"));
  await page.waitForFunction(() => window.__pendingSessionLists.length >= 1);
  await deleteTarget.click({ button: "right" });
  const deleteAction = page.locator(".context-menu-row", { hasText: "删除会话 Profile" });
  await deleteAction.waitFor();
  await page.screenshot({ path: `${screenshotPrefix}-profile-delete.png`, fullPage: true });
  await page.evaluate(() => {
    window.__profileDeletePrompts = [];
    window.__originalProfileDeleteConfirm = window.confirm;
    window.confirm = (message) => {
      window.__profileDeletePrompts.push(String(message));
      return true;
    };
  });
  await deleteAction.evaluate((button) => {
    button.click();
    button.click();
  });
  await page.waitForFunction(() => window.__pendingSessionProfileDeletes.length === 1);
  await deleteTarget.click({ button: "right" });
  const pendingDeleteAction = page.locator(".context-menu-row", { hasText: "删除会话 Profile" });
  assert(await pendingDeleteAction.isDisabled(),
    "pending Profile deletion left the destructive action enabled");
  await page.mouse.click(1_300, 820);
  const staleSerialControlBaseline = await page.evaluate(() => (
    window.__invokeCalls.filter((call) => call.command === "serial_set_lines").length
  ));
  const staleSerialControlClicked = await page.evaluate(() => {
    window.confirm = window.__originalProfileDeleteConfirm;
    window.__deferSessionProfileDeletes = false;
    window.__pendingSessionProfileDeletes.shift().resolve();
    const staleDtrButton = document.querySelector('.pane-serial-tools button[title="切换 DTR"]');
    staleDtrButton?.click();
    return Boolean(staleDtrButton);
  });
  const deletionNotice = page.locator(".notice-dialog", { hasText: "会话已删除" });
  await deletionNotice.waitFor();
  const staleSerialControlCalls = await page.evaluate(() => (
    window.__invokeCalls.filter((call) => call.command === "serial_set_lines").length
  ));
  assert(staleSerialControlClicked && staleSerialControlCalls === staleSerialControlBaseline,
    `a stale serial control reached the backend after same-frame Profile deletion: ${JSON.stringify({
      clicked: staleSerialControlClicked,
      before: staleSerialControlBaseline,
      after: staleSerialControlCalls,
    })}`);
  const deletionPrompts = await page.evaluate(() => window.__profileDeletePrompts);
  assert(deletionPrompts.length === 1
    && deletionPrompts[0].includes("Bench UART")
    && deletionPrompts[0].includes("磁盘日志分片与安全审计保留"),
  `profile deletion was confirmed repeatedly or omitted its target/retention boundary: ${JSON.stringify(deletionPrompts)}`);
  assert(await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" }).count() === 0,
    "deleted profile remained in the resource explorer");
  assert(await page.locator(".workspace-pane-tab", { hasText: "Bench UART" }).count() === 0,
    "deleted profile retained a workspace view");
  await page.waitForFunction(() => !localStorage.getItem("portmate.workspace.v1")?.includes("bench-uart"));
  const deleteCalls = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "delete_session_profile"));
  const deleteEventOrderingState = await page.evaluate(() => ({
    eventBeforeResponsePending: window.__emitSessionProfileDeleteBeforeResolve,
    pendingDeletes: window.__pendingSessionProfileDeletes.length,
  }));
  assert(deleteCalls.length === 1
    && deleteCalls[0].args.sessionId === "bench-uart"
    && deleteEventOrderingState.eventBeforeResponsePending === false
    && deleteEventOrderingState.pendingDeletes === 0,
    `profile deletion did not reach the backend exactly once: ${JSON.stringify(deleteCalls)}`);
  await deletionNotice.getByRole("button", { name: "确定", exact: true }).click();
  await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Local Shell" }).click();
  const workspaceViewOpenedDuringDeleteRefresh = page.locator(".workspace-pane-tab", { hasText: "Local Shell" });
  await workspaceViewOpenedDuringDeleteRefresh.waitFor();

  const staleDeletedLogMarker = "STALE-DELETED-PROFILE-LOG";
  await page.evaluate((marker) => {
    for (const pending of window.__pendingTailLogs) {
      if (pending.args.sessionId === "bench-uart") {
        pending.resolve([{
          id: "stale-deleted-profile-log",
          sessionId: "bench-uart",
          paneId: "bench-uart:main",
          ts: new Date().toISOString(),
          direction: "inbound",
          stream: "stdout",
          bytesRef: null,
          text: marker,
          annotations: {},
        }]);
      } else {
        pending.resolve(window.__events.filter((event) => event.sessionId === pending.args.sessionId));
      }
    }
    window.__pendingTailLogs = [];
    window.__deferTailLogs = false;
    for (const pending of window.__pendingSessionLists) pending.resolve(pending.result);
    window.__pendingSessionLists = [];
    window.__deferSessionLists = false;
  }, staleDeletedLogMarker);
  await page.waitForTimeout(100);
  assert(await page.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Bench UART" }).count() === 0,
    "a list_sessions response that completed after Profile deletion restored the deleted Profile");
  assert(await workspaceViewOpenedDuringDeleteRefresh.count() === 1,
    "a list_sessions response that completed after Profile deletion discarded a newer workspace view");
  await page.getByRole("button", { name: "搜索会话", exact: true }).click();
  await page.getByRole("tab", { name: "日志", exact: true }).click();
  const deletedLogSearch = page.getByRole("combobox", { name: "搜索会话和日志", exact: true });
  await deletedLogSearch.fill(staleDeletedLogMarker);
  assert(await page.getByRole("option").count() === 0,
    "a tail_log response that completed after Profile deletion restored the deleted log state");
  await deletedLogSearch.press("Escape");
  await page.locator(".search-dialog").waitFor({ state: "detached" });

  const terminalWrites = await page.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "send_text" || call.command === "send_bytes" || call.command === "run_command"
  )));
  assert(terminalWrites.length === 0,
    `non-input workspace actions wrote to the terminal: ${JSON.stringify(terminalWrites)}`);
  assert(pageErrors.length === 0, `browser exceptions: ${JSON.stringify(pageErrors)}`);

  const startupRacePage = await context.newPage();
  const startupRaceErrors = [];
  startupRacePage.on("pageerror", (error) => startupRaceErrors.push(error.message));
  await startupRacePage.goto(appUrl);
  await startupRacePage.evaluate(() => {
    sessionStorage.setItem("portmate.workspaceUiCheck.deferStartupSessions", "true");
  });
  await startupRacePage.reload();
  await startupRacePage.getByRole("button", { name: "会话", exact: true }).waitFor();
  await startupRacePage.waitForFunction(() => window.__pendingSessionLists.length >= 2);
  await startupRacePage.getByRole("button", { name: "会话", exact: true }).click();
  await startupRacePage.getByRole("button", { name: "新建会话", exact: true }).click();
  const startupSessionDialog = startupRacePage.locator(".session-settings-dialog");
  await startupSessionDialog.waitFor();
  await startupSessionDialog.getByRole("heading", { name: "会话信息", exact: true }).waitFor();
  await startupSessionDialog.getByRole("textbox", { name: "会话名称", exact: true }).fill("Startup Race Profile");
  await startupSessionDialog.getByRole("button", { name: "仅保存", exact: true }).click();
  await startupSessionDialog.waitFor({ state: "detached" });
  const startupRaceProfile = startupRacePage.locator(".tree-session", { hasText: "Startup Race Profile" });
  await startupRaceProfile.waitFor();
  await startupRacePage.evaluate(() => {
    const stale = window.__pendingSessionLists.splice(0);
    for (const request of stale) request.resolve(request.result);
  });
  await startupRacePage.waitForFunction(() => window.__pendingSessionLists.length === 1);
  await startupRacePage.evaluate(() => {
    window.__deferSessionLists = false;
    const replacement = window.__pendingSessionLists.shift();
    replacement.resolve(replacement.result);
  });
  await startupRacePage.waitForTimeout(200);
  const startupHydrationState = await startupRacePage.evaluate(() => ({
    backend: window.__sessions.map((session) => session.profile.name),
    visible: [...document.querySelectorAll(".tree-session")].map((item) => item.textContent?.trim() ?? ""),
    listCalls: window.__invokeCalls.filter((call) => call.command === "list_sessions").length,
    pending: window.__pendingSessionLists.length,
  }));
  assert(await startupRaceProfile.count() === 1,
    `a stale startup session snapshot removed a Profile saved during hydration: ${JSON.stringify(startupHydrationState)}`);
  assert(startupHydrationState.visible.some((name) => name.includes("Local Shell"))
    && startupHydrationState.visible.some((name) => name.includes("Bench UART")),
  `replacement startup hydration did not restore the complete authoritative session list: ${JSON.stringify(startupHydrationState)}`);
  assert(startupRaceErrors.length === 0, `startup hydration browser exceptions: ${JSON.stringify(startupRaceErrors)}`);
  await startupRacePage.close();

  const inactiveStartupPage = await context.newPage();
  const inactiveStartupErrors = [];
  inactiveStartupPage.on("pageerror", (error) => inactiveStartupErrors.push(error.message));
  await inactiveStartupPage.goto(appUrl);
  await inactiveStartupPage.evaluate(() => {
    sessionStorage.setItem("portmate.workspaceUiCheck.recoverInactiveStartup", "true");
  });
  await inactiveStartupPage.reload();
  await inactiveStartupPage.waitForFunction(() => window.__invokeCalls.some((call) => (
    call.command === "open_session" && call.args.request?.sessionId === "local-shell"
  )));
  const inactiveStartupState = await inactiveStartupPage.evaluate(() => ({
    status: window.__sessions.find((session) => session.profile.id === "local-shell")?.runtime.status ?? "missing",
    opens: window.__invokeCalls.filter((call) => call.command === "open_session" && call.args.request?.sessionId === "local-shell").length,
    connectedOpens: window.__invokeCalls.filter((call) => (
      call.command === "open_session" && call.args.request?.sessionId !== "local-shell"
    )).length,
  }));
  assert(inactiveStartupState.status === "connected"
    && inactiveStartupState.opens === 1
    && inactiveStartupState.connectedOpens === 0,
  `startup recovery did not connect only the configured inactive session: ${JSON.stringify(inactiveStartupState)}`);
  assert(inactiveStartupErrors.length === 0,
    `inactive startup recovery browser exceptions: ${JSON.stringify(inactiveStartupErrors)}`);
  await inactiveStartupPage.close();

  const silentSshStartupPage = await context.newPage();
  const silentSshStartupErrors = [];
  silentSshStartupPage.on("pageerror", (error) => silentSshStartupErrors.push(error.message));
  await silentSshStartupPage.goto(appUrl);
  await silentSshStartupPage.evaluate(() => {
    sessionStorage.setItem("portmate.workspaceUiCheck.recoverSilentSshStartup", "true");
  });
  await silentSshStartupPage.reload();
  const silentSshTab = silentSshStartupPage.locator('.workspace-pane-tab[data-view-id="view-edge"]');
  await silentSshStartupPage.waitForFunction(() => window.__invokeCalls.some((call) => (
    call.command === "open_session" && call.args.request?.sessionId === "edge-router"
  )));
  await silentSshTab.waitFor();
  await silentSshStartupPage.waitForFunction(() => {
    const tab = document.querySelector('.workspace-pane-tab.status-error[data-view-id="view-edge"]');
    return Boolean(tab?.querySelector(".session-status-dot")
      && tab.querySelector(".workspace-pane-tab-label")?.getAttribute("title")?.includes("simulated silent startup failure"));
  });
  const silentSshStartupState = await silentSshStartupPage.evaluate(() => {
    const open = window.__invokeCalls.find((call) => (
      call.command === "open_session" && call.args.request?.sessionId === "edge-router"
    ));
    const dot = document.querySelector('.workspace-pane-tab[data-view-id="view-edge"] .session-status-dot');
    const tab = dot?.closest(".workspace-pane-tab");
    const label = tab?.querySelector(".workspace-pane-tab-label");
    return {
      credentialHandle: open?.args.request?.credentialHandle,
      inlinePassword: open?.args.password,
      inlinePassphrase: open?.args.passphrase,
      stagedCredentials: window.__invokeCalls.filter((call) => call.command === "stage_session_credentials").length,
      credentialDialogs: document.querySelectorAll(".credential-dialog").length,
      hostKeyDialogs: document.querySelectorAll(".hostkey-dialog").length,
      noticeDialogs: document.querySelectorAll(".notice-dialog").length,
      dotColor: dot ? getComputedStyle(dot).backgroundColor : "missing",
      title: label?.getAttribute("title") ?? "",
    };
  });
  assert(silentSshStartupState.credentialHandle === null
    && silentSshStartupState.inlinePassword === undefined
    && silentSshStartupState.inlinePassphrase === undefined
    && silentSshStartupState.stagedCredentials === 0
    && silentSshStartupState.credentialDialogs === 0
    && silentSshStartupState.hostKeyDialogs === 0
    && silentSshStartupState.noticeDialogs === 0
    && silentSshStartupState.dotColor === "rgb(248, 113, 113)"
    && silentSshStartupState.title.includes("连接错误")
    && silentSshStartupState.title.includes("simulated silent startup failure"),
    `SSH startup recovery was not silent or did not expose its red tab status: ${JSON.stringify(silentSshStartupState)}`);
  assert(silentSshStartupErrors.length === 0,
    `silent SSH startup recovery browser exceptions: ${JSON.stringify(silentSshStartupErrors)}`);
  await silentSshStartupPage.close();

  const startupDomainPage = await context.newPage();
  const startupDomainErrors = [];
  startupDomainPage.on("pageerror", (error) => startupDomainErrors.push(error.message));
  await startupDomainPage.goto(appUrl);
  await startupDomainPage.evaluate(() => {
    sessionStorage.setItem("portmate.workspaceUiCheck.deferStartupDomains", "true");
  });
  await startupDomainPage.reload();
  await startupDomainPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await startupDomainPage.waitForFunction(() => (
    window.__pendingTransferLists.length >= 2 && window.__pendingGrantLists.length >= 2
  ));
  await startupDomainPage.waitForFunction(() => (
    (window.__tauriEventListeners.get("portmate-transfer-task") || []).length > 0
  ));
  const hydrationTransfer = {
    id: "startup-hydration-transfer",
    sessionId: "edge-router",
    protocol: "sftp",
    source: "/tmp/startup-source.bin",
    destination: "/srv/startup-target.bin",
    bytesTotal: 1024,
    bytesDone: 512,
    status: "running",
    message: "startup transfer retained",
    startedAt: new Date().toISOString(),
    finishedAt: null,
    averageBytesPerSecond: 256,
  };
  await startupDomainPage.evaluate((task) => {
    window.__transfers = [structuredClone(task)];
    window.__emitTauriEvent("portmate-transfer-task", task);
  }, hydrationTransfer);

  await startupDomainPage.getByRole("button", { name: "工具", exact: true }).click();
  await startupDomainPage.getByRole("button", { name: "MCP Bridge", exact: true }).click();
  const startupMcpDialog = startupDomainPage.locator(".mcp-dialog");
  await startupMcpDialog.waitFor();
  await startupMcpDialog.locator(".mcp-new").click();
  await startupMcpDialog.locator(".dialog-field", { hasText: "Client ID:" }).locator("input").fill("startup-hydration-client");
  await startupMcpDialog.locator(".dialog-field", { hasText: "名称:" }).locator("input").fill("Startup Hydration Client");
  await startupMcpDialog.locator(".mcp-actions").getByRole("button", { name: "保存", exact: true }).click();
  await startupMcpDialog.locator(".mcp-grants button", { hasText: "Startup Hydration Client" }).waitFor();

  await startupDomainPage.evaluate(() => {
    for (const pending of window.__pendingTransferLists.splice(0)) pending.resolve(pending.result);
    for (const pending of window.__pendingGrantLists.splice(0)) pending.resolve(pending.result);
  });
  await startupDomainPage.waitForFunction(() => (
    window.__pendingTransferLists.length === 1 && window.__pendingGrantLists.length === 1
  ));
  await startupDomainPage.evaluate(() => {
    window.__deferTransferLists = false;
    window.__deferGrantLists = false;
    const transferReplacement = window.__pendingTransferLists.shift();
    const grantReplacement = window.__pendingGrantLists.shift();
    transferReplacement.resolve(transferReplacement.result);
    grantReplacement.resolve(grantReplacement.result);
  });
  await startupDomainPage.waitForTimeout(200);
  const startupGrantLabels = await startupMcpDialog.locator(".mcp-grants button strong").allTextContents();
  assert(startupGrantLabels.includes("Startup Hydration Client")
    && startupGrantLabels.includes("Operations Console")
    && startupGrantLabels.includes("Audit Reader"),
  `startup grant hydration did not converge: ${JSON.stringify(startupGrantLabels)}`);
  await startupMcpDialog.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
  await startupDomainPage.getByRole("button", { name: "工具", exact: true }).click();
  await startupDomainPage.getByRole("button", { name: "传输任务", exact: true }).click();
  const startupTransferRow = startupDomainPage.locator(".transfer-row", { hasText: "/tmp/startup-source.bin" });
  await startupTransferRow.waitFor();
  await startupDomainPage.evaluate(() => {
    window.__trackedTransferDismissTimers = [];
    window.__originalTransferSetTimeout = window.setTimeout;
    window.__originalTransferClearTimeout = window.clearTimeout;
    window.setTimeout = (handler, delay, ...args) => {
      const timerId = window.__originalTransferSetTimeout(handler, delay, ...args);
      const numericDelay = Number(delay);
      if (numericDelay >= 5_000 && numericDelay <= 6_100) {
        window.__trackedTransferDismissTimers.push({ timerId, delay: numericDelay, cancelled: false });
      }
      return timerId;
    };
    window.clearTimeout = (timerId) => {
      const tracked = window.__trackedTransferDismissTimers.find((item) => item.timerId === timerId);
      if (tracked) tracked.cancelled = true;
      return window.__originalTransferClearTimeout(timerId);
    };
  });
  const completedAt = new Date().toISOString();
  await startupDomainPage.evaluate(({ task, completedAt }) => {
    window.__emitTauriEvent("portmate-transfer-task", {
      ...task,
      bytesDone: task.bytesTotal,
      status: "completed",
      message: "completed",
      finishedAt: completedAt,
    });
  }, { task: hydrationTransfer, completedAt });
  await startupTransferRow.getByRole("button", { name: "关闭已完成传输", exact: true }).click();
  await startupTransferRow.waitFor({ state: "detached" });
  await startupDomainPage.waitForFunction(() => window.__trackedTransferDismissTimers.length === 1
    && window.__trackedTransferDismissTimers[0].cancelled);
  await startupDomainPage.evaluate((task) => {
    window.__emitTauriEvent("portmate-transfer-task", {
      ...task,
      id: "startup-dismiss-timer-probe",
      source: "/tmp/dismiss-timer-probe.bin",
      destination: "/srv/dismiss-timer-probe.bin",
      status: "queued",
      message: "queued",
      bytesDone: 0,
      startedAt: null,
      finishedAt: null,
      averageBytesPerSecond: null,
    });
  }, hydrationTransfer);
  await startupDomainPage.waitForTimeout(100);
  const transferDismissTimerState = await startupDomainPage.evaluate(() => {
    const state = structuredClone(window.__trackedTransferDismissTimers);
    window.setTimeout = window.__originalTransferSetTimeout;
    window.clearTimeout = window.__originalTransferClearTimeout;
    delete window.__originalTransferSetTimeout;
    delete window.__originalTransferClearTimeout;
    return state;
  });
  assert(transferDismissTimerState.length === 1 && transferDismissTimerState[0].cancelled,
    `a hidden completed transfer retained or recreated its timer: ${JSON.stringify(transferDismissTimerState)}`);
  await startupDomainPage.locator(".transfer-dialog .utility-actions button", { hasText: "取消" }).click();
  await startupDomainPage.locator(".transfer-dialog").waitFor({ state: "detached" });
  await startupDomainPage.getByRole("button", { name: "工具", exact: true }).click();
  await startupDomainPage.getByRole("button", { name: "传输任务", exact: true }).click();
  await startupDomainPage.locator(".transfer-dialog").waitFor();
  await startupTransferRow.waitFor({ state: "detached", timeout: 1_000 });
  const restoredCompletedTransfer = {
    ...hydrationTransfer,
    id: "startup-restored-completed-transfer",
    source: "/tmp/restored-completed-source.bin",
    destination: "/srv/restored-completed-target.bin",
    bytesDone: hydrationTransfer.bytesTotal,
    status: "completed",
    message: "completed",
    finishedAt: new Date(Date.now() - 60_000).toISOString(),
  };
  await startupDomainPage.evaluate((task) => {
    window.__emitTauriEvent("portmate-transfer-task", task);
  }, restoredCompletedTransfer);
  const restoredCompletedTransferRow = startupDomainPage.locator(".transfer-row", {
    hasText: "/tmp/restored-completed-source.bin",
  });
  await restoredCompletedTransferRow.waitFor({ state: "detached", timeout: 1_000 });
  const lateTimestampTransfer = {
    ...restoredCompletedTransfer,
    id: "startup-late-completion-timestamp",
    source: "/tmp/late-completion-source.bin",
    destination: "/srv/late-completion-target.bin",
    finishedAt: null,
  };
  await startupDomainPage.evaluate((task) => {
    window.__emitTauriEvent("portmate-transfer-task", task);
  }, lateTimestampTransfer);
  const lateTimestampTransferRow = startupDomainPage.locator(".transfer-row", {
    hasText: "/tmp/late-completion-source.bin",
  });
  await lateTimestampTransferRow.waitFor();
  await startupDomainPage.evaluate((task) => {
    window.__emitTauriEvent("portmate-transfer-task", {
      ...task,
      finishedAt: new Date(Date.now() - 60_000).toISOString(),
    });
  }, lateTimestampTransfer);
  await lateTimestampTransferRow.waitFor({ state: "detached", timeout: 1_000 });
  const autoDismissTransfer = {
    ...hydrationTransfer,
    id: "startup-auto-dismiss-transfer",
    source: "/tmp/auto-dismiss-source.bin",
    destination: "/srv/auto-dismiss-target.bin",
    bytesDone: hydrationTransfer.bytesTotal,
    status: "completed",
    message: "completed",
    finishedAt: completedAt,
  };
  await startupDomainPage.evaluate((task) => {
    window.__emitTauriEvent("portmate-transfer-task", task);
  }, autoDismissTransfer);
  const autoDismissTransferRow = startupDomainPage.locator(".transfer-row", { hasText: "/tmp/auto-dismiss-source.bin" });
  await autoDismissTransferRow.waitFor();
  await autoDismissTransferRow.waitFor({ state: "detached", timeout: 8_000 });
  const startupDomainState = await startupDomainPage.evaluate(() => ({
    transfers: window.__transfers.map((task) => task.id),
    grants: window.__mcpGrants.map((grant) => grant.clientId),
    transferListCalls: window.__invokeCalls.filter((call) => call.command === "list_transfers").length,
    grantListCalls: window.__invokeCalls.filter((call) => call.command === "list_mcp_grants").length,
    pendingTransfers: window.__pendingTransferLists.length,
    pendingGrants: window.__pendingGrantLists.length,
  }));
  assert(startupDomainErrors.length === 0,
    `startup domain hydration browser exceptions: ${JSON.stringify(startupDomainErrors)}`);
  await startupDomainPage.close();

  const grantLifecyclePage = await context.newPage();
  const grantLifecycleErrors = [];
  grantLifecyclePage.on("pageerror", (error) => grantLifecycleErrors.push(error.message));
  await grantLifecyclePage.goto(appUrl);
  await grantLifecyclePage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await grantLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await grantLifecyclePage.getByRole("button", { name: "MCP Bridge", exact: true }).click();
  const lifecycleMcpDialog = grantLifecyclePage.locator(".mcp-dialog");
  await lifecycleMcpDialog.waitFor();
  await lifecycleMcpDialog.locator(".mcp-new").click();
  await lifecycleMcpDialog.locator(".dialog-field", { hasText: "Client ID:" }).locator("input").fill("late-close-client");
  await lifecycleMcpDialog.locator(".dialog-field", { hasText: "名称:" }).locator("input").fill("Late Close Client");
  await grantLifecyclePage.evaluate(() => { window.__deferGrantMutations = true; });
  await lifecycleMcpDialog.locator(".mcp-actions").getByRole("button", { name: "保存", exact: true }).click();
  await grantLifecyclePage.waitForFunction(() => window.__pendingGrantMutations.length === 1);
  assert(await lifecycleMcpDialog.locator(".dialog-field", { hasText: "Client ID:" }).locator("input").isDisabled()
    && await lifecycleMcpDialog.locator(".dialog-field", { hasText: "名称:" }).locator("input").isDisabled(),
  "MCP grant editor remained mutable while a save request was pending");
  await grantLifecyclePage.evaluate(() => {
    window.__mcpPendingSavePrompts = [];
    window.__originalMcpConfirm = window.confirm;
    window.confirm = (message) => {
      window.__mcpPendingSavePrompts.push(String(message));
      return true;
    };
  });
  await lifecycleMcpDialog.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
  await lifecycleMcpDialog.waitFor({ state: "detached" });
  const pendingMcpSavePrompts = await grantLifecyclePage.evaluate(() => {
    window.confirm = window.__originalMcpConfirm;
    return window.__mcpPendingSavePrompts;
  });
  assert(pendingMcpSavePrompts.length === 1
    && pendingMcpSavePrompts[0].includes("授权草稿")
    && !pendingMcpSavePrompts[0].includes("late-close-client"),
  `MCP pending-save close did not protect or exposed its draft: ${JSON.stringify(pendingMcpSavePrompts)}`);
  await grantLifecyclePage.evaluate(() => {
    window.__deferGrantMutations = false;
    const pending = window.__pendingGrantMutations.shift();
    pending.resolve(pending.result);
  });
  await grantLifecyclePage.waitForTimeout(100);
  await grantLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await grantLifecyclePage.getByRole("button", { name: "MCP Bridge", exact: true }).click();
  const reopenedMcpDialog = grantLifecyclePage.locator(".mcp-dialog");
  await reopenedMcpDialog.locator(".mcp-grants button", { hasText: "Late Close Client" }).waitFor();
  const grantLifecycleState = await grantLifecyclePage.evaluate(() => ({
    backend: window.__mcpGrants.map((grant) => grant.clientId),
    pending: window.__pendingGrantMutations.length,
    saveCalls: window.__invokeCalls.filter((call) => call.command === "save_mcp_grant").length,
  }));
  assert(grantLifecycleErrors.length === 0,
    `grant lifecycle browser exceptions: ${JSON.stringify(grantLifecycleErrors)}`);
  await grantLifecyclePage.close();

  const oneKeyLifecyclePage = await context.newPage();
  const oneKeyLifecycleErrors = [];
  oneKeyLifecyclePage.on("pageerror", (error) => oneKeyLifecycleErrors.push(error.message));
  await oneKeyLifecyclePage.goto(appUrl);
  await oneKeyLifecyclePage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await oneKeyLifecyclePage.locator(".tree-session", { hasText: "Edge Router" }).click();
  await oneKeyLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await oneKeyLifecyclePage.getByRole("button", { name: "OneKeys", exact: true }).click();
  const firstOneKeyDialog = oneKeyLifecyclePage.locator(".one-key-dialog");
  await firstOneKeyDialog.waitFor();
  await firstOneKeyDialog.locator(".one-key-fields label", { hasText: "名称" }).locator("input").fill("First deferred key");
  await firstOneKeyDialog.locator(".one-key-fields label", { hasText: "用户名" }).locator("input").fill("first-user");
  await firstOneKeyDialog.locator(".one-key-fields label", { hasText: "密码" }).locator("input").fill("first-secret");
  await firstOneKeyDialog.locator(".one-key-sessions label", { hasText: "Edge Router" }).click();
  await firstOneKeyDialog.locator(".one-key-sessions > header span", { hasText: "1" }).waitFor();
  await oneKeyLifecyclePage.evaluate(() => { window.__deferOneKeyMutations = true; });
  await firstOneKeyDialog.locator('button[title="保存 OneKey"]').evaluate((button) => {
    button.click();
    button.click();
  });
  await oneKeyLifecyclePage.waitForFunction(() => window.__pendingOneKeyMutations.length === 1);
  const pendingOneKeySave = await oneKeyLifecyclePage.evaluate(() => ({
    pending: window.__pendingOneKeyMutations.length,
    saves: window.__invokeCalls.filter((call) => call.command === "save_one_key").length,
  }));
  assert(pendingOneKeySave.pending === 1
    && pendingOneKeySave.saves === 1
    && await firstOneKeyDialog.locator(".one-key-fields label", { hasText: "名称" }).locator("input").isDisabled()
    && await firstOneKeyDialog.getByRole("button", { name: "添加 OneKey", exact: true }).isDisabled(),
  `OneKey save was duplicated or the editor remained mutable: ${JSON.stringify(pendingOneKeySave)}`);
  await oneKeyLifecyclePage.evaluate(() => {
    window.__oneKeyClosePrompts = [];
    window.__originalOneKeyConfirm = window.confirm;
    window.confirm = (message) => {
      window.__oneKeyClosePrompts.push(String(message));
      return true;
    };
  });
  await firstOneKeyDialog.getByRole("button", { name: "关闭 OneKey 管理器", exact: true }).click();
  await firstOneKeyDialog.waitFor({ state: "detached" });
  const pendingOneKeyClosePrompts = await oneKeyLifecyclePage.evaluate(() => {
    window.confirm = window.__originalOneKeyConfirm;
    return window.__oneKeyClosePrompts;
  });
  assert(pendingOneKeyClosePrompts.length === 1
    && pendingOneKeyClosePrompts[0].includes("关闭窗口")
    && !pendingOneKeyClosePrompts[0].includes("first-secret"),
  `OneKey pending-save close did not protect or exposed its secret draft: ${JSON.stringify(pendingOneKeyClosePrompts)}`);

  await oneKeyLifecyclePage.evaluate(() => { window.__deferOneKeyMutations = false; });
  await oneKeyLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await oneKeyLifecyclePage.getByRole("button", { name: "OneKeys", exact: true }).click();
  const secondOneKeyDialog = oneKeyLifecyclePage.locator(".one-key-dialog");
  await secondOneKeyDialog.waitFor();
  await secondOneKeyDialog.locator(".one-key-fields label", { hasText: "名称" }).locator("input").fill("Second current key");
  await secondOneKeyDialog.locator(".one-key-fields label", { hasText: "用户名" }).locator("input").fill("second-user");
  await secondOneKeyDialog.locator(".one-key-fields label", { hasText: "密码" }).locator("input").fill("second-secret");
  await secondOneKeyDialog.locator(".one-key-sessions label", { hasText: "Edge Router" }).click();
  await secondOneKeyDialog.locator(".one-key-sessions > header span", { hasText: "1" }).waitFor();
  await secondOneKeyDialog.locator('button[title="保存 OneKey"]').click();
  await secondOneKeyDialog.locator('.one-key-list [role="option"]', { hasText: "First deferred key" }).waitFor();
  await secondOneKeyDialog.locator('.one-key-list [role="option"]', { hasText: "Second current key" }).waitFor();
  await oneKeyLifecyclePage.evaluate(() => {
    const pending = window.__pendingOneKeyMutations.shift();
    pending.resolve(pending.result);
  });
  await oneKeyLifecyclePage.waitForTimeout(100);
  const oneKeyLifecycleState = await oneKeyLifecyclePage.evaluate(() => ({
    backend: window.__oneKeys.map((item) => item.label),
    visible: [...document.querySelectorAll('.one-key-list [role="option"] strong')].map((item) => item.textContent),
    selected: document.querySelector('.one-key-list [role="option"][aria-selected="true"] strong')?.textContent ?? "",
    pending: window.__pendingOneKeyMutations.length,
    saveCalls: window.__invokeCalls.filter((call) => call.command === "save_one_key").length,
  }));
  assert(JSON.stringify(oneKeyLifecycleState.backend) === JSON.stringify(["First deferred key", "Second current key"])
    && JSON.stringify(oneKeyLifecycleState.visible) === JSON.stringify(["First deferred key", "Second current key"])
    && oneKeyLifecycleState.selected === "Second current key"
    && oneKeyLifecycleState.pending === 0
    && oneKeyLifecycleState.saveCalls === 2,
  `a stale OneKey mutation replaced the latest dialog state: ${JSON.stringify(oneKeyLifecycleState)}`);
  const sendSavedOneKeyUsername = secondOneKeyDialog.getByRole("button", { name: "用户名", exact: true });
  assert(!await sendSavedOneKeyUsername.isDisabled(),
    "saved OneKey username was not available to the connected bound session");
  const oneKeySendStart = await oneKeyLifecyclePage.evaluate(() => (
    window.__invokeCalls.filter((call) => call.command === "send_one_key").length
  ));
  await sendSavedOneKeyUsername.evaluate((button) => {
    button.click();
    button.click();
  });
  await secondOneKeyDialog.getByText("用户名已发送。", { exact: true }).waitFor();
  const oneKeySendCalls = await oneKeyLifecyclePage.evaluate(() => (
    window.__invokeCalls.filter((call) => call.command === "send_one_key").length
  ));
  assert(oneKeySendCalls === oneKeySendStart + 1,
    `OneKey username was sent more than once from a same-frame action: ${oneKeySendCalls - oneKeySendStart}`);
  const secondOneKeyUsername = secondOneKeyDialog.locator(".one-key-fields label", { hasText: "用户名" }).locator("input");
  await secondOneKeyUsername.fill("unsaved-second-user");
  assert(await sendSavedOneKeyUsername.isDisabled(),
    "OneKey manager could send a stale saved username while a different value was visible");
  await oneKeyLifecyclePage.evaluate(() => {
    window.__oneKeyDiscardPrompts = [];
    window.__originalOneKeyConfirm = window.confirm;
    window.confirm = (message) => {
      window.__oneKeyDiscardPrompts.push(String(message));
      return false;
    };
  });
  await secondOneKeyDialog.locator('.one-key-list [role="option"]', { hasText: "First deferred key" }).click();
  await secondOneKeyDialog.getByRole("button", { name: "关闭 OneKey 管理器", exact: true }).click();
  const retainedOneKeyDraft = await oneKeyLifecyclePage.evaluate(() => ({
    prompts: window.__oneKeyDiscardPrompts,
    selected: document.querySelector('.one-key-list [role="option"][aria-selected="true"] strong')?.textContent ?? "",
    username: document.querySelector('.one-key-fields label:nth-child(3) input')?.value ?? "",
    dialogVisible: Boolean(document.querySelector(".one-key-dialog")),
  }));
  assert(retainedOneKeyDraft.dialogVisible
    && retainedOneKeyDraft.selected === "Second current key"
    && retainedOneKeyDraft.username === "unsaved-second-user"
    && retainedOneKeyDraft.prompts.length === 2
    && retainedOneKeyDraft.prompts.some((prompt) => prompt.includes("切换 OneKey"))
    && retainedOneKeyDraft.prompts.some((prompt) => prompt.includes("关闭窗口"))
    && retainedOneKeyDraft.prompts.every((prompt) => !prompt.includes("second-secret")),
  `OneKey draft was discarded or exposed without confirmation: ${JSON.stringify(retainedOneKeyDraft)}`);
  await oneKeyLifecyclePage.evaluate(() => {
    window.confirm = window.__originalOneKeyConfirm;
  });
  await secondOneKeyUsername.fill("second-user");
  await secondOneKeyDialog.getByRole("button", { name: "关闭 OneKey 管理器", exact: true }).click();
  await secondOneKeyDialog.waitFor({ state: "detached" });
  assert(oneKeyLifecycleErrors.length === 0,
    `OneKey lifecycle browser exceptions: ${JSON.stringify(oneKeyLifecycleErrors)}`);
  await oneKeyLifecyclePage.close();

  const deletedOneKeySendPage = await context.newPage();
  const deletedOneKeySendErrors = [];
  deletedOneKeySendPage.on("pageerror", (error) => deletedOneKeySendErrors.push(error.message));
  await deletedOneKeySendPage.goto(appUrl);
  await deletedOneKeySendPage.locator(".tree-session", { hasText: "Edge Router" }).click();
  await deletedOneKeySendPage.getByRole("button", { name: "工具", exact: true }).click();
  await deletedOneKeySendPage.getByRole("button", { name: "OneKeys", exact: true }).click();
  const deletedOneKeyDialog = deletedOneKeySendPage.locator(".one-key-dialog");
  await deletedOneKeyDialog.locator(".one-key-fields label", { hasText: "名称" }).locator("input").fill("Deleted target key");
  await deletedOneKeyDialog.locator(".one-key-fields label", { hasText: "用户名" }).locator("input").fill("deleted-user");
  await deletedOneKeyDialog.locator(".one-key-fields label", { hasText: "密码" }).locator("input").fill("deleted-secret");
  await deletedOneKeyDialog.locator(".one-key-sessions label", { hasText: "Edge Router" }).click();
  await deletedOneKeyDialog.locator('button[title="保存 OneKey"]').click();
  await deletedOneKeyDialog.locator('.one-key-list [role="option"]', { hasText: "Deleted target key" }).waitFor();
  await deletedOneKeySendPage.evaluate(() => { window.__deferOneKeySends = true; });
  await deletedOneKeyDialog.getByRole("button", { name: "用户名", exact: true }).click();
  await deletedOneKeySendPage.waitForFunction(() => window.__pendingOneKeySends.length === 1);
  await deletedOneKeySendPage.evaluate(() => {
    window.__oneKeys = window.__oneKeys.map((item) => ({
      ...item,
      identity: item.identity?.sourceProfileId === "edge-router" ? null : item.identity,
      sessionIds: item.sessionIds.filter((sessionId) => sessionId !== "edge-router"),
    }));
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
  });
  await deletedOneKeySendPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  await deletedOneKeySendPage.waitForFunction(() => {
    const input = document.querySelector(".one-key-fields input");
    return input && !input.disabled;
  });
  await deletedOneKeySendPage.evaluate(() => {
    window.__deferOneKeySends = false;
    window.__pendingOneKeySends.shift().resolve(null);
  });
  await deletedOneKeySendPage.waitForTimeout(100);
  const deletedOneKeySendState = await deletedOneKeySendPage.evaluate(() => ({
    feedback: document.querySelector(".one-key-dialog-actions [role='status']")?.textContent ?? "",
    boundCount: document.querySelector(".one-key-sessions > header span")?.textContent ?? "",
    pending: window.__pendingOneKeySends.length,
  }));
  assert(deletedOneKeySendState.feedback === ""
    && deletedOneKeySendState.boundCount === "0"
    && deletedOneKeySendState.pending === 0,
  `a OneKey send response survived Profile deletion: ${JSON.stringify(deletedOneKeySendState)}`);
  assert(deletedOneKeySendErrors.length === 0,
    `deleted OneKey send browser exceptions: ${JSON.stringify(deletedOneKeySendErrors)}`);
  await deletedOneKeySendPage.close();

  const deletedTerminalInputPage = await context.newPage();
  const deletedTerminalInputErrors = [];
  deletedTerminalInputPage.on("pageerror", (error) => deletedTerminalInputErrors.push(error.message));
  await deletedTerminalInputPage.goto(appUrl);
  await deletedTerminalInputPage.locator(".tree-session", { hasText: "Edge Router" }).click();
  const deletedTerminalInput = deletedTerminalInputPage.locator(".terminal-pane.active .xterm-helper-textarea");
  const deletedTerminalInputStart = await deletedTerminalInputPage.evaluate(() => {
    window.__deferTerminalSends = true;
    window.__pendingTerminalSends = [];
    return window.__invokeCalls.filter((call) => call.command === "send_text").length;
  });
  await deletedTerminalInput.focus();
  await deletedTerminalInputPage.keyboard.press("x");
  await deletedTerminalInputPage.waitForFunction(() => window.__pendingTerminalSends.length === 1);
  await deletedTerminalInputPage.keyboard.press("y");
  await deletedTerminalInputPage.waitForTimeout(50);
  const queuedDeletedInput = await deletedTerminalInputPage.evaluate((start) => window.__invokeCalls
    .filter((call) => call.command === "send_text").slice(start), deletedTerminalInputStart);
  assert(queuedDeletedInput.length === 1 && queuedDeletedInput[0].args.text === "x",
    `terminal input was not queued before Profile deletion: ${JSON.stringify(queuedDeletedInput)}`);
  const deletedTerminalInputMarker = "STALE-DELETED-TERMINAL-INPUT";
  await deletedTerminalInputPage.evaluate((marker) => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
    window.__deferTerminalSends = false;
    window.__pendingTerminalSends.shift().reject(new Error(marker));
  }, deletedTerminalInputMarker);
  await deletedTerminalInputPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  await deletedTerminalInputPage.waitForTimeout(100);
  const deletedTerminalInputState = await deletedTerminalInputPage.evaluate((start) => ({
    calls: window.__invokeCalls.filter((call) => call.command === "send_text").slice(start),
    notices: [...document.querySelectorAll(".notice-dialog")].map((item) => item.textContent),
    pending: window.__pendingTerminalSends.length,
  }), deletedTerminalInputStart);
  assert(deletedTerminalInputState.calls.length === 1
    && deletedTerminalInputState.calls[0].args.text === "x"
    && deletedTerminalInputState.notices.every((notice) => !notice?.includes(deletedTerminalInputMarker))
    && deletedTerminalInputState.pending === 0,
  `queued terminal input survived Profile deletion: ${JSON.stringify(deletedTerminalInputState)}`);
  assert(deletedTerminalInputErrors.length === 0,
    `deleted terminal input browser exceptions: ${JSON.stringify(deletedTerminalInputErrors)}`);
  await deletedTerminalInputPage.close();

  const deletedDetachedTerminalUrl = new URL(detachedUrl);
  deletedDetachedTerminalUrl.searchParams.set("windowId", "deleted-input-window");
  deletedDetachedTerminalUrl.searchParams.set("paneId", "deleted-input-pane");
  deletedDetachedTerminalUrl.searchParams.set("viewId", "deleted-input-view");
  deletedDetachedTerminalUrl.searchParams.set("sessionId", "edge-router");
  deletedDetachedTerminalUrl.searchParams.set("title", "Edge Router");
  const deletedDetachedTerminalPage = await context.newPage();
  const deletedDetachedTerminalErrors = [];
  deletedDetachedTerminalPage.on("pageerror", (error) => deletedDetachedTerminalErrors.push(error.message));
  await deletedDetachedTerminalPage.goto(deletedDetachedTerminalUrl.toString());
  const deletedDetachedTerminalInput = deletedDetachedTerminalPage.locator(".detached-pane-terminal .xterm-helper-textarea");
  await deletedDetachedTerminalInput.waitFor();
  const deletedDetachedTerminalStart = await deletedDetachedTerminalPage.evaluate(() => {
    window.__deferTerminalSends = true;
    window.__pendingTerminalSends = [];
    return window.__invokeCalls.filter((call) => call.command === "send_text").length;
  });
  await deletedDetachedTerminalInput.focus();
  await deletedDetachedTerminalPage.keyboard.press("m");
  await deletedDetachedTerminalPage.waitForFunction(() => window.__pendingTerminalSends.length === 1);
  await deletedDetachedTerminalPage.keyboard.press("n");
  await deletedDetachedTerminalPage.waitForTimeout(50);
  await deletedDetachedTerminalPage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
    window.__deferTerminalSends = false;
    window.__pendingTerminalSends.shift().reject(new Error("STALE-DELETED-DETACHED-INPUT"));
  });
  await deletedDetachedTerminalPage.locator(".detached-pane-status", { hasText: "会话 Profile 已删除" }).waitFor();
  await deletedDetachedTerminalPage.waitForTimeout(100);
  const deletedDetachedTerminalState = await deletedDetachedTerminalPage.evaluate((start) => ({
    calls: window.__invokeCalls.filter((call) => call.command === "send_text").slice(start),
    pending: window.__pendingTerminalSends.length,
    status: document.querySelector(".detached-pane-status")?.textContent ?? "",
  }), deletedDetachedTerminalStart);
  assert(deletedDetachedTerminalState.calls.length === 1
    && deletedDetachedTerminalState.calls[0].args.text === "m"
    && deletedDetachedTerminalState.pending === 0
    && deletedDetachedTerminalState.status.includes("会话 Profile 已删除"),
  `detached terminal input survived Profile deletion: ${JSON.stringify(deletedDetachedTerminalState)}`);
  assert(deletedDetachedTerminalErrors.length === 0,
    `deleted detached terminal input browser exceptions: ${JSON.stringify(deletedDetachedTerminalErrors)}`);
  await deletedDetachedTerminalPage.close();

  const deletedScriptRunPage = await context.newPage();
  const deletedScriptRunErrors = [];
  deletedScriptRunPage.on("pageerror", (error) => deletedScriptRunErrors.push(error.message));
  await deletedScriptRunPage.goto(appUrl);
  await deletedScriptRunPage.locator(".tree-session", { hasText: "Edge Router" }).click();
  await deletedScriptRunPage.getByRole("button", { name: "工具", exact: true }).click();
  await deletedScriptRunPage.getByRole("button", { name: "自定义脚本", exact: true }).click();
  const deletedScriptDialog = deletedScriptRunPage.locator(".custom-script-dialog");
  const deletedScriptRunButton = deletedScriptDialog.getByRole("button", { name: "运行自定义脚本", exact: true });
  await deletedScriptRunPage.evaluate(() => { window.__deferCustomScriptRuns = true; });
  await deletedScriptRunButton.click();
  await deletedScriptRunPage.waitForFunction(() => window.__pendingCustomScriptRuns.length === 1);
  await deletedScriptRunPage.evaluate(() => {
    window.__customScripts = window.__customScripts.map((script) => ({
      ...script,
      allowedSessionIds: script.allowedSessionIds.filter((sessionId) => sessionId !== "edge-router"),
      mcpEnabled: false,
      updatedAt: new Date(Date.now() + 1_000).toISOString(),
    }));
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
  });
  await deletedScriptRunPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  await deletedScriptDialog.getByRole("button", { name: "关闭自定义脚本", exact: true }).waitFor({ state: "visible" });
  await deletedScriptRunPage.waitForFunction(() => !document.querySelector('[aria-label="关闭自定义脚本"]')?.disabled);
  await deletedScriptRunPage.evaluate(() => {
    window.__deferCustomScriptRuns = false;
    const pending = window.__pendingCustomScriptRuns.shift();
    pending.resolve(pending.result);
  });
  await deletedScriptRunPage.waitForTimeout(100);
  const deletedScriptRunState = await deletedScriptRunPage.evaluate(() => ({
    notices: [...document.querySelectorAll(".notice-dialog")].map((item) => item.textContent),
    pending: window.__pendingCustomScriptRuns.length,
    closeDisabled: document.querySelector('[aria-label="关闭自定义脚本"]')?.disabled,
  }));
  assert(deletedScriptRunState.notices.length === 0
    && deletedScriptRunState.pending === 0
    && deletedScriptRunState.closeDisabled === false,
  `a custom script response survived Profile deletion: ${JSON.stringify(deletedScriptRunState)}`);
  assert(deletedScriptRunErrors.length === 0,
    `deleted custom script run browser exceptions: ${JSON.stringify(deletedScriptRunErrors)}`);
  await deletedScriptRunPage.close();

  const quickCommandDraftPage = await context.newPage();
  const quickCommandDraftErrors = [];
  quickCommandDraftPage.on("pageerror", (error) => quickCommandDraftErrors.push(error.message));
  await quickCommandDraftPage.goto(appUrl);
  await quickCommandDraftPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await quickCommandDraftPage.getByRole("button", { name: "工具", exact: true }).click();
  await quickCommandDraftPage.getByRole("button", { name: "快速命令", exact: true }).click();
  const quickCommandDialog = quickCommandDraftPage.locator(".quick-command-dialog");
  await quickCommandDialog.waitFor();
  await quickCommandDialog.getByRole("button", { name: "添加快速命令", exact: true }).click();
  await quickCommandDialog.getByRole("textbox", { name: "快速命令名称", exact: true }).fill("Unsaved private command");
  await quickCommandDialog.getByRole("textbox", { name: "快速命令内容", exact: true }).fill("printf hidden-command-body");
  await quickCommandDraftPage.evaluate(() => {
    window.__quickCommandDiscardPrompts = [];
    window.__originalQuickCommandConfirm = window.confirm;
    window.confirm = (message) => {
      window.__quickCommandDiscardPrompts.push(String(message));
      return false;
    };
  });
  await quickCommandDialog.getByRole("button", { name: "关闭快速命令", exact: true }).click();
  assert(await quickCommandDialog.isVisible()
    && await quickCommandDialog.getByRole("textbox", { name: "快速命令名称", exact: true }).inputValue() === "Unsaved private command"
    && await quickCommandDialog.getByRole("textbox", { name: "快速命令内容", exact: true }).inputValue() === "printf hidden-command-body",
  "quick-command close discarded an unsaved draft after cancellation");
  await quickCommandDraftPage.evaluate(() => {
    window.confirm = (message) => {
      window.__quickCommandDiscardPrompts.push(String(message));
      return true;
    };
  });
  await quickCommandDialog.getByRole("button", { name: "取消", exact: true }).click();
  await quickCommandDialog.waitFor({ state: "detached" });
  const quickCommandDiscardPrompts = await quickCommandDraftPage.evaluate(() => {
    window.confirm = window.__originalQuickCommandConfirm;
    return window.__quickCommandDiscardPrompts;
  });
  assert(quickCommandDiscardPrompts.length === 2
    && quickCommandDiscardPrompts.every((prompt) => prompt.includes("未保存的更改"))
    && quickCommandDiscardPrompts.every((prompt) => !prompt.includes("Unsaved private command") && !prompt.includes("hidden-command-body")),
  `quick-command discard confirmation was missing or exposed command text: ${JSON.stringify(quickCommandDiscardPrompts)}`);
  await quickCommandDraftPage.getByRole("button", { name: "工具", exact: true }).click();
  await quickCommandDraftPage.getByRole("button", { name: "快速命令", exact: true }).click();
  const reopenedQuickCommandDialog = quickCommandDraftPage.locator(".quick-command-dialog");
  await reopenedQuickCommandDialog.waitFor();
  assert(await reopenedQuickCommandDialog.locator('[role="option"]', { hasText: "Unsaved private command" }).count() === 0,
    "discarded quick-command draft was unexpectedly persisted");
  await reopenedQuickCommandDialog.getByRole("button", { name: "关闭快速命令", exact: true }).click();
  assert(quickCommandDraftErrors.length === 0,
    `quick-command draft browser exceptions: ${JSON.stringify(quickCommandDraftErrors)}`);
  await quickCommandDraftPage.close();

  const hostKeyLifecyclePage = await context.newPage();
  const hostKeyLifecycleErrors = [];
  hostKeyLifecyclePage.on("pageerror", (error) => hostKeyLifecycleErrors.push(error.message));
  await hostKeyLifecyclePage.goto(appUrl);
  await hostKeyLifecyclePage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await hostKeyLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await hostKeyLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const firstKeyManager = hostKeyLifecyclePage.locator(".key-dialog");
  await firstKeyManager.waitFor();
  await firstKeyManager.locator('.dialog-field', { hasText: "known_hosts:" }).locator("textarea").fill("first.example ssh-ed25519 AAAAFIRST");
  const firstHostKeyImportBaseline = await hostKeyLifecyclePage.evaluate(() => {
    window.__deferHostKeyMutations = true;
    return window.__invokeCalls.filter((call) => call.command === "import_known_hosts").length;
  });
  const firstHostKeyImport = firstKeyManager.locator(".key-actions").getByRole("button", { name: "导入", exact: true });
  await firstHostKeyImport.evaluate((button) => {
    button.click();
    button.click();
  });
  await hostKeyLifecyclePage.waitForFunction(() => window.__pendingHostKeyMutations.length === 1);
  const firstHostKeyImportState = await hostKeyLifecyclePage.evaluate((baseline) => ({
    pending: window.__pendingHostKeyMutations.length,
    calls: window.__invokeCalls.filter((call) => call.command === "import_known_hosts").length - baseline,
  }), firstHostKeyImportBaseline);
  assert(firstHostKeyImportState.pending === 1
    && firstHostKeyImportState.calls === 1
    && await firstHostKeyImport.isDisabled(),
  `known_hosts import submitted duplicate writes: ${JSON.stringify(firstHostKeyImportState)}`);
  assert(await firstKeyManager.locator('.dialog-field', { hasText: "known_hosts:" }).locator("textarea").isDisabled(),
    "known_hosts editor remained mutable while an import was pending");
  await hostKeyLifecyclePage.evaluate(() => {
    window.__keyManagerClosePrompts = [];
    window.__originalKeyManagerConfirm = window.confirm;
    window.confirm = (message) => {
      window.__keyManagerClosePrompts.push(String(message));
      return true;
    };
  });
  await firstKeyManager.getByRole("button", { name: "关闭密钥管理器", exact: true }).click();
  await firstKeyManager.waitFor({ state: "detached" });
  await hostKeyLifecyclePage.evaluate(() => {
    const pending = window.__pendingHostKeyMutations.shift();
    pending.resolve(pending.result);
  });
  await hostKeyLifecyclePage.waitForTimeout(100);

  await hostKeyLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await hostKeyLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const secondKeyManager = hostKeyLifecyclePage.locator(".key-dialog");
  await secondKeyManager.waitFor();
  await secondKeyManager.locator(".key-row", { hasText: "first.example:22" }).waitFor();
  await secondKeyManager.locator('.dialog-field', { hasText: "known_hosts:" }).locator("textarea").fill("second.example ssh-ed25519 AAAASECOND");
  await secondKeyManager.locator(".key-actions").getByRole("button", { name: "导入", exact: true }).click();
  await hostKeyLifecyclePage.waitForFunction(() => window.__pendingHostKeyMutations.length === 1);
  await secondKeyManager.getByRole("button", { name: "关闭密钥管理器", exact: true }).click();
  await secondKeyManager.waitFor({ state: "detached" });
  const keyManagerClosePrompts = await hostKeyLifecyclePage.evaluate(() => {
    window.confirm = window.__originalKeyManagerConfirm;
    return window.__keyManagerClosePrompts;
  });
  assert(keyManagerClosePrompts.length === 2
    && keyManagerClosePrompts.every((prompt) => prompt.includes("known_hosts 导入内容"))
    && keyManagerClosePrompts.every((prompt) => !prompt.includes("AAAAFIRST") && !prompt.includes("AAAASECOND")),
  `key-manager close confirmation was missing or exposed known_hosts contents: ${JSON.stringify(keyManagerClosePrompts)}`);

  await hostKeyLifecyclePage.evaluate(() => { window.__deferHostKeyMutations = false; });
  await hostKeyLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await hostKeyLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const thirdKeyManager = hostKeyLifecyclePage.locator(".key-dialog");
  await thirdKeyManager.waitFor();
  await thirdKeyManager.locator('.dialog-field', { hasText: "known_hosts:" }).locator("textarea").fill("third.example ssh-ed25519 AAAATHIRD");
  await thirdKeyManager.locator(".key-actions").getByRole("button", { name: "导入", exact: true }).click();
  await thirdKeyManager.locator(".key-row", { hasText: "first.example:22" }).waitFor();
  await thirdKeyManager.locator(".key-row", { hasText: "second.example:22" }).waitFor();
  await thirdKeyManager.locator(".key-row", { hasText: "third.example:22" }).waitFor();
  await hostKeyLifecyclePage.evaluate(() => {
    const pending = window.__pendingHostKeyMutations.shift();
    pending.resolve(pending.result);
  });
  await hostKeyLifecyclePage.waitForTimeout(100);
  const firstHostKeyRow = thirdKeyManager.locator(".key-row", { hasText: "first.example:22" });
  await firstHostKeyRow.getByRole("button", { name: "编辑", exact: true }).click();
  const hostKeyEditPanel = thirdKeyManager.locator(".key-edit-panel");
  await hostKeyEditPanel.locator(".dialog-field", { hasText: "Label:" }).locator("input").fill("Operator label");
  const secondHostKeyRow = thirdKeyManager.locator(".key-row", { hasText: "second.example:22" });
  await hostKeyLifecyclePage.evaluate(() => {
    window.__hostKeyDraftPrompts = [];
    window.__originalHostKeyDraftConfirm = window.confirm;
    window.confirm = (message) => {
      window.__hostKeyDraftPrompts.push(String(message));
      return false;
    };
  });
  await secondHostKeyRow.getByRole("button", { name: "编辑", exact: true }).click();
  assert(await hostKeyEditPanel.locator(".dialog-field", { hasText: "Alias:" }).locator("input").inputValue() === "first.example"
    && await hostKeyEditPanel.locator(".dialog-field", { hasText: "Label:" }).locator("input").inputValue() === "Operator label",
  "Host Key editor discarded an unsaved draft after switch cancellation");
  await hostKeyLifecyclePage.evaluate(() => {
    window.confirm = (message) => {
      window.__hostKeyDraftPrompts.push(String(message));
      return true;
    };
  });
  await secondHostKeyRow.getByRole("button", { name: "编辑", exact: true }).click();
  await firstHostKeyRow.getByRole("button", { name: "编辑", exact: true }).click();
  const hostKeyDraftPrompts = await hostKeyLifecyclePage.evaluate(() => {
    window.confirm = window.__originalHostKeyDraftConfirm;
    return window.__hostKeyDraftPrompts;
  });
  assert(hostKeyDraftPrompts.length === 2
    && hostKeyDraftPrompts.every((prompt) => prompt.includes("切换 Host Key"))
    && hostKeyDraftPrompts.every((prompt) => !prompt.includes("Operator label")),
  `Host Key draft confirmation was missing or exposed draft values: ${JSON.stringify(hostKeyDraftPrompts)}`);
  await hostKeyEditPanel.locator(".dialog-field", { hasText: "Label:" }).locator("input").fill("Operator label");
  await hostKeyLifecyclePage.evaluate(() => { window.__deferHostKeyMutations = true; });
  await hostKeyEditPanel.getByRole("button", { name: "保存编辑", exact: true }).click();
  await hostKeyLifecyclePage.waitForFunction(() => window.__pendingHostKeyMutations.length === 1);
  assert(await hostKeyEditPanel.locator(".dialog-field", { hasText: "Label:" }).locator("input").isDisabled()
    && await secondHostKeyRow.getByRole("button", { name: "编辑", exact: true }).isDisabled(),
  "Host Key editor remained mutable while an update was pending");
  await hostKeyLifecyclePage.evaluate(() => {
    window.__deferHostKeyMutations = false;
    const pending = window.__pendingHostKeyMutations.shift();
    pending.resolve(pending.result);
  });
  await hostKeyEditPanel.waitFor({ state: "detached" });
  const hostKeyLifecycleState = await hostKeyLifecyclePage.evaluate(() => {
    const updates = window.__invokeCalls.filter((call) => call.command === "update_host_key");
    return {
      backend: window.__hostKeys.map((key) => key.alias),
      visible: [...document.querySelectorAll(".key-row > strong")].map((item) => item.textContent),
      pending: window.__pendingHostKeyMutations.length,
      importCalls: window.__invokeCalls.filter((call) => call.command === "import_known_hosts").length,
      updateCalls: updates.length,
      expectedAliases: updates.map((call) => call.args.request.expectedKey?.alias ?? null),
    };
  });
  assert(JSON.stringify(hostKeyLifecycleState.backend) === JSON.stringify(["first.example", "second.example", "third.example"])
    && JSON.stringify(hostKeyLifecycleState.visible) === JSON.stringify(["first.example:22", "second.example:22", "third.example:22"])
    && hostKeyLifecycleState.pending === 0
    && hostKeyLifecycleState.importCalls === 3
    && hostKeyLifecycleState.updateCalls === 1
    && JSON.stringify(hostKeyLifecycleState.expectedAliases) === JSON.stringify(["first.example"]),
  `a stale host-key mutation replaced the latest manager state: ${JSON.stringify(hostKeyLifecycleState)}`);

  await thirdKeyManager.getByRole("button", { name: "扫描", exact: true }).click();
  const hostKeyScanResult = thirdKeyManager.locator(".host-key-scan-result");
  await hostKeyScanResult.waitFor();
  assert((await hostKeyScanResult.textContent()).includes("尚未信任此 Host Key")
    && (await hostKeyScanResult.textContent()).includes("SHA256:scan-first")
    && (await thirdKeyManager.locator(".host-key-scan-panel > header").textContent()).includes("10.0.0.1:2222"),
  "unknown Host Key scan did not expose the target and observed fingerprint");
  const firstHostKeyTrust = hostKeyScanResult.getByRole("button", { name: "加入 Profile", exact: true });
  const firstHostKeyTrustBaseline = await hostKeyLifecyclePage.evaluate(() => {
    window.__deferSessionValidation = true;
    window.__pendingSessionValidation = [];
    return window.__invokeCalls.filter((call) => call.command === "trust_scanned_host_key").length;
  });
  await firstHostKeyTrust.evaluate((button) => {
    button.click();
    button.click();
  });
  await hostKeyLifecyclePage.waitForFunction(() => window.__pendingSessionValidation.length === 1);
  const firstHostKeyTrustState = await hostKeyLifecyclePage.evaluate((baseline) => ({
    pending: window.__pendingSessionValidation.length,
    calls: window.__invokeCalls.filter((call) => call.command === "trust_scanned_host_key").length - baseline,
  }), firstHostKeyTrustBaseline);
  assert(firstHostKeyTrustState.pending === 1
    && firstHostKeyTrustState.calls === 1
    && await firstHostKeyTrust.isDisabled(),
  `Host Key scan trust submitted duplicate writes: ${JSON.stringify(firstHostKeyTrustState)}`);
  await hostKeyLifecyclePage.evaluate(() => {
    window.__deferSessionValidation = false;
    window.__pendingSessionValidation.shift().resolve();
  });
  await thirdKeyManager.locator(".key-row", { hasText: "edge-router:2222" }).waitFor();
  await hostKeyLifecyclePage.evaluate(() => { window.__hostKeyScanMode = "mismatch"; });
  await thirdKeyManager.getByRole("button", { name: "扫描", exact: true }).click();
  await hostKeyScanResult.waitFor();
  const mismatchScanText = await hostKeyScanResult.textContent();
  assert(mismatchScanText.includes("Host Key 与已保存记录不一致")
    && mismatchScanText.includes("SHA256:scan-second")
    && mismatchScanText.includes("SHA256:scan-first"),
  `Host Key mismatch comparison is incomplete: ${mismatchScanText}`);
  await hostKeyLifecyclePage.screenshot({ path: `${screenshotPrefix}-host-key-scan.png`, fullPage: true });
  await hostKeyScanResult.getByRole("button", { name: "替换 Profile", exact: true }).click();
  await hostKeyLifecyclePage.waitForFunction(() => (
    window.__hostKeys.filter((key) => key.alias === "edge-router").length === 1
      && window.__hostKeys.find((key) => key.alias === "edge-router")?.fingerprintSha256 === "SHA256:scan-second"
  ));
  const hostKeyScanState = await hostKeyLifecyclePage.evaluate(() => ({
    scanCalls: window.__invokeCalls.filter((call) => call.command === "scan_ssh_host_key").length,
    decisions: window.__invokeCalls
      .filter((call) => call.command === "trust_scanned_host_key")
      .map((call) => call.args.request.decision),
    saved: window.__hostKeys
      .filter((key) => key.alias === "edge-router")
      .map((key) => ({ fingerprint: key.fingerprintSha256, lastSeen: key.lastSeen })),
  }));
  assert(hostKeyScanState.scanCalls === 2
    && JSON.stringify(hostKeyScanState.decisions) === JSON.stringify(["append-to-profile", "replace-for-profile"])
    && hostKeyScanState.saved.length === 1
    && hostKeyScanState.saved[0].fingerprint === "SHA256:scan-second"
    && !Number.isNaN(Date.parse(hostKeyScanState.saved[0].lastSeen)),
  `Host Key scan trust/replace lifecycle is wrong: ${JSON.stringify(hostKeyScanState)}`);
  const hostKeyProfileCopy = firstHostKeyRow.getByRole("button", { name: "复制到 Profile", exact: true });
  const hostKeyProfileCopyBaseline = await hostKeyLifecyclePage.evaluate(() => {
    window.__deferSessionProfileSaves = true;
    window.__pendingSessionProfileSaves = [];
    return window.__invokeCalls.filter((call) => call.command === "save_session_profile").length;
  });
  await hostKeyProfileCopy.evaluate((button) => {
    button.click();
    button.click();
  });
  await hostKeyLifecyclePage.waitForFunction(() => window.__pendingSessionProfileSaves.length === 1);
  assert(await hostKeyProfileCopy.isDisabled(),
    "a pending key-manager Profile copy remained actionable");
  await hostKeyLifecyclePage.evaluate(() => {
    window.__deferSessionProfileSaves = false;
    window.__pendingSessionProfileSaves.shift().resolve();
  });
  await thirdKeyManager.getByText("已复制 1 个 host key 到 Edge Router", { exact: true }).waitFor();
  const hostKeyProfileCopyState = await hostKeyLifecyclePage.evaluate((baseline) => ({
    pending: window.__pendingSessionProfileSaves.length,
    saveCalls: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length - baseline,
  }), hostKeyProfileCopyBaseline);
  assert(hostKeyProfileCopyState.pending === 0 && hostKeyProfileCopyState.saveCalls === 1,
    `key-manager Profile copy submitted duplicate writes: ${JSON.stringify(hostKeyProfileCopyState)}`);
  assert(hostKeyLifecycleErrors.length === 0,
    `host-key lifecycle browser exceptions: ${JSON.stringify(hostKeyLifecycleErrors)}`);
  await hostKeyLifecyclePage.close();

  const profileLifecyclePage = await context.newPage();
  const profileLifecycleErrors = [];
  profileLifecyclePage.on("pageerror", (error) => profileLifecycleErrors.push(error.message));
  await profileLifecyclePage.goto(appUrl);
  await profileLifecyclePage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await profileLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await profileLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const firstProfileManager = profileLifecyclePage.locator(".key-dialog");
  await firstProfileManager.waitFor();
  await firstProfileManager.getByRole("button", { name: "编辑 Initial identity", exact: true }).click();
  await firstProfileManager.locator(".client-key-inspector label", { hasText: "Label" }).locator("input").fill("Closed identity");
  await profileLifecyclePage.evaluate(() => { window.__deferProfileMutations = true; });
  await firstProfileManager.getByRole("button", { name: "保存字段", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await profileLifecyclePage.waitForFunction(() => window.__pendingProfileMutations.length === 1);
  assert(await firstProfileManager.locator(".client-key-inspector label", { hasText: "Label" }).locator("input").isDisabled()
    && await firstProfileManager.locator(".client-key-edit-button").first().isDisabled(),
  "Identity editor remained mutable while a save was pending");
  await profileLifecyclePage.evaluate(() => {
    window.__identityClosePrompts = [];
    window.__originalIdentityCloseConfirm = window.confirm;
    window.confirm = (message) => {
      window.__identityClosePrompts.push(String(message));
      return true;
    };
  });
  await firstProfileManager.getByRole("button", { name: "关闭密钥管理器", exact: true }).click();
  await firstProfileManager.waitFor({ state: "detached" });
  await profileLifecyclePage.evaluate(() => {
    const pending = window.__pendingProfileMutations.shift();
    pending.resolve(pending.result);
  });
  await profileLifecyclePage.waitForTimeout(100);

  await profileLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await profileLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const secondProfileManager = profileLifecyclePage.locator(".key-dialog");
  await secondProfileManager.getByRole("button", { name: "编辑 Closed identity", exact: true }).waitFor();
  await secondProfileManager.getByRole("button", { name: "编辑 Closed identity", exact: true }).click();
  await secondProfileManager.locator(".client-key-inspector label", { hasText: "Label" }).locator("input").fill("Deferred identity");
  await secondProfileManager.getByRole("button", { name: "保存字段", exact: true }).click();
  await profileLifecyclePage.waitForFunction(() => window.__pendingProfileMutations.length === 1);
  await secondProfileManager.getByRole("button", { name: "关闭密钥管理器", exact: true }).click();
  await secondProfileManager.waitFor({ state: "detached" });
  const identityClosePrompts = await profileLifecyclePage.evaluate(() => {
    window.confirm = window.__originalIdentityCloseConfirm;
    return window.__identityClosePrompts;
  });
  assert(identityClosePrompts.length === 2
    && identityClosePrompts.every((prompt) => prompt.includes("Identity 草稿"))
    && identityClosePrompts.every((prompt) => !prompt.includes("Closed identity") && !prompt.includes("Deferred identity")),
  `Identity close confirmation was missing or exposed draft values: ${JSON.stringify(identityClosePrompts)}`);

  await profileLifecyclePage.evaluate(() => { window.__deferProfileMutations = false; });
  await profileLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await profileLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const thirdProfileManager = profileLifecyclePage.locator(".key-dialog");
  await thirdProfileManager.getByRole("button", { name: "编辑 Closed identity", exact: true }).waitFor();
  await thirdProfileManager.getByRole("button", { name: "编辑 Closed identity", exact: true }).click();
  await thirdProfileManager.locator(".client-key-inspector label", { hasText: "Label" }).locator("input").fill("Current identity");
  await thirdProfileManager.locator(".client-key-inspector label", { hasText: "Path / Agent comment" }).locator("input").fill("/home/operator/.ssh/id_ed25519 ");
  await thirdProfileManager.getByRole("button", { name: "保存字段", exact: true }).click();
  await thirdProfileManager.getByRole("button", { name: "编辑 Current identity", exact: true }).waitFor();
  await profileLifecyclePage.evaluate(() => {
    const pending = window.__pendingProfileMutations.shift();
    pending.resolve(pending.result);
  });
  await profileLifecyclePage.waitForTimeout(100);
  const profileLifecycleState = await profileLifecyclePage.evaluate(() => {
    const edge = window.__sessions.find((session) => session.profile.id === "edge-router");
    const updates = window.__invokeCalls.filter((call) => call.command === "update_client_identity");
    return {
      backend: edge.profile.connection.identityRefs.map((identity) => identity.label),
      visible: [...document.querySelectorAll(".client-key-row .client-key-main > strong")].map((item) => item.textContent),
      pending: window.__pendingProfileMutations.length,
      updateCalls: updates.length,
      expectedLabels: updates.map((call) => call.args.request.expectedIdentity?.label ?? null),
      latestPath: updates.at(-1)?.args.request.path ?? null,
    };
  });
  assert(JSON.stringify(profileLifecycleState.backend) === JSON.stringify(["Current identity"])
    && JSON.stringify(profileLifecycleState.visible) === JSON.stringify(["Current identity"])
    && profileLifecycleState.pending === 0
    && profileLifecycleState.updateCalls === 3
    && profileLifecycleState.latestPath === "/home/operator/.ssh/id_ed25519 "
    && JSON.stringify(profileLifecycleState.expectedLabels) === JSON.stringify([
      "Initial identity", "Closed identity", "Closed identity",
    ]),
  `a stale Profile mutation replaced the latest identity state: ${JSON.stringify(profileLifecycleState)}`);
  const currentIdentityLabel = thirdProfileManager.locator(".client-key-inspector label", { hasText: "Label" }).locator("input");
  await currentIdentityLabel.fill("Unsaved identity label");
  await profileLifecyclePage.evaluate(() => {
    window.__identityDraftPrompts = [];
    window.__originalIdentityDraftConfirm = window.confirm;
    window.confirm = (message) => {
      window.__identityDraftPrompts.push(String(message));
      return false;
    };
  });
  await thirdProfileManager.getByRole("button", { name: "关闭 identity 检查器", exact: true }).click();
  assert(await currentIdentityLabel.inputValue() === "Unsaved identity label"
    && await thirdProfileManager.locator(".client-key-inspector").isVisible(),
  "Identity inspector discarded an unsaved draft after close cancellation");
  await currentIdentityLabel.fill("Current identity");
  await thirdProfileManager.getByRole("button", { name: "关闭 identity 检查器", exact: true }).click();
  await thirdProfileManager.locator(".client-key-inspector").waitFor({ state: "detached" });
  const identityDraftPrompts = await profileLifecyclePage.evaluate(() => {
    window.confirm = window.__originalIdentityDraftConfirm;
    return window.__identityDraftPrompts;
  });
  assert(identityDraftPrompts.length === 1
    && identityDraftPrompts[0].includes("关闭检查器")
    && !identityDraftPrompts[0].includes("Unsaved identity label"),
  `Identity draft confirmation was missing or exposed draft values: ${JSON.stringify(identityDraftPrompts)}`);
  assert(profileLifecycleErrors.length === 0,
    `Profile lifecycle browser exceptions: ${JSON.stringify(profileLifecycleErrors)}`);
  await profileLifecyclePage.close();

  const deletedProfileMutationPage = await context.newPage();
  const deletedProfileMutationErrors = [];
  deletedProfileMutationPage.on("pageerror", (error) => deletedProfileMutationErrors.push(error.message));
  await deletedProfileMutationPage.goto(appUrl);
  await deletedProfileMutationPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await deletedProfileMutationPage.getByRole("button", { name: "工具", exact: true }).click();
  await deletedProfileMutationPage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const deletedProfileMutationManager = deletedProfileMutationPage.locator(".key-dialog");
  await deletedProfileMutationManager.getByRole("button", { name: "编辑 Initial identity", exact: true }).click();
  await deletedProfileMutationManager.locator(".client-key-inspector label", { hasText: "Label" }).locator("input").fill("Deleted Profile identity");
  await deletedProfileMutationPage.evaluate(() => { window.__deferProfileMutations = true; });
  await deletedProfileMutationManager.getByRole("button", { name: "保存字段", exact: true }).click();
  await deletedProfileMutationPage.waitForFunction(() => window.__pendingProfileMutations.length === 1);
  await deletedProfileMutationPage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
  });
  await deletedProfileMutationPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  await deletedProfileMutationPage.evaluate(() => {
    window.__deferProfileMutations = false;
    const pending = window.__pendingProfileMutations.shift();
    pending.resolve(pending.result);
  });
  await deletedProfileMutationPage.waitForTimeout(100);
  const deletedProfileMutationState = await deletedProfileMutationPage.evaluate(() => ({
    visibleProfiles: [...document.querySelectorAll(".tree-session")].map((item) => item.textContent),
    status: document.querySelector(".key-dialog .utility-status")?.textContent ?? "",
    pending: window.__pendingProfileMutations.length,
  }));
  assert(!deletedProfileMutationState.visibleProfiles.some((name) => name?.includes("Edge Router"))
    && !deletedProfileMutationState.status.includes("Client identity 已更新")
    && deletedProfileMutationState.pending === 0,
  `a key-manager mutation response restored a deleted Profile: ${JSON.stringify(deletedProfileMutationState)}`);
  assert(deletedProfileMutationErrors.length === 0,
    `deleted Profile mutation browser exceptions: ${JSON.stringify(deletedProfileMutationErrors)}`);
  await deletedProfileMutationPage.close();

  const privateKeyImportLifecyclePage = await context.newPage();
  const privateKeyImportLifecycleErrors = [];
  privateKeyImportLifecyclePage.on("pageerror", (error) => privateKeyImportLifecycleErrors.push(error.message));
  await privateKeyImportLifecyclePage.goto(appUrl);
  await privateKeyImportLifecyclePage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await privateKeyImportLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await privateKeyImportLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const importingKeyManager = privateKeyImportLifecyclePage.locator(".key-dialog");
  await importingKeyManager.getByLabel("新建 Stronghold 主密码", { exact: true }).fill("private key import vault");
  await importingKeyManager.getByRole("button", { name: "解锁 portable vault", exact: true }).click();
  await importingKeyManager.locator(".portable-vault-bar small", { hasText: "Unlocked" }).waitFor();
  const importPanel = importingKeyManager.locator(".key-import-panel");
  await importPanel.locator("summary").click();
  await privateKeyImportLifecyclePage.evaluate(() => {
    window.__pendingPrivateKeyFileReads = [];
    window.__originalPrivateKeyFileText = File.prototype.text;
    File.prototype.text = function deferredPrivateKeyFileText() {
      const file = this;
      return new Promise((resolve, reject) => window.__pendingPrivateKeyFileReads.push({
        name: file.name,
        resolve: () => window.__originalPrivateKeyFileText.call(file).then(resolve, reject),
      }));
    };
  });
  const privateKeyFileInput = importPanel.locator('input[type="file"]');
  const privateKeyLabelInput = importPanel.getByPlaceholder("Key label", { exact: true });
  const privateKeyTextInput = importPanel.getByPlaceholder("粘贴 OpenSSH private key", { exact: true });
  const privateKeyImportButton = importPanel.getByRole("button", { name: "导入到 Profile", exact: true });
  await privateKeyFileInput.setInputFiles({
    name: "first.key",
    mimeType: "text/plain",
    buffer: Buffer.from("-----BEGIN OPENSSH PRIVATE KEY-----\nfirst-key-body\n-----END OPENSSH PRIVATE KEY-----"),
  });
  await privateKeyFileInput.setInputFiles({
    name: "second.key",
    mimeType: "text/plain",
    buffer: Buffer.from("-----BEGIN OPENSSH PRIVATE KEY-----\nsecond-key-body\n-----END OPENSSH PRIVATE KEY-----"),
  });
  await privateKeyImportLifecyclePage.waitForFunction(() => window.__pendingPrivateKeyFileReads.length === 2);
  assert(await privateKeyFileInput.inputValue() === ""
    && await privateKeyFileInput.isEnabled()
    && await privateKeyLabelInput.isDisabled()
    && await privateKeyTextInput.isDisabled()
    && await privateKeyTextInput.getAttribute("maxlength") === String(1024 * 1024)
    && await privateKeyImportButton.isDisabled()
    && await importPanel.getAttribute("aria-busy") === "true",
  "a pending private-key file read did not lock stale import fields or reset the file input");
  await privateKeyImportLifecyclePage.evaluate(() => window.__pendingPrivateKeyFileReads
    .find((pending) => pending.name === "second.key").resolve());
  await privateKeyImportLifecyclePage.waitForFunction(() => document.querySelector('.key-import-panel textarea')?.value.includes("second-key-body"));
  assert(await privateKeyTextInput.isEnabled()
    && await privateKeyImportButton.isEnabled()
    && await importPanel.getAttribute("aria-busy") === "false",
  "private-key import controls did not recover after the latest file read");
  await privateKeyImportLifecyclePage.evaluate(() => window.__pendingPrivateKeyFileReads
    .find((pending) => pending.name === "first.key").resolve());
  await privateKeyImportLifecyclePage.waitForTimeout(100);
  const privateKeyFileReadState = await privateKeyImportLifecyclePage.evaluate(() => {
    File.prototype.text = window.__originalPrivateKeyFileText;
    return {
      text: document.querySelector('.key-import-panel textarea')?.value ?? "",
      status: document.querySelector(".key-dialog .utility-status")?.textContent ?? "",
    };
  });
  assert(privateKeyFileReadState.text.includes("second-key-body")
    && !privateKeyFileReadState.text.includes("first-key-body")
    && privateKeyFileReadState.status.includes("second.key"),
  `an older private-key file read replaced the latest selection: ${JSON.stringify(privateKeyFileReadState)}`);
  await importPanel.getByPlaceholder("Key label", { exact: true }).fill("Deferred imported key");
  await importPanel.getByPlaceholder("粘贴 OpenSSH private key", { exact: true }).fill([
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "test-key-body",
    "-----END OPENSSH PRIVATE KEY-----",
  ].join("\n"));
  await privateKeyImportLifecyclePage.evaluate(() => { window.__deferSecretWrites = true; });
  await importPanel.getByRole("button", { name: "导入到 Profile", exact: true }).click();
  await privateKeyImportLifecyclePage.waitForFunction(() => window.__pendingSecretWrites.length === 1);
  assert(await importPanel.getByPlaceholder("粘贴 OpenSSH private key", { exact: true }).isDisabled(),
    "private-key import editor remained mutable while a secret write was pending");
  await privateKeyImportLifecyclePage.evaluate(() => {
    window.__privateKeyClosePrompts = [];
    window.__originalPrivateKeyCloseConfirm = window.confirm;
    window.confirm = (message) => {
      window.__privateKeyClosePrompts.push(String(message));
      return true;
    };
  });
  await importingKeyManager.getByRole("button", { name: "关闭密钥管理器", exact: true }).click();
  await importingKeyManager.waitFor({ state: "detached" });
  const privateKeyClosePrompts = await privateKeyImportLifecyclePage.evaluate(() => {
    window.confirm = window.__originalPrivateKeyCloseConfirm;
    return window.__privateKeyClosePrompts;
  });
  assert(privateKeyClosePrompts.length === 1
    && privateKeyClosePrompts[0].includes("私钥导入内容")
    && !privateKeyClosePrompts[0].includes("test-key-body")
    && !privateKeyClosePrompts[0].includes("private key import vault"),
  `private-key close confirmation was missing or exposed secret values: ${JSON.stringify(privateKeyClosePrompts)}`);

  await privateKeyImportLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await privateKeyImportLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const currentKeyManager = privateKeyImportLifecyclePage.locator(".key-dialog");
  await currentKeyManager.getByRole("button", { name: "编辑 Initial identity", exact: true }).click();
  await currentKeyManager.locator(".client-key-inspector label", { hasText: "Label" }).locator("input").fill("Current identity");
  await currentKeyManager.getByRole("button", { name: "保存字段", exact: true }).click();
  await currentKeyManager.getByRole("button", { name: "编辑 Current identity", exact: true }).waitFor();

  await privateKeyImportLifecyclePage.evaluate(() => {
    window.__deferSecretWrites = false;
    window.__pendingSecretWrites.shift().resolve();
  });
  await privateKeyImportLifecyclePage.waitForFunction(() => (
    window.__invokeCalls.filter((call) => call.command === "delete_secret").length === 1
  ));
  const privateKeyImportLifecycleState = await privateKeyImportLifecyclePage.evaluate(() => {
    const edge = window.__sessions.find((session) => session.profile.id === "edge-router");
    return {
      backend: edge.profile.connection.identityRefs.map((identity) => identity.label),
      visible: [...document.querySelectorAll(".client-key-row .client-key-main > strong")].map((item) => item.textContent),
      retainedSecrets: Object.keys(window.__secrets),
      pending: window.__pendingSecretWrites.length,
      profileSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
      secretSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_secret").length,
      secretDeleteCalls: window.__invokeCalls.filter((call) => call.command === "delete_secret").length,
    };
  });
  assert(JSON.stringify(privateKeyImportLifecycleState.backend) === JSON.stringify(["Current identity"])
    && JSON.stringify(privateKeyImportLifecycleState.visible) === JSON.stringify(["Current identity"])
    && privateKeyImportLifecycleState.retainedSecrets.length === 0
    && privateKeyImportLifecycleState.pending === 0
    && privateKeyImportLifecycleState.profileSaveCalls === 0
    && privateKeyImportLifecycleState.secretSaveCalls === 1
    && privateKeyImportLifecycleState.secretDeleteCalls === 1,
  `a late private-key import overwrote the current Profile or leaked its secret: ${JSON.stringify(privateKeyImportLifecycleState)}`);
  assert(privateKeyImportLifecycleErrors.length === 0,
    `private-key import lifecycle browser exceptions: ${JSON.stringify(privateKeyImportLifecycleErrors)}`);
  await privateKeyImportLifecyclePage.close();

  const migrationOperationPage = await context.newPage();
  const migrationOperationErrors = [];
  migrationOperationPage.on("pageerror", (error) => migrationOperationErrors.push(error.message));
  await migrationOperationPage.goto(appUrl);
  await migrationOperationPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await migrationOperationPage.getByRole("button", { name: "工具", exact: true }).click();
  await migrationOperationPage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const migrationManager = migrationOperationPage.locator(".key-dialog");
  await migrationManager.getByLabel("新建 Stronghold 主密码", { exact: true }).fill("migration test vault");
  await migrationManager.getByRole("button", { name: "解锁 portable vault", exact: true }).click();
  await migrationManager.locator(".portable-vault-bar small", { hasText: "Unlocked" }).waitFor();
  await migrationManager.locator("details.portable-vault-migration").evaluate((details) => { details.open = true; });
  const migrationPreviewButton = migrationManager.locator(".portable-vault-migration-preview-button");
  const migrationPreviewBaseline = await migrationOperationPage.evaluate(() => {
    window.__deferMigrationPreviews = true;
    return window.__invokeCalls.filter((call) => call.command === "preview_profile_secret_migration").length;
  });
  await migrationPreviewButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await migrationOperationPage.waitForFunction(() => window.__pendingMigrationPreviews.length === 1);
  const pendingMigrationPreviewState = await migrationOperationPage.evaluate((baseline) => ({
    calls: window.__invokeCalls.filter((call) => call.command === "preview_profile_secret_migration").length - baseline,
    pending: window.__pendingMigrationPreviews.length,
    profileIds: window.__pendingMigrationPreviews[0]?.args.request.profileIds ?? [],
  }), migrationPreviewBaseline);
  assert(pendingMigrationPreviewState.calls === 1
    && pendingMigrationPreviewState.pending === 1
    && JSON.stringify(pendingMigrationPreviewState.profileIds) === JSON.stringify(["edge-router"])
    && await migrationPreviewButton.isDisabled(),
  `migration preview submitted duplicate reads: ${JSON.stringify(pendingMigrationPreviewState)}`);
  await migrationOperationPage.evaluate(() => {
    const index = window.__sessions.findIndex((session) => session.profile.id === "edge-router");
    const updated = structuredClone(window.__sessions[index]);
    updated.profile.connection.passwordSecretRef = "stronghold:concurrent-password";
    window.__sessions[index] = updated;
    window.__emitTauriEvent("portmate-session-profile-updated", updated);
  });
  await migrationManager.getByRole("button", { name: "锁定 portable vault", exact: true }).waitFor({ state: "visible" });
  assert(await migrationManager.getByRole("button", { name: "锁定 portable vault", exact: true }).isEnabled(),
    "a stale migration preview kept the credential operation gate locked after its Profile credentials changed");
  await migrationOperationPage.evaluate(() => {
    window.__pendingMigrationPreviews.shift().resolve();
  });
  await migrationOperationPage.waitForTimeout(100);
  assert(await migrationManager.locator(".portable-vault-migration-preview").count() === 0,
    "a late migration preview restored a plan for an obsolete Profile credential snapshot");

  await migrationManager.getByRole("button", { name: "关闭密钥管理器", exact: true }).click();
  await migrationManager.waitFor({ state: "detached" });
  await migrationOperationPage.evaluate(() => {
    const now = new Date().toISOString();
    window.__migrationRecovery = {
      migrationId: "89b35790-6b62-4ca1-a81f-678c30bf8428",
      state: "source-cleanup-pending",
      disposition: "committed",
      targetStorage: "portable",
      cleanupSource: true,
      profileCount: 1,
      secretCount: 1,
      requiresPortableVaultUnlock: false,
      canRecover: true,
      message: "Pending source cleanup",
      createdAt: now,
      updatedAt: now,
    };
    window.__deferMigrationDiagnostics = true;
  });
  await migrationOperationPage.getByRole("button", { name: "工具", exact: true }).click();
  await migrationOperationPage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const diagnosticManager = migrationOperationPage.locator(".key-dialog");
  const diagnosticButton = diagnosticManager.locator(".portable-vault-migration-diagnostic-button");
  await diagnosticButton.waitFor();
  const diagnosticBaseline = await migrationOperationPage.evaluate(() => (
    window.__invokeCalls.filter((call) => call.command === "export_profile_secret_migration_diagnostics").length
  ));
  await diagnosticButton.evaluate((button) => {
    button.click();
    button.click();
  });
  await migrationOperationPage.waitForFunction(() => window.__pendingMigrationDiagnostics.length === 1);
  const pendingDiagnosticState = await migrationOperationPage.evaluate((baseline) => ({
    calls: window.__invokeCalls.filter((call) => call.command === "export_profile_secret_migration_diagnostics").length - baseline,
    pending: window.__pendingMigrationDiagnostics.length,
  }), diagnosticBaseline);
  assert(pendingDiagnosticState.calls === 1
    && pendingDiagnosticState.pending === 1
    && await diagnosticButton.isDisabled(),
  `migration diagnostics submitted duplicate exports: ${JSON.stringify(pendingDiagnosticState)}`);
  await migrationOperationPage.evaluate(() => window.__pendingMigrationDiagnostics.shift().resolve());
  await diagnosticManager.locator(".portable-vault-migration-diagnostic-result").waitFor();
  assert((await diagnosticManager.locator(".portable-vault-migration-diagnostic-result").textContent()).includes("portmate-migration-diagnostic.json"),
    "migration diagnostic result did not recover after the guarded export completed");
  assert(migrationOperationErrors.length === 0,
    `migration operation browser exceptions: ${JSON.stringify(migrationOperationErrors)}`);
  await migrationOperationPage.close();

  const partialCredentialPage = await context.newPage();
  const partialCredentialErrors = [];
  partialCredentialPage.on("pageerror", (error) => partialCredentialErrors.push(error.message));
  await partialCredentialPage.goto(appUrl);
  await partialCredentialPage.getByRole("button", { name: "断开 Edge Router", exact: true }).click();
  const partialCredentialConnect = partialCredentialPage.getByRole("button", { name: "连接 Edge Router", exact: true });
  await partialCredentialConnect.waitFor();
  await partialCredentialConnect.click();
  const partialCredentialDialog = partialCredentialPage.locator(".credential-dialog");
  await partialCredentialDialog.waitFor();
  await partialCredentialDialog.getByLabel("登录密码", { exact: true }).fill("saved password");
  await partialCredentialDialog.getByLabel("保存登录密码到 Stronghold（需先解锁）", { exact: true }).check();
  await partialCredentialDialog.getByLabel("私钥口令", { exact: true }).fill("saved passphrase");
  await partialCredentialDialog.getByLabel("保存私钥口令到 Stronghold（需先解锁）", { exact: true }).check();
  await partialCredentialPage.evaluate(() => { window.__failSecretWriteAt = 2; });
  await partialCredentialDialog.getByRole("button", { name: "连接", exact: true }).click();
  const partialCredentialNotice = partialCredentialPage.locator(".notice-dialog", { hasText: "保存凭据失败" });
  await partialCredentialNotice.waitFor();
  const partialCredentialState = await partialCredentialPage.evaluate(() => ({
    retainedSecrets: Object.keys(window.__secrets),
    secretSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_secret").length,
    secretDeleteCalls: window.__invokeCalls.filter((call) => call.command === "delete_secret").length,
    profileSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
  }));
  assert(partialCredentialState.retainedSecrets.length === 0
    && partialCredentialState.secretSaveCalls === 2
    && partialCredentialState.secretDeleteCalls === 1
    && partialCredentialState.profileSaveCalls === 0,
  `a partial connection credential write leaked its first Secret: ${JSON.stringify(partialCredentialState)}`);
  assert(partialCredentialErrors.length === 0,
    `partial connection credential browser exceptions: ${JSON.stringify(partialCredentialErrors)}`);
  await partialCredentialPage.close();

  const failedProfileCredentialPage = await context.newPage();
  const failedProfileCredentialErrors = [];
  failedProfileCredentialPage.on("pageerror", (error) => failedProfileCredentialErrors.push(error.message));
  await failedProfileCredentialPage.goto(appUrl);
  await failedProfileCredentialPage.getByRole("button", { name: "断开 Edge Router", exact: true }).click();
  const failedProfileConnect = failedProfileCredentialPage.getByRole("button", { name: "连接 Edge Router", exact: true });
  await failedProfileConnect.waitFor();
  await failedProfileConnect.click();
  const failedProfileCredentialDialog = failedProfileCredentialPage.locator(".credential-dialog");
  await failedProfileCredentialDialog.waitFor();
  await failedProfileCredentialDialog.getByLabel("登录密码", { exact: true }).fill("saved password");
  await failedProfileCredentialDialog.getByLabel("保存登录密码到 Stronghold（需先解锁）", { exact: true }).check();
  await failedProfileCredentialPage.evaluate(() => { window.__failNextProfileSave = true; });
  await failedProfileCredentialDialog.getByRole("button", { name: "连接", exact: true }).click();
  const failedProfileCredentialNotice = failedProfileCredentialPage.locator(".notice-dialog", { hasText: "连接失败" });
  await failedProfileCredentialNotice.waitFor();
  const failedProfileCredentialState = await failedProfileCredentialPage.evaluate(() => {
    const edge = window.__sessions.find((session) => session.profile.id === "edge-router");
    return {
      backendPasswordRef: edge.profile.connection.passwordSecretRef ?? null,
      retainedSecrets: Object.keys(window.__secrets),
      secretSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_secret").length,
      secretDeleteCalls: window.__invokeCalls.filter((call) => call.command === "delete_secret").length,
      profileSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
      openCalls: window.__invokeCalls.filter((call) => call.command === "open_session").length,
    };
  });
  assert(failedProfileCredentialState.backendPasswordRef === null
    && failedProfileCredentialState.retainedSecrets.length === 0
    && failedProfileCredentialState.secretSaveCalls === 1
    && failedProfileCredentialState.secretDeleteCalls === 1
    && failedProfileCredentialState.profileSaveCalls === 1
    && failedProfileCredentialState.openCalls === 0,
  `a failed Profile save retained connection credentials: ${JSON.stringify(failedProfileCredentialState)}`);
  const failedProfileHealth = await failedProfileCredentialPage
    .getByRole("button", { name: "连接 Edge Router", exact: true })
    .getAttribute("title");
  assert(failedProfileHealth?.includes("连接错误")
    && failedProfileHealth.includes("simulated Profile save failure"),
  `connection failure health lost its actual reason: ${failedProfileHealth}`);
  await failedProfileCredentialNotice.getByRole("button", { name: "确定", exact: true }).click();
  await failedProfileCredentialPage.getByRole("button", { name: "连接 Edge Router", exact: true }).click();
  const retryCredentialDialog = failedProfileCredentialPage.locator(".credential-dialog");
  await retryCredentialDialog.waitFor();
  assert(await retryCredentialDialog.getByText("登录密码", { exact: true }).count() === 1
    && await retryCredentialDialog.getByText("登录密码(已存)", { exact: true }).count() === 0,
  "a failed Profile save left the deleted password ref in current frontend state");
  await retryCredentialDialog.getByRole("button", { name: "取消", exact: true }).click();
  assert(failedProfileCredentialErrors.length === 0,
    `failed Profile credential browser exceptions: ${JSON.stringify(failedProfileCredentialErrors)}`);
  await failedProfileCredentialPage.close();

  const connectionLifecyclePage = await context.newPage();
  const connectionLifecycleErrors = [];
  connectionLifecyclePage.on("pageerror", (error) => connectionLifecycleErrors.push(error.message));
  await connectionLifecyclePage.goto(appUrl);
  const lifecycleSession = connectionLifecyclePage.locator(".workspace-dock-content.panel-explorer .tree-session", { hasText: "Local Shell" });
  await lifecycleSession.click();
  await connectionLifecyclePage.getByRole("button", { name: "断开 Local Shell", exact: true }).click();
  const lifecycleConnect = connectionLifecyclePage.getByRole("button", { name: "连接 Local Shell", exact: true });
  await lifecycleConnect.waitFor();
  const connectionLifecycleBaseline = await connectionLifecyclePage.evaluate(() => {
    window.__deferSessionOpens = true;
    return {
      saves: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
      opens: window.__invokeCalls.filter((call) => call.command === "open_session").length,
    };
  });
  await lifecycleConnect.click();
  await connectionLifecyclePage.waitForFunction(() => window.__pendingSessionOpens.length === 1);
  await connectionLifecyclePage.getByRole("button", { name: "断开 Local Shell", exact: true }).waitFor();

  await lifecycleSession.dispatchEvent("contextmenu", { clientX: 120, clientY: 160 });
  const sessionContextMenu = connectionLifecyclePage.locator(".portmate-context-menu:not(.workspace-view-context-menu):not(.terminal-context-menu)");
  await sessionContextMenu.waitFor();
  const sessionReconnect = sessionContextMenu.locator("button", { hasText: "重新连接会话(R)" });
  const sessionDisconnect = sessionContextMenu.locator("button", { hasText: "断开会话(C)" });
  assert(await sessionReconnect.isDisabled() && !await sessionDisconnect.isDisabled(),
    "session context menu exposed reconnect or hid disconnect during a pending connection");
  await connectionLifecyclePage.mouse.click(700, 400);
  await sessionContextMenu.waitFor({ state: "detached" });

  const lifecycleTab = connectionLifecyclePage.locator(".workspace-pane-tab", { hasText: "Local Shell" }).first();
  await lifecycleTab.dispatchEvent("contextmenu", { clientX: 240, clientY: 80 });
  await connectionLifecyclePage.locator(".workspace-view-context-menu").waitFor();
  const viewReconnect = connectionLifecyclePage.getByRole("button", { name: "重新连接会话", exact: true });
  assert(await viewReconnect.isDisabled(),
    "view context menu exposed reconnect during a pending connection");
  await connectionLifecyclePage.mouse.click(700, 400);
  await connectionLifecyclePage.locator(".workspace-view-context-menu").waitFor({ state: "detached" });

  await connectionLifecyclePage.getByRole("button", { name: "断开 Local Shell", exact: true }).click();
  await lifecycleConnect.waitFor();
  await connectionLifecyclePage.evaluate(() => {
    const pending = window.__pendingSessionOpens.shift();
    pending.resolve(pending.result);
  });
  await connectionLifecyclePage.waitForTimeout(150);
  const staleConnectionState = await connectionLifecyclePage.evaluate(() => ({
    backend: window.__sessions.find((session) => session.profile.id === "local-shell")?.runtime.status ?? "missing",
    pending: window.__pendingSessionOpens.length,
    saves: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
    opens: window.__invokeCalls.filter((call) => call.command === "open_session").length,
  }));
  assert(staleConnectionState.backend === "disconnected"
    && staleConnectionState.pending === 0
    && staleConnectionState.saves === connectionLifecycleBaseline.saves + 1
    && staleConnectionState.opens === connectionLifecycleBaseline.opens + 1
    && await lifecycleConnect.isVisible(),
  `a late connection response restored a manually disconnected session: ${JSON.stringify(staleConnectionState)}`);

  await lifecycleConnect.evaluate((button) => {
    button.click();
    button.click();
  });
  await connectionLifecyclePage.waitForFunction(() => window.__pendingSessionOpens.length === 1);
  const duplicateConnectionState = await connectionLifecyclePage.evaluate(() => ({
    pending: window.__pendingSessionOpens.length,
    saves: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
    opens: window.__invokeCalls.filter((call) => call.command === "open_session").length,
  }));
  assert(duplicateConnectionState.pending === 1
    && duplicateConnectionState.saves === connectionLifecycleBaseline.saves + 2
    && duplicateConnectionState.opens === connectionLifecycleBaseline.opens + 2,
  `duplicate connection triggers escaped the per-session gate: ${JSON.stringify(duplicateConnectionState)}`);
  await connectionLifecyclePage.evaluate(() => {
    const pending = window.__pendingSessionOpens.shift();
    const index = window.__sessions.findIndex((session) => session.profile.id === "local-shell");
    window.__sessions[index] = structuredClone(pending.result);
    window.__deferSessionOpens = false;
    pending.resolve(structuredClone(pending.result));
  });
  await connectionLifecyclePage.getByRole("button", { name: "断开 Local Shell", exact: true }).waitFor();
  await connectionLifecyclePage.waitForTimeout(100);
  const reconnectLifecycleStart = await connectionLifecyclePage.evaluate(() => {
    window.__deferSessionOpens = true;
    window.__deferSessionCloses = true;
    window.__pendingSessionCloses = [];
    return window.__invokeCalls.length;
  });
  await lifecycleSession.dispatchEvent("contextmenu", { clientX: 120, clientY: 160 });
  const reconnectContextMenu = connectionLifecyclePage.locator(".portmate-context-menu:not(.workspace-view-context-menu):not(.terminal-context-menu)");
  await reconnectContextMenu.waitFor();
  const reconnectAction = reconnectContextMenu.locator("button", { hasText: "重新连接会话(R)" });
  assert(!await reconnectAction.isDisabled(), "connected session context menu disabled reconnect");
  await reconnectAction.evaluate((button) => {
    button.click();
    button.click();
  });
  await connectionLifecyclePage.waitForFunction(() => window.__pendingSessionCloses.length === 1);
  const pendingReconnectClose = await connectionLifecyclePage.evaluate((start) => ({
    closes: window.__invokeCalls.slice(start).filter((call) => call.command === "close_session").length,
    pending: window.__pendingSessionCloses.length,
  }), reconnectLifecycleStart);
  const pendingDisconnectButton = connectionLifecyclePage.getByRole("button", { name: "正在断开 Local Shell", exact: true });
  assert(pendingReconnectClose.closes === 1
    && pendingReconnectClose.pending === 1
    && await pendingDisconnectButton.isDisabled()
    && await pendingDisconnectButton.getAttribute("aria-busy") === "true",
  `pending reconnect did not serialize or lock disconnect controls: ${JSON.stringify(pendingReconnectClose)}`);
  await connectionLifecyclePage.getByRole("button", { name: "会话", exact: true }).click();
  assert(await connectionLifecyclePage.getByRole("button", { name: "启动会话", exact: true }).isDisabled()
    && await connectionLifecyclePage.getByRole("button", { name: "关闭会话", exact: true }).isDisabled(),
  "top-level session actions remained enabled during a pending disconnect");
  await connectionLifecyclePage.getByRole("button", { name: "会话", exact: true }).click();
  await connectionLifecyclePage.locator(".menu-popover").waitFor({ state: "detached" });
  await lifecycleSession.click({ button: "right" });
  const pendingSessionMenu = connectionLifecyclePage.locator(".portmate-context-menu:not(.workspace-view-context-menu):not(.terminal-context-menu)");
  await pendingSessionMenu.waitFor();
  assert(await pendingSessionMenu.locator("button", { hasText: "重新连接会话(R)" }).isDisabled()
    && await pendingSessionMenu.locator("button", { hasText: "断开会话(C)" }).isDisabled(),
  "session context actions remained enabled during a pending disconnect");
  await connectionLifecyclePage.mouse.click(700, 400);
  await lifecycleTab.click({ button: "right" });
  const pendingViewMenu = connectionLifecyclePage.locator(".workspace-view-context-menu");
  await pendingViewMenu.waitFor();
  assert(await pendingViewMenu.getByRole("button", { name: "重新连接会话", exact: true }).isDisabled(),
    "view reconnect remained enabled during a pending disconnect");
  await connectionLifecyclePage.mouse.click(700, 400);
  await connectionLifecyclePage.evaluate(() => {
    window.__deferSessionCloses = false;
    window.__pendingSessionCloses.shift().resolve();
  });
  await connectionLifecyclePage.waitForFunction(() => window.__pendingSessionOpens.length === 1);
  const reconnectHealth = await connectionLifecyclePage
    .getByRole("button", { name: "断开 Local Shell", exact: true })
    .getAttribute("title");
  const reconnectLifecycle = await connectionLifecyclePage.evaluate((start) => ({
    calls: window.__invokeCalls.slice(start)
      .filter((call) => ["close_session", "save_session_profile", "open_session"].includes(call.command))
      .map((call) => ({
        command: call.command,
        sessionId: call.args.request?.sessionId ?? call.args.sessionId ?? call.args.profile?.id ?? "",
      })),
    status: window.__sessions.find((session) => session.profile.id === "local-shell")?.runtime.status ?? "missing",
  }), reconnectLifecycleStart);
  assert(JSON.stringify(reconnectLifecycle.calls) === JSON.stringify([
    { command: "close_session", sessionId: "local-shell" },
    { command: "save_session_profile", sessionId: "local-shell" },
    { command: "open_session", sessionId: "local-shell" },
  ])
    && reconnectLifecycle.status === "connecting"
    && reconnectHealth?.includes("正在连接")
    && reconnectHealth.includes("user closed session"),
  `context reconnect did not preserve the authoritative disconnect summary: ${JSON.stringify({ reconnectLifecycle, reconnectHealth })}`);
  await connectionLifecyclePage.evaluate(() => {
    const pending = window.__pendingSessionOpens.shift();
    const index = window.__sessions.findIndex((session) => session.profile.id === "local-shell");
    window.__sessions[index] = structuredClone(pending.result);
    window.__deferSessionOpens = false;
    pending.resolve(structuredClone(pending.result));
  });
  await connectionLifecyclePage.getByRole("button", { name: "断开 Local Shell", exact: true }).waitFor();
  assert(connectionLifecycleErrors.length === 0,
    `connection lifecycle browser exceptions: ${JSON.stringify(connectionLifecycleErrors)}`);
  await connectionLifecyclePage.close();

  const hostKeyPromptPage = await context.newPage();
  const hostKeyPromptErrors = [];
  hostKeyPromptPage.on("pageerror", (error) => hostKeyPromptErrors.push(error.message));
  await hostKeyPromptPage.goto(appUrl);
  await hostKeyPromptPage.getByRole("button", { name: "断开 Edge Router", exact: true }).click();
  const hostKeyPromptConnect = hostKeyPromptPage.getByRole("button", { name: "连接 Edge Router", exact: true });
  await hostKeyPromptConnect.waitFor();
  await hostKeyPromptPage.evaluate(() => {
    window.__sessionOpenErrors["edge-router"] = "SSH Host Key 已变化: simulated mismatch";
    window.__deferSessionValidation = true;
    window.__pendingSessionValidation = [];
  });
  await hostKeyPromptConnect.click();
  const hostKeyCredentialDialog = hostKeyPromptPage.locator(".credential-dialog");
  await hostKeyCredentialDialog.waitFor();
  await hostKeyCredentialDialog.getByRole("button", { name: "连接", exact: true }).click();
  await hostKeyPromptPage.waitForFunction(() => window.__pendingSessionValidation.length === 1);
  const hostKeyPromptDialog = hostKeyPromptPage.locator(".hostkey-dialog");
  await hostKeyPromptDialog.waitFor();
  assert((await hostKeyPromptDialog.textContent()).includes("扫描中"),
    "Host Key prompt did not expose its pending scan state");
  await hostKeyPromptPage.evaluate(() => window.__pendingSessionValidation.shift().resolve());
  const trustAndReconnect = hostKeyPromptDialog.getByRole("button", { name: "加入 Profile 并重连", exact: true });
  await trustAndReconnect.waitFor();
  const hostKeyTrustBaseline = await hostKeyPromptPage.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "trust_scanned_host_key").length);
  await trustAndReconnect.evaluate((button) => {
    button.click();
    button.click();
  });
  await hostKeyPromptPage.waitForFunction(() => window.__pendingSessionValidation.length === 1);
  const hostKeyPromptPendingState = await hostKeyPromptPage.evaluate((baseline) => ({
    pending: window.__pendingSessionValidation.length,
    trustCalls: window.__invokeCalls.filter((call) => call.command === "trust_scanned_host_key").length - baseline,
  }), hostKeyTrustBaseline);
  assert(hostKeyPromptPendingState.pending === 1
    && hostKeyPromptPendingState.trustCalls === 1
    && await trustAndReconnect.isDisabled()
    && await hostKeyPromptDialog.getByRole("button", { name: "仅本次并重连", exact: true }).isDisabled()
    && await hostKeyPromptDialog.getByRole("button", { name: "替换 Profile 并重连", exact: true }).isDisabled()
    && await hostKeyPromptDialog.getByRole("button", { name: "拒绝", exact: true }).isDisabled(),
  `Host Key prompt submitted duplicate or conflicting decisions: ${JSON.stringify(hostKeyPromptPendingState)}`);
  await hostKeyPromptPage.evaluate(() => {
    delete window.__sessionOpenErrors["edge-router"];
    window.__deferSessionValidation = false;
    window.__pendingSessionValidation.shift().resolve();
  });
  await hostKeyPromptDialog.waitFor({ state: "detached" });
  const hostKeyPromptFinalState = await hostKeyPromptPage.evaluate((baseline) => ({
    trustCalls: window.__invokeCalls.filter((call) => call.command === "trust_scanned_host_key").length - baseline,
    trustedKeys: window.__hostKeys.filter((key) => key.profileId === "edge-router").length,
  }), hostKeyTrustBaseline);
  assert(hostKeyPromptFinalState.trustCalls === 1 && hostKeyPromptFinalState.trustedKeys === 1,
    `Host Key prompt did not commit exactly one trust decision: ${JSON.stringify(hostKeyPromptFinalState)}`);
  const hostKeyNotice = hostKeyPromptPage.locator(".notice-dialog", { hasText: "Host key 已确认" });
  await hostKeyNotice.waitFor();
  await hostKeyNotice.getByRole("button", { name: "确定", exact: true }).click();
  const reconnectCredentialDialog = hostKeyPromptPage.locator(".credential-dialog");
  if (await reconnectCredentialDialog.count()) {
    await reconnectCredentialDialog.getByRole("button", { name: "取消", exact: true }).click();
  }
  assert(hostKeyPromptErrors.length === 0,
    `Host Key prompt lifecycle browser exceptions: ${JSON.stringify(hostKeyPromptErrors)}`);
  await hostKeyPromptPage.close();

  const deletedHostKeyDecisionPage = await context.newPage();
  const deletedHostKeyDecisionErrors = [];
  deletedHostKeyDecisionPage.on("pageerror", (error) => deletedHostKeyDecisionErrors.push(error.message));
  await deletedHostKeyDecisionPage.goto(appUrl);
  await deletedHostKeyDecisionPage.getByRole("button", { name: "断开 Edge Router", exact: true }).click();
  const deletedHostKeyConnect = deletedHostKeyDecisionPage.getByRole("button", { name: "连接 Edge Router", exact: true });
  await deletedHostKeyDecisionPage.evaluate(() => {
    window.__sessionOpenErrors["edge-router"] = "SSH Host Key 已变化: simulated mismatch";
    window.__deferSessionValidation = true;
    window.__pendingSessionValidation = [];
  });
  await deletedHostKeyConnect.click();
  await deletedHostKeyDecisionPage.locator(".credential-dialog").getByRole("button", { name: "连接", exact: true }).click();
  await deletedHostKeyDecisionPage.waitForFunction(() => window.__pendingSessionValidation.length === 1);
  const deletedHostKeyDialog = deletedHostKeyDecisionPage.locator(".hostkey-dialog");
  await deletedHostKeyDialog.waitFor();
  await deletedHostKeyDecisionPage.evaluate(() => window.__pendingSessionValidation.shift().resolve());
  const deletedHostKeyDecision = deletedHostKeyDialog.getByRole("button", { name: "加入 Profile 并重连", exact: true });
  await deletedHostKeyDecision.waitFor();
  const deletedHostKeyTrustBaseline = await deletedHostKeyDecisionPage.evaluate(() => window.__invokeCalls
    .filter((call) => call.command === "trust_scanned_host_key").length);
  await deletedHostKeyDecision.evaluate((button) => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
    button.click();
  });
  await deletedHostKeyDialog.waitFor({ state: "detached" });
  await deletedHostKeyDecisionPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  const deletedHostKeyDecisionState = await deletedHostKeyDecisionPage.evaluate((baseline) => ({
    profiles: window.__sessions.map((session) => session.profile.id),
    trustCalls: window.__invokeCalls.filter((call) => call.command === "trust_scanned_host_key").length - baseline,
    credentialDialogs: document.querySelectorAll(".credential-dialog").length,
    settingsDialogs: document.querySelectorAll(".session-settings-dialog").length,
  }), deletedHostKeyTrustBaseline);
  assert(!deletedHostKeyDecisionState.profiles.includes("edge-router")
    && deletedHostKeyDecisionState.trustCalls === 0
    && deletedHostKeyDecisionState.credentialDialogs === 0
    && deletedHostKeyDecisionState.settingsDialogs === 0,
  `a stale Host Key decision survived Profile deletion: ${JSON.stringify(deletedHostKeyDecisionState)}`);
  assert(deletedHostKeyDecisionErrors.length === 0,
    `deleted Host Key decision browser exceptions: ${JSON.stringify(deletedHostKeyDecisionErrors)}`);
  await deletedHostKeyDecisionPage.close();

  const deletedHostKeySettingsPage = await context.newPage();
  const deletedHostKeySettingsErrors = [];
  deletedHostKeySettingsPage.on("pageerror", (error) => deletedHostKeySettingsErrors.push(error.message));
  await deletedHostKeySettingsPage.goto(appUrl);
  await deletedHostKeySettingsPage.getByRole("button", { name: "断开 Edge Router", exact: true }).click();
  const deletedHostKeySettingsConnect = deletedHostKeySettingsPage.getByRole("button", { name: "连接 Edge Router", exact: true });
  await deletedHostKeySettingsPage.evaluate(() => {
    window.__sessionOpenErrors["edge-router"] = "SSH Host Key 已变化: simulated mismatch";
    window.__deferSessionValidation = true;
    window.__pendingSessionValidation = [];
  });
  await deletedHostKeySettingsConnect.click();
  await deletedHostKeySettingsPage.locator(".credential-dialog").getByRole("button", { name: "连接", exact: true }).click();
  await deletedHostKeySettingsPage.waitForFunction(() => window.__pendingSessionValidation.length === 1);
  const deletedHostKeySettingsDialog = deletedHostKeySettingsPage.locator(".hostkey-dialog");
  const deletedHostKeySettings = deletedHostKeySettingsDialog.getByRole("button", { name: "打开验证设置", exact: true });
  await deletedHostKeySettings.waitFor();
  await deletedHostKeySettings.evaluate((button) => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
    button.click();
    window.__deferSessionValidation = false;
    window.__pendingSessionValidation.shift().resolve();
  });
  await deletedHostKeySettingsDialog.waitFor({ state: "detached" });
  await deletedHostKeySettingsPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  await deletedHostKeySettingsPage.waitForTimeout(100);
  const deletedHostKeySettingsState = await deletedHostKeySettingsPage.evaluate(() => ({
    profiles: window.__sessions.map((session) => session.profile.id),
    pendingScans: window.__pendingSessionValidation.length,
    credentialDialogs: document.querySelectorAll(".credential-dialog").length,
    settingsDialogs: document.querySelectorAll(".session-settings-dialog").length,
  }));
  assert(!deletedHostKeySettingsState.profiles.includes("edge-router")
    && deletedHostKeySettingsState.pendingScans === 0
    && deletedHostKeySettingsState.credentialDialogs === 0
    && deletedHostKeySettingsState.settingsDialogs === 0,
  `stale Host Key settings opened after Profile deletion: ${JSON.stringify(deletedHostKeySettingsState)}`);
  assert(deletedHostKeySettingsErrors.length === 0,
    `deleted Host Key settings browser exceptions: ${JSON.stringify(deletedHostKeySettingsErrors)}`);
  await deletedHostKeySettingsPage.close();

  const sessionOperationPage = await context.newPage();
  const sessionOperationErrors = [];
  sessionOperationPage.on("pageerror", (error) => sessionOperationErrors.push(error.message));
  await sessionOperationPage.goto(appUrl);
  await sessionOperationPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await sessionOperationPage.getByRole("button", { name: "会话", exact: true }).click();
  await sessionOperationPage.getByRole("button", { name: "会话设置", exact: true }).click();
  let sessionOperationDialog = sessionOperationPage.locator(".session-settings-dialog");
  await sessionOperationDialog.waitFor();
  const sessionSaveBaseline = await sessionOperationPage.evaluate(() => {
    window.__deferSessionProfileSaves = true;
    window.__pendingSessionProfileSaves = [];
    return window.__invokeCalls.filter((call) => call.command === "save_session_profile").length;
  });
  await sessionOperationDialog.evaluate((dialog) => {
    const buttons = [...dialog.querySelectorAll("button")];
    buttons.find((button) => button.textContent?.trim() === "保存")?.click();
    buttons.find((button) => button.textContent?.trim() === "保存并连接")?.click();
    buttons.find((button) => button.textContent?.trim() === "取消")?.click();
  });
  await sessionOperationPage.waitForFunction(() => window.__pendingSessionProfileSaves.length === 1);
  assert(await sessionOperationDialog.getByRole("button", { name: "保存", exact: true }).isDisabled()
    && await sessionOperationDialog.getByRole("button", { name: "保存并连接", exact: true }).isDisabled()
    && await sessionOperationDialog.getByRole("button", { name: "取消", exact: true }).isDisabled()
    && await sessionOperationDialog.locator(".dialog-title button").isDisabled()
    && await sessionOperationDialog.locator(".session-form").evaluate((form) => form.inert),
  "a pending Session Settings save did not lock its draft and conflicting actions");
  await sessionOperationPage.evaluate(() => {
    window.__deferSessionProfileSaves = false;
    window.__pendingSessionProfileSaves.shift().resolve();
  });
  await sessionOperationDialog.waitFor({ state: "detached" });
  const sessionSaveCalls = await sessionOperationPage.evaluate((baseline) => (
    window.__invokeCalls.filter((call) => call.command === "save_session_profile").length - baseline
  ), sessionSaveBaseline);
  assert(sessionSaveCalls === 1, `Session Settings submitted a duplicate save: ${sessionSaveCalls}`);

  await sessionOperationPage.getByRole("button", { name: "会话", exact: true }).click();
  await sessionOperationPage.getByRole("button", { name: "会话设置", exact: true }).click();
  sessionOperationDialog = sessionOperationPage.locator(".session-settings-dialog");
  await sessionOperationDialog.getByRole("combobox", { name: "会话配置项", exact: true }).selectOption("验证");
  await sessionOperationPage.evaluate(() => {
    window.__deferSessionValidation = true;
    window.__pendingSessionValidation = [];
  });
  const deferredHealth = sessionOperationDialog.getByRole("button", { name: "检查 SSH 健康", exact: true });
  await deferredHealth.evaluate((button) => {
    button.click();
    button.click();
  });
  await sessionOperationPage.waitForFunction(() => window.__pendingSessionValidation.length === 1);
  assert(await deferredHealth.isDisabled(), "a pending SSH health check remained actionable");
  await sessionOperationPage.evaluate(() => window.__pendingSessionValidation.shift().resolve());
  await sessionOperationDialog.getByText("健康 · russh · 公钥 · SSH 7 ms · Channel 11 ms · SFTP 13 ms", { exact: true }).waitFor();

  const deferredHostKeyScan = sessionOperationDialog.getByRole("button", { name: "扫描 Host Key", exact: true });
  await deferredHostKeyScan.evaluate((button) => {
    button.click();
    button.click();
  });
  await sessionOperationPage.waitForFunction(() => window.__pendingSessionValidation.length === 1);
  assert(await sessionOperationDialog.getByRole("button", { name: "扫描中", exact: true }).isDisabled(),
    "a pending Host Key scan remained actionable");
  await sessionOperationDialog.getByRole("combobox", { name: "会话配置项", exact: true }).selectOption("SSH");
  await sessionOperationDialog.getByLabel("主机:(H)", { exact: true }).fill("new-router.local");
  await sessionOperationDialog.getByRole("combobox", { name: "会话配置项", exact: true }).selectOption("验证");
  await sessionOperationPage.evaluate(() => window.__pendingSessionValidation.shift().resolve());
  await sessionOperationPage.waitForTimeout(100);
  assert(await sessionOperationDialog.getByRole("button", { name: "扫描 Host Key", exact: true }).count() === 1
    && !(await sessionOperationDialog.textContent()).includes("尚未信任此 Host Key"),
  "a Host Key scan for the previous target updated the changed draft");

  await sessionOperationDialog.getByRole("button", { name: "扫描 Host Key", exact: true }).click();
  await sessionOperationPage.waitForFunction(() => window.__pendingSessionValidation.length === 1);
  await sessionOperationPage.evaluate(() => window.__pendingSessionValidation.shift().resolve());
  const trustScannedHostKey = sessionOperationDialog.getByRole("button", { name: "加入 Profile", exact: true });
  await trustScannedHostKey.waitFor();
  await trustScannedHostKey.evaluate((button) => {
    button.click();
    button.click();
  });
  await sessionOperationPage.waitForFunction(() => window.__pendingSessionValidation.length === 1);
  assert(await sessionOperationDialog.locator(".dialog-title button").isDisabled()
    && await sessionOperationDialog.getByRole("button", { name: "保存", exact: true }).isDisabled()
    && await sessionOperationDialog.locator(".session-form").evaluate((form) => form.inert),
  "a pending Host Key trust write did not lock Session Settings");
  await sessionOperationPage.evaluate(() => {
    window.__deferSessionValidation = false;
    window.__pendingSessionValidation.shift().resolve();
  });
  await sessionOperationDialog.getByText(/已信任 SHA256:scan-first/).waitFor();
  const sessionValidationState = await sessionOperationPage.evaluate(() => ({
    healthCalls: window.__invokeCalls.filter((call) => call.command === "check_ssh_health").length,
    scanHosts: window.__invokeCalls
      .filter((call) => call.command === "scan_ssh_host_key")
      .map((call) => call.args.request.profile.connection.endpoint.host),
    trustCalls: window.__invokeCalls.filter((call) => call.command === "trust_scanned_host_key").length,
    trustedHosts: window.__hostKeys.map((key) => key.host),
  }));
  assert(sessionValidationState.healthCalls === 1
    && JSON.stringify(sessionValidationState.scanHosts) === JSON.stringify(["10.0.0.1", "new-router.local"])
    && sessionValidationState.trustCalls === 1
    && JSON.stringify(sessionValidationState.trustedHosts) === JSON.stringify(["new-router.local"]),
  `Session Settings validation operations were duplicated or used stale targets: ${JSON.stringify(sessionValidationState)}`);
  await sessionOperationDialog.getByRole("button", { name: "取消", exact: true }).click();
  await sessionOperationDialog.waitFor({ state: "detached" });
  assert(sessionOperationErrors.length === 0,
    `Session Settings operation lifecycle browser exceptions: ${JSON.stringify(sessionOperationErrors)}`);
  await sessionOperationPage.close();

  const deletedSettingsSavePage = await context.newPage();
  const deletedSettingsSaveErrors = [];
  deletedSettingsSavePage.on("pageerror", (error) => deletedSettingsSaveErrors.push(error.message));
  await deletedSettingsSavePage.goto(appUrl);
  await deletedSettingsSavePage.locator(".tree-session", { hasText: "Edge Router" }).click();
  await deletedSettingsSavePage.getByRole("button", { name: "会话", exact: true }).click();
  await deletedSettingsSavePage.getByRole("button", { name: "会话设置", exact: true }).click();
  const deletedSettingsSaveDialog = deletedSettingsSavePage.locator(".session-settings-dialog");
  await deletedSettingsSaveDialog.getByLabel("名称:(N)", { exact: true }).fill("Stale settings save");
  await deletedSettingsSavePage.evaluate(() => {
    window.__deferSessionProfileSaves = true;
    window.__pendingSessionProfileSaves = [];
  });
  await deletedSettingsSaveDialog.getByRole("button", { name: "保存", exact: true }).click();
  await deletedSettingsSavePage.waitForFunction(() => window.__pendingSessionProfileSaves.length === 1);
  await deletedSettingsSavePage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
  });
  await deletedSettingsSavePage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  await deletedSettingsSavePage.evaluate(() => {
    window.__deferSessionProfileSaves = false;
    window.__pendingSessionProfileSaves.shift().resolve();
  });
  await deletedSettingsSavePage.waitForTimeout(100);
  const deletedSettingsSaveState = await deletedSettingsSavePage.evaluate(() => ({
    profiles: window.__sessions.map((session) => session.profile.id),
    pending: window.__pendingSessionProfileSaves.length,
  }));
  assert(!deletedSettingsSaveState.profiles.includes("edge-router")
    && deletedSettingsSaveState.pending === 0
    && await deletedSettingsSavePage.locator(".tree-session", { hasText: "Stale settings save" }).count() === 0
    && await deletedSettingsSavePage.locator(".notice-dialog").count() === 0
    && !await deletedSettingsSaveDialog.getByRole("button", { name: "取消", exact: true }).isDisabled(),
  `a Session Settings save response restored a deleted Profile: ${JSON.stringify(deletedSettingsSaveState)}`);
  await deletedSettingsSaveDialog.getByRole("button", { name: "取消", exact: true }).click();
  await deletedSettingsSaveDialog.waitFor({ state: "detached" });
  assert(deletedSettingsSaveErrors.length === 0,
    `deleted Session Settings save browser exceptions: ${JSON.stringify(deletedSettingsSaveErrors)}`);
  await deletedSettingsSavePage.close();

  const deletedSettingsSecretPage = await context.newPage();
  const deletedSettingsSecretErrors = [];
  deletedSettingsSecretPage.on("pageerror", (error) => deletedSettingsSecretErrors.push(error.message));
  await deletedSettingsSecretPage.goto(appUrl);
  await deletedSettingsSecretPage.locator(".tree-session", { hasText: "Edge Router" }).click();
  await deletedSettingsSecretPage.getByRole("button", { name: "会话", exact: true }).click();
  await deletedSettingsSecretPage.getByRole("button", { name: "会话设置", exact: true }).click();
  const deletedSettingsSecretDialog = deletedSettingsSecretPage.locator(".session-settings-dialog");
  await deletedSettingsSecretDialog.getByLabel("会话配置项", { exact: true }).selectOption({ label: "公钥" });
  await deletedSettingsSecretDialog.locator(".dialog-field", { hasText: "公钥:(K)" }).locator("select").selectOption("profile-vault");
  await deletedSettingsSecretDialog.getByPlaceholder("粘贴 OpenSSH 私钥，保存后只保留 secretRef", { exact: true }).fill("deleted profile private key");
  await deletedSettingsSecretPage.evaluate(() => {
    window.__deferSecretWrites = true;
    window.__pendingSecretWrites = [];
  });
  await deletedSettingsSecretDialog.locator("button", { hasText: "保存到 Stronghold" }).click();
  await deletedSettingsSecretPage.waitForFunction(() => window.__pendingSecretWrites.length === 1);
  await deletedSettingsSecretPage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
  });
  await deletedSettingsSecretPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  const replacementDraftName = await deletedSettingsSecretDialog.getByLabel("名称:(N)", { exact: true }).inputValue();
  await deletedSettingsSecretPage.evaluate(() => {
    window.__deferSecretWrites = false;
    window.__pendingSecretWrites.shift().resolve();
  });
  await deletedSettingsSecretPage.waitForFunction(() => (
    Object.keys(window.__secrets).length === 0
    && window.__invokeCalls.filter((call) => call.command === "delete_secret").length === 1
  ));
  await deletedSettingsSecretPage.waitForTimeout(100);
  const deletedSettingsSecretState = await deletedSettingsSecretPage.evaluate(() => ({
    profiles: window.__sessions.map((session) => session.profile.id),
    retainedSecrets: Object.keys(window.__secrets),
    secretSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_secret").length,
    secretDeleteCalls: window.__invokeCalls.filter((call) => call.command === "delete_secret").length,
    profileSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
    pending: window.__pendingSecretWrites.length,
  }));
  assert(!deletedSettingsSecretState.profiles.includes("edge-router")
    && deletedSettingsSecretState.retainedSecrets.length === 0
    && deletedSettingsSecretState.secretSaveCalls === 1
    && deletedSettingsSecretState.secretDeleteCalls === 1
    && deletedSettingsSecretState.profileSaveCalls === 0
    && deletedSettingsSecretState.pending === 0
    && await deletedSettingsSecretDialog.getByLabel("名称:(N)", { exact: true }).inputValue() === replacementDraftName,
  `a staged Secret response for a deleted Profile leaked or restored its draft: ${JSON.stringify(deletedSettingsSecretState)}`);
  await deletedSettingsSecretDialog.getByRole("button", { name: "取消", exact: true }).click();
  await deletedSettingsSecretDialog.waitFor({ state: "detached" });
  assert(deletedSettingsSecretErrors.length === 0,
    `deleted Session Settings Secret browser exceptions: ${JSON.stringify(deletedSettingsSecretErrors)}`);
  await deletedSettingsSecretPage.close();

  const deletedCredentialPromptPage = await context.newPage();
  const deletedCredentialPromptErrors = [];
  deletedCredentialPromptPage.on("pageerror", (error) => deletedCredentialPromptErrors.push(error.message));
  await deletedCredentialPromptPage.goto(appUrl);
  await deletedCredentialPromptPage.getByRole("button", { name: "断开 Edge Router", exact: true }).click();
  const deletedCredentialConnect = deletedCredentialPromptPage.getByRole("button", { name: "连接 Edge Router", exact: true });
  await deletedCredentialConnect.waitFor();
  await deletedCredentialConnect.click();
  const deletedCredentialDialog = deletedCredentialPromptPage.locator(".credential-dialog");
  await deletedCredentialDialog.waitFor();
  await deletedCredentialPromptPage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
  });
  await deletedCredentialDialog.waitFor({ state: "detached" });
  await deletedCredentialPromptPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  const deletedCredentialPromptState = await deletedCredentialPromptPage.evaluate(() => ({
    saves: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
    opens: window.__invokeCalls.filter((call) => call.command === "open_session").length,
  }));
  assert(deletedCredentialPromptState.saves === 0
    && deletedCredentialPromptState.opens === 0
    && await deletedCredentialPromptPage.locator(".notice-dialog").count() === 0,
  `Profile deletion did not cancel its pending credential prompt: ${JSON.stringify(deletedCredentialPromptState)}`);
  assert(deletedCredentialPromptErrors.length === 0,
    `deleted credential prompt browser exceptions: ${JSON.stringify(deletedCredentialPromptErrors)}`);
  await deletedCredentialPromptPage.close();

  const credentialPromptOwnershipPage = await context.newPage();
  const credentialPromptOwnershipErrors = [];
  credentialPromptOwnershipPage.on("pageerror", (error) => credentialPromptOwnershipErrors.push(error.message));
  await credentialPromptOwnershipPage.goto(appUrl);
  await credentialPromptOwnershipPage.getByRole("button", { name: "断开 Edge Router", exact: true }).click();
  await credentialPromptOwnershipPage.getByRole("button", { name: "连接 Edge Router", exact: true }).waitFor();
  await credentialPromptOwnershipPage.evaluate(() => {
    const edge = window.__sessions.find((session) => session.profile.id === "edge-router");
    const backup = structuredClone(edge);
    backup.profile.id = "backup-router";
    backup.profile.name = "Backup Router";
    backup.profile.connection.endpoint.host = "10.0.0.2";
    backup.profile.connection.username = "backup-admin";
    backup.runtime.sessionId = "backup-router";
    backup.runtime.paneId = "backup-router:main";
    backup.runtime.status = "disconnected";
    backup.runtime.title = "Backup Router";
    backup.runtime.connectedSince = null;
    window.__sessions.push(backup);
    window.__emitTauriEvent("portmate-session-profile-updated", backup);
  });
  await credentialPromptOwnershipPage.getByRole("button", { name: "连接 Edge Router", exact: true }).click();
  const firstCredentialDialog = credentialPromptOwnershipPage.locator(".credential-dialog");
  await firstCredentialDialog.waitFor();
  await firstCredentialDialog.getByLabel("用户名", { exact: true }).fill("edge-admin");
  await firstCredentialDialog.getByLabel("登录密码", { exact: true }).fill("edge-password");
  await credentialPromptOwnershipPage.evaluate(() => {
    const dialog = document.querySelector(".credential-dialog");
    const buttons = [...dialog.querySelectorAll("button")];
    const oldSubmit = buttons.find((button) => button.textContent === "连接");
    const oldCancel = buttons.find((button) => button.textContent === "取消");
    oldSubmit.click();
    window.__emitTauriEvent("portmate-detached-pane-command", {
      action: "connect",
      requestId: "credential-ownership-connect",
      windowId: "credential-ownership-window",
      ownerWindowId: "main",
      paneId: "credential-ownership-pane",
      viewId: "credential-ownership-view",
      sessionId: "backup-router",
      title: "Backup Router",
      color: "",
      keyMode: "remote",
    });
    oldCancel.click();
  });
  const currentCredentialDialog = credentialPromptOwnershipPage.locator(".credential-dialog");
  await currentCredentialDialog.waitFor();
  assert(await currentCredentialDialog.getByLabel("用户名", { exact: true }).inputValue() === "backup-admin"
    && await currentCredentialDialog.getByLabel("登录密码", { exact: true }).inputValue() === "",
    "a stale credential dialog completed the next session's credential request");
  await currentCredentialDialog.getByRole("button", { name: "取消", exact: true }).click();
  await currentCredentialDialog.waitFor({ state: "detached" });
  await credentialPromptOwnershipPage.waitForFunction(() => (
    window.__invokeCalls.some((call) => call.command === "open_session" && call.args.request?.sessionId === "edge-router")
  ));
  const credentialPromptOwnershipState = await credentialPromptOwnershipPage.evaluate(() => ({
    edgeOpens: window.__invokeCalls.filter((call) => call.command === "open_session" && call.args.request?.sessionId === "edge-router").length,
    backupOpens: window.__invokeCalls.filter((call) => call.command === "open_session" && call.args.request?.sessionId === "backup-router").length,
    dialogs: document.querySelectorAll(".credential-dialog").length,
  }));
  assert(credentialPromptOwnershipState.edgeOpens === 1
    && credentialPromptOwnershipState.backupOpens === 0
    && credentialPromptOwnershipState.dialogs === 0,
  `credential prompt request ownership was not isolated: ${JSON.stringify(credentialPromptOwnershipState)}`);
  assert(credentialPromptOwnershipErrors.length === 0,
    `credential prompt ownership browser exceptions: ${JSON.stringify(credentialPromptOwnershipErrors)}`);
  await credentialPromptOwnershipPage.close();

  const deletedConnectionSavePage = await context.newPage();
  const deletedConnectionSaveErrors = [];
  deletedConnectionSavePage.on("pageerror", (error) => deletedConnectionSaveErrors.push(error.message));
  await deletedConnectionSavePage.goto(appUrl);
  await deletedConnectionSavePage.getByRole("button", { name: "断开 Edge Router", exact: true }).click();
  await deletedConnectionSavePage.evaluate(() => {
    window.__deferSessionProfileSaves = true;
    window.__pendingSessionProfileSaves = [];
  });
  await deletedConnectionSavePage.getByRole("button", { name: "连接 Edge Router", exact: true }).click();
  const deletedConnectionCredentialDialog = deletedConnectionSavePage.locator(".credential-dialog");
  await deletedConnectionCredentialDialog.waitFor();
  await deletedConnectionCredentialDialog.getByRole("button", { name: "连接", exact: true }).click();
  await deletedConnectionSavePage.waitForFunction(() => window.__pendingSessionProfileSaves.length === 1);
  await deletedConnectionSavePage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
  });
  await deletedConnectionSavePage.locator(".tree-session", { hasText: "Edge Router" }).waitFor({ state: "detached" });
  await deletedConnectionSavePage.evaluate(() => {
    window.__deferSessionProfileSaves = false;
    window.__pendingSessionProfileSaves.shift().resolve();
  });
  await deletedConnectionSavePage.waitForTimeout(100);
  const deletedConnectionSaveState = await deletedConnectionSavePage.evaluate(() => ({
    pending: window.__pendingSessionProfileSaves.length,
    opens: window.__invokeCalls.filter((call) => call.command === "open_session").length,
  }));
  assert(deletedConnectionSaveState.pending === 0
    && deletedConnectionSaveState.opens === 0
    && await deletedConnectionSavePage.locator(".tree-session", { hasText: "Edge Router" }).count() === 0
    && await deletedConnectionSavePage.locator(".notice-dialog").count() === 0,
  `a connection save response restored its deleted Profile: ${JSON.stringify(deletedConnectionSaveState)}`);
  assert(deletedConnectionSaveErrors.length === 0,
    `deleted connection save browser exceptions: ${JSON.stringify(deletedConnectionSaveErrors)}`);
  await deletedConnectionSavePage.close();

  const profileShortcutPage = await context.newPage();
  const profileShortcutErrors = [];
  profileShortcutPage.on("pageerror", (error) => profileShortcutErrors.push(error.message));
  await profileShortcutPage.goto(appUrl);
  const profileShortcutTarget = profileShortcutPage.locator(
    ".workspace-dock-content.panel-explorer .tree-session",
    { hasText: "Edge Router" },
  );
  await profileShortcutTarget.waitFor();
  await profileShortcutTarget.click();
  const profileShortcutSaveBaseline = await profileShortcutPage.evaluate(() => {
    window.__deferSessionProfileSaves = true;
    window.__pendingSessionProfileSaves = [];
    return window.__invokeCalls.filter((call) => call.command === "save_session_profile").length;
  });
  await profileShortcutTarget.click({ button: "right" });
  const profileShortcutSave = profileShortcutPage.locator(".context-menu-row", { hasText: "保存会话(S)" });
  await profileShortcutSave.evaluate((button) => {
    button.click();
    button.click();
  });
  await profileShortcutPage.waitForFunction(() => window.__pendingSessionProfileSaves.length === 1);
  await profileShortcutTarget.click({ button: "right" });
  const busyProfileMenu = profileShortcutPage.locator('.portmate-context-menu[aria-label="会话菜单"]');
  for (const label of ["重命名会话(R)", "保存会话(S)", "移动视图到分组(M)", "会话设置...(S)", "删除会话 Profile"]) {
    assert(await busyProfileMenu.locator(".context-menu-row", { hasText: label }).isDisabled(),
      `pending Profile shortcut left ${label} actionable`);
  }
  await profileShortcutPage.evaluate(() => {
    window.__deferSessionProfileSaves = false;
    window.__pendingSessionProfileSaves.shift().resolve();
  });
  const profileShortcutSuccess = profileShortcutPage.locator(".notice-dialog", { hasText: "已保存 Edge Router" });
  await profileShortcutSuccess.waitFor();
  const profileShortcutSaveCalls = await profileShortcutPage.evaluate((baseline) => (
    window.__invokeCalls.filter((call) => call.command === "save_session_profile").length - baseline
  ), profileShortcutSaveBaseline);
  assert(profileShortcutSaveCalls === 1,
    `Profile context shortcut submitted duplicate saves: ${profileShortcutSaveCalls}`);
  await profileShortcutSuccess.getByRole("button", { name: "确定", exact: true }).click();

  await profileShortcutPage.evaluate(() => { window.__failNextProfileSave = true; });
  await profileShortcutTarget.click({ button: "right" });
  await profileShortcutPage.locator(".context-menu-row", { hasText: "保存会话(S)" }).click();
  const profileShortcutFailure = profileShortcutPage.locator(".notice-dialog", { hasText: "保存会话失败" });
  await profileShortcutFailure.waitFor();
  assert((await profileShortcutFailure.textContent())?.includes("simulated Profile save failure"),
    "Profile shortcut failure hid the backend error");
  await profileShortcutFailure.getByRole("button", { name: "确定", exact: true }).click();

  await profileShortcutPage.evaluate(() => {
    window.__deferSessionProfileSaves = true;
    window.__pendingSessionProfileSaves = [];
  });
  await profileShortcutTarget.click({ button: "right" });
  await profileShortcutPage.locator(".context-menu-row", { hasText: "保存会话(S)" }).click();
  await profileShortcutPage.waitForFunction(() => window.__pendingSessionProfileSaves.length === 1);
  await profileShortcutPage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
  });
  await profileShortcutTarget.waitFor({ state: "detached" });
  await profileShortcutPage.evaluate(() => {
    window.__deferSessionProfileSaves = false;
    window.__pendingSessionProfileSaves.shift().resolve();
  });
  await profileShortcutPage.waitForTimeout(100);
  assert(await profileShortcutPage.locator(".tree-session", { hasText: "Edge Router" }).count() === 0,
    "a Profile shortcut response that arrived after deletion restored the deleted Profile");
  assert(profileShortcutErrors.length === 0,
    `Profile shortcut lifecycle browser exceptions: ${JSON.stringify(profileShortcutErrors)}`);
  await profileShortcutPage.close();

  const cancelledDraftSecretPage = await context.newPage();
  const cancelledDraftSecretErrors = [];
  cancelledDraftSecretPage.on("pageerror", (error) => cancelledDraftSecretErrors.push(error.message));
  await cancelledDraftSecretPage.goto(appUrl);
  await cancelledDraftSecretPage.getByRole("button", { name: "会话", exact: true }).click();
  await cancelledDraftSecretPage.getByRole("button", { name: "会话设置", exact: true }).click();
  const cancelledDraftSecretDialog = cancelledDraftSecretPage.locator(".session-settings-dialog");
  await cancelledDraftSecretDialog.getByLabel("会话配置项", { exact: true }).selectOption({ label: "公钥" });
  await cancelledDraftSecretDialog.locator(".dialog-field", { hasText: "公钥:(K)" }).locator("select").selectOption("profile-vault");
  const cancelledDraftSecretText = cancelledDraftSecretDialog.getByPlaceholder("粘贴 OpenSSH 私钥，保存后只保留 secretRef", { exact: true });
  await cancelledDraftSecretText.fill("staged private key");
  const cancelledDraftSecretSave = cancelledDraftSecretDialog.locator("button", { hasText: "保存到 Stronghold" });
  const cancelledDraftSecretControls = {
    value: await cancelledDraftSecretText.inputValue(),
    disabled: await cancelledDraftSecretSave.isDisabled(),
    errors: [...cancelledDraftSecretErrors],
  };
  assert(cancelledDraftSecretControls.value === "staged private key" && !cancelledDraftSecretControls.disabled,
    `staged Secret controls did not become actionable: ${JSON.stringify(cancelledDraftSecretControls)}`);
  await cancelledDraftSecretPage.evaluate(() => { window.__deferSecretWrites = true; });
  await cancelledDraftSecretSave.click();
  await cancelledDraftSecretPage.waitForFunction(() => window.__pendingSecretWrites.length === 1);
  assert(await cancelledDraftSecretDialog.locator(".dialog-title button").isDisabled()
    && await cancelledDraftSecretDialog.getByRole("button", { name: "保存", exact: true }).isDisabled()
    && await cancelledDraftSecretDialog.getByRole("button", { name: "保存并连接", exact: true }).isDisabled()
    && await cancelledDraftSecretDialog.getByRole("button", { name: "取消", exact: true }).isDisabled(),
  "Session Settings allowed close or submit while a staged Secret write was pending");
  await cancelledDraftSecretPage.evaluate(() => {
    window.__deferSecretWrites = false;
    window.__pendingSecretWrites.shift().resolve();
  });
  const cancelDraftSecret = cancelledDraftSecretDialog.getByRole("button", { name: "取消", exact: true });
  await cancelDraftSecret.waitFor({ state: "visible" });
  await cancelledDraftSecretPage.waitForFunction(() => !document.querySelector(".session-settings-dialog .dialog-actions button:last-child")?.disabled);
  await cancelDraftSecret.click();
  await cancelledDraftSecretDialog.waitFor({ state: "detached" });
  const cancelledDraftSecretState = await cancelledDraftSecretPage.evaluate(() => ({
    retainedSecrets: Object.keys(window.__secrets),
    secretSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_secret").length,
    secretDeleteCalls: window.__invokeCalls.filter((call) => call.command === "delete_secret").length,
    profileSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
  }));
  assert(cancelledDraftSecretState.retainedSecrets.length === 0
    && cancelledDraftSecretState.secretSaveCalls === 1
    && cancelledDraftSecretState.secretDeleteCalls === 1
    && cancelledDraftSecretState.profileSaveCalls === 0,
  `cancelling Session Settings leaked a staged Secret: ${JSON.stringify(cancelledDraftSecretState)}`);
  assert(cancelledDraftSecretErrors.length === 0,
    `cancelled staged Secret browser exceptions: ${JSON.stringify(cancelledDraftSecretErrors)}`);
  await cancelledDraftSecretPage.close();

  const committedDraftSecretPage = await context.newPage();
  const committedDraftSecretErrors = [];
  committedDraftSecretPage.on("pageerror", (error) => committedDraftSecretErrors.push(error.message));
  await committedDraftSecretPage.goto(appUrl);
  await committedDraftSecretPage.getByRole("button", { name: "会话", exact: true }).click();
  await committedDraftSecretPage.getByRole("button", { name: "会话设置", exact: true }).click();
  const committedDraftSecretDialog = committedDraftSecretPage.locator(".session-settings-dialog");
  await committedDraftSecretDialog.getByLabel("会话配置项", { exact: true }).selectOption({ label: "公钥" });
  await committedDraftSecretDialog.locator(".dialog-field", { hasText: "公钥:(K)" }).locator("select").selectOption("profile-vault");
  const committedDraftSecretText = committedDraftSecretDialog.getByPlaceholder("粘贴 OpenSSH 私钥，保存后只保留 secretRef", { exact: true });
  await committedDraftSecretText.fill("committed private key");
  const committedDraftSecretSave = committedDraftSecretDialog.locator("button", { hasText: "保存到 Stronghold" });
  assert(!await committedDraftSecretSave.isDisabled(), "committed staged Secret control remained disabled after input");
  await committedDraftSecretSave.click();
  await committedDraftSecretPage.waitForFunction(() => Object.keys(window.__secrets).length === 1);
  await committedDraftSecretDialog.getByRole("button", { name: "保存", exact: true }).click();
  await committedDraftSecretDialog.waitFor({ state: "detached" });
  const committedDraftSecretState = await committedDraftSecretPage.evaluate(() => {
    const edge = window.__sessions.find((session) => session.profile.id === "edge-router");
    return {
      backendSecretRef: edge.profile.connection.identityRefs[0]?.secretRef ?? null,
      retainedSecrets: Object.keys(window.__secrets),
      secretSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_secret").length,
      secretDeleteCalls: window.__invokeCalls.filter((call) => call.command === "delete_secret").length,
      profileSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
    };
  });
  assert(committedDraftSecretState.backendSecretRef !== null
    && committedDraftSecretState.retainedSecrets.includes(committedDraftSecretState.backendSecretRef)
    && committedDraftSecretState.retainedSecrets.length === 1
    && committedDraftSecretState.secretSaveCalls === 1
    && committedDraftSecretState.secretDeleteCalls === 0
    && committedDraftSecretState.profileSaveCalls === 1,
  `saving Session Settings deleted or lost its staged Secret: ${JSON.stringify(committedDraftSecretState)}`);
  assert(committedDraftSecretErrors.length === 0,
    `committed staged Secret browser exceptions: ${JSON.stringify(committedDraftSecretErrors)}`);
  await committedDraftSecretPage.close();

  const vaultLifecyclePage = await context.newPage();
  const vaultLifecycleErrors = [];
  vaultLifecyclePage.on("pageerror", (error) => vaultLifecycleErrors.push(error.message));
  await vaultLifecyclePage.goto(appUrl);
  await vaultLifecyclePage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await vaultLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await vaultLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const firstVaultManager = vaultLifecyclePage.locator(".key-dialog");
  await firstVaultManager.getByLabel("新建 Stronghold 主密码", { exact: true }).fill("correct horse battery staple");
  await vaultLifecyclePage.evaluate(() => { window.__deferVaultMutations = true; });
  await firstVaultManager.getByRole("button", { name: "解锁 portable vault", exact: true }).click();
  await vaultLifecyclePage.waitForFunction(() => window.__pendingVaultMutations.length === 1);
  await firstVaultManager.getByRole("button", { name: "关闭密钥管理器", exact: true }).click();
  await firstVaultManager.waitFor({ state: "detached" });

  await vaultLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await vaultLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const secondVaultManager = vaultLifecyclePage.locator(".key-dialog");
  const blockedUnlock = secondVaultManager.getByRole("button", { name: "解锁 portable vault", exact: true });
  await blockedUnlock.waitFor();
  assert(await blockedUnlock.isDisabled(), "a reopened key manager bypassed the in-flight vault operation");
  const vaultStateBeforeRelease = await vaultLifecyclePage.evaluate(() => ({
    backend: structuredClone(window.__portableVault),
    unlockCalls: window.__invokeCalls.filter((call) => call.command === "unlock_portable_vault").length,
  }));
  assert(!vaultStateBeforeRelease.backend.exists && !vaultStateBeforeRelease.backend.unlocked
    && vaultStateBeforeRelease.unlockCalls === 1,
  `the reopened manager started another vault mutation: ${JSON.stringify(vaultStateBeforeRelease)}`);

  await vaultLifecyclePage.evaluate(() => {
    window.__deferVaultMutations = false;
    window.__pendingVaultMutations.shift().resolve();
  });
  const lockVault = secondVaultManager.getByRole("button", { name: "锁定 portable vault", exact: true });
  await lockVault.waitFor();
  await lockVault.click();
  await secondVaultManager.locator(".portable-vault-bar small", { hasText: "Locked" }).waitFor();
  const vaultLifecycleState = await vaultLifecyclePage.evaluate(() => ({
    backend: structuredClone(window.__portableVault),
    pending: window.__pendingVaultMutations.length,
    unlockCalls: window.__invokeCalls.filter((call) => call.command === "unlock_portable_vault").length,
    lockCalls: window.__invokeCalls.filter((call) => call.command === "lock_portable_vault").length,
  }));
  assert(vaultLifecycleState.backend.exists && !vaultLifecycleState.backend.unlocked
    && vaultLifecycleState.pending === 0
    && vaultLifecycleState.unlockCalls === 1
    && vaultLifecycleState.lockCalls === 1,
  `the reopened manager did not converge after the vault mutation: ${JSON.stringify(vaultLifecycleState)}`);
  assert(vaultLifecycleErrors.length === 0,
    `vault lifecycle browser exceptions: ${JSON.stringify(vaultLifecycleErrors)}`);
  await vaultLifecyclePage.close();

  const screenLockVaultPage = await context.newPage();
  const screenLockVaultErrors = [];
  screenLockVaultPage.on("pageerror", (error) => screenLockVaultErrors.push(error.message));
  await screenLockVaultPage.goto(appUrl);
  await screenLockVaultPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  const screenLockVaultBaseline = await screenLockVaultPage.evaluate(() => {
    window.__portableVault = { ...window.__portableVault, exists: true, unlocked: false };
    window.__deferVaultMutations = false;
    window.__pendingVaultMutations = [];
    window.localStorage.removeItem("portmate.screenLock.v1");
    return {
      unlock: window.__invokeCalls.filter((call) => call.command === "unlock_portable_vault").length,
      lock: window.__invokeCalls.filter((call) => call.command === "lock_portable_vault").length,
    };
  });
  await screenLockVaultPage.keyboard.press("Control+Alt+L");
  const screenLockVaultOverlay = screenLockVaultPage.locator(".screen-lock-overlay");
  const screenLockPassword = screenLockVaultOverlay.getByLabel("Portable Vault 主密码", { exact: true });
  await screenLockPassword.waitFor();
  await screenLockPassword.fill("correct horse battery staple");
  await screenLockVaultPage.evaluate(() => { window.__deferVaultMutations = true; });
  const screenLockUnlock = screenLockVaultOverlay.locator("button.screen-lock-primary");
  await screenLockUnlock.evaluate((button) => {
    button.click();
    button.click();
  });
  await screenLockVaultPage.waitForFunction(() => window.__pendingVaultMutations.length === 1);
  const screenLockPendingUnlock = await screenLockVaultPage.evaluate((baseline) => ({
    unlockCalls: window.__invokeCalls.filter((call) => call.command === "unlock_portable_vault").length - baseline.unlock,
    lockCalls: window.__invokeCalls.filter((call) => call.command === "lock_portable_vault").length - baseline.lock,
    pending: window.__pendingVaultMutations.length,
  }), screenLockVaultBaseline);
  assert(screenLockPendingUnlock.unlockCalls === 1
    && screenLockPendingUnlock.lockCalls === 0
    && screenLockPendingUnlock.pending === 1
    && await screenLockUnlock.isDisabled(),
  `screen lock submitted duplicate Vault unlocks: ${JSON.stringify(screenLockPendingUnlock)}`);
  await screenLockVaultPage.evaluate(() => window.__pendingVaultMutations.shift().resolve());
  await screenLockVaultPage.waitForFunction((baseline) => window.__pendingVaultMutations.length === 1
    && window.__invokeCalls.filter((call) => call.command === "lock_portable_vault").length - baseline === 1,
  screenLockVaultBaseline.lock);
  assert(await screenLockVaultOverlay.count() === 1 && await screenLockUnlock.isDisabled(),
    "screen lock cleared before restoring the original locked Vault state");
  await screenLockVaultPage.evaluate(() => {
    window.__deferVaultMutations = false;
    window.__pendingVaultMutations.shift().resolve();
  });
  await screenLockVaultOverlay.waitFor({ state: "detached" });
  const screenLockVaultFinal = await screenLockVaultPage.evaluate((baseline) => ({
    unlocked: window.__portableVault.unlocked,
    unlockCalls: window.__invokeCalls.filter((call) => call.command === "unlock_portable_vault").length - baseline.unlock,
    lockCalls: window.__invokeCalls.filter((call) => call.command === "lock_portable_vault").length - baseline.lock,
    marker: window.localStorage.getItem("portmate.screenLock.v1"),
    pending: window.__pendingVaultMutations.length,
  }), screenLockVaultBaseline);
  assert(!screenLockVaultFinal.unlocked
    && screenLockVaultFinal.unlockCalls === 1
    && screenLockVaultFinal.lockCalls === 1
    && screenLockVaultFinal.marker === null
    && screenLockVaultFinal.pending === 0,
  `screen lock Vault lifecycle did not converge exactly once: ${JSON.stringify(screenLockVaultFinal)}`);
  assert(screenLockVaultErrors.length === 0,
    `screen lock Vault browser exceptions: ${JSON.stringify(screenLockVaultErrors)}`);
  await screenLockVaultPage.close();

  const profileRecoveryPage = await context.newPage();
  const profileRecoveryErrors = [];
  profileRecoveryPage.on("pageerror", (error) => profileRecoveryErrors.push(error.message));
  await profileRecoveryPage.goto(appUrl);
  await profileRecoveryPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await profileRecoveryPage.getByRole("button", { name: "工具", exact: true }).click();
  await profileRecoveryPage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const profileRecoveryManager = profileRecoveryPage.locator(".key-dialog");
  await profileRecoveryManager.getByRole("button", { name: "编辑 Initial identity", exact: true }).click();
  await profileRecoveryManager.locator(".client-key-inspector label", { hasText: "Label" }).locator("input").fill("Rejected identity");
  await profileRecoveryPage.evaluate(() => { window.__profileMutationFailureMode = "rename"; });
  await profileRecoveryManager.getByRole("button", { name: "保存字段", exact: true }).click();
  await profileRecoveryManager.locator(".utility-error", { hasText: "simulated conflicting Profile mutation" }).waitFor();
  await profileRecoveryPage.locator(".tree-session", { hasText: "Externally renamed router" }).waitFor();
  const renamedProfileRecoveryState = await profileRecoveryPage.evaluate(() => ({
    backend: window.__sessions.map((session) => session.profile.name),
    visible: [...document.querySelectorAll(".tree-session > span:last-child")].map((item) => item.textContent),
    listCalls: window.__invokeCalls.filter((call) => call.command === "list_sessions").length,
  }));
  assert(renamedProfileRecoveryState.backend.includes("Externally renamed router")
    && renamedProfileRecoveryState.visible.includes("Externally renamed router")
    && renamedProfileRecoveryState.listCalls >= 2,
  `a Profile compensation read ignored changed Profile fields: ${JSON.stringify(renamedProfileRecoveryState)}`);
  assert(profileRecoveryErrors.length === 0,
    `Profile compensation browser exceptions: ${JSON.stringify(profileRecoveryErrors)}`);
  await profileRecoveryPage.close();

  const emptyProfileRecoveryPage = await context.newPage();
  const emptyProfileRecoveryErrors = [];
  emptyProfileRecoveryPage.on("pageerror", (error) => emptyProfileRecoveryErrors.push(error.message));
  await emptyProfileRecoveryPage.goto(appUrl);
  await emptyProfileRecoveryPage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await emptyProfileRecoveryPage.getByRole("button", { name: "工具", exact: true }).click();
  await emptyProfileRecoveryPage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const emptyProfileRecoveryManager = emptyProfileRecoveryPage.locator(".key-dialog");
  await emptyProfileRecoveryManager.getByRole("button", { name: "编辑 Initial identity", exact: true }).click();
  await emptyProfileRecoveryManager.locator(".client-key-inspector label", { hasText: "Label" }).locator("input").fill("Deleted identity");
  await emptyProfileRecoveryPage.evaluate(() => { window.__profileMutationFailureMode = "empty"; });
  await emptyProfileRecoveryManager.getByRole("button", { name: "保存字段", exact: true }).click();
  await emptyProfileRecoveryManager.locator(".utility-error", { hasText: "simulated deleted Profile mutation" }).waitFor();
  await emptyProfileRecoveryPage.waitForFunction(() => document.querySelectorAll(".tree-session").length === 0);
  const emptyProfileRecoveryState = await emptyProfileRecoveryPage.evaluate(() => ({
    backendCount: window.__sessions.length,
    visibleCount: document.querySelectorAll(".tree-session").length,
    workspaceTabCount: document.querySelectorAll(".workspace-pane-tab").length,
    listCalls: window.__invokeCalls.filter((call) => call.command === "list_sessions").length,
  }));
  assert(emptyProfileRecoveryState.backendCount === 0
    && emptyProfileRecoveryState.visibleCount === 0
    && emptyProfileRecoveryState.workspaceTabCount === 0
    && emptyProfileRecoveryState.listCalls >= 2,
  `a Profile compensation read rejected the authoritative empty snapshot: ${JSON.stringify(emptyProfileRecoveryState)}`);
  assert(emptyProfileRecoveryErrors.length === 0,
    `empty Profile compensation browser exceptions: ${JSON.stringify(emptyProfileRecoveryErrors)}`);
  await emptyProfileRecoveryPage.close();

  const serialLaunchPage = await context.newPage();
  const serialLaunchErrors = [];
  serialLaunchPage.on("pageerror", (error) => serialLaunchErrors.push(error.message));
  await serialLaunchPage.goto(appUrl);
  await serialLaunchPage.locator(".tree-session", { hasText: "Bench UART" }).click();
  await serialLaunchPage.getByRole("button", { name: "工具", exact: true }).click();
  const serialAnalyzerAction = serialLaunchPage.getByRole("button", { name: "串口分析器", exact: true });
  await serialAnalyzerAction.waitFor();
  const serialLaunchBaseline = await serialLaunchPage.evaluate(() => {
    window.__deferChildWindowCreates = true;
    return {
      creates: window.__invokeCalls.filter((call) => call.command === "plugin:webview|create_webview_window").length,
      destroys: window.__invokeCalls.filter((call) => call.command === "plugin:window|destroy").length,
    };
  });
  await serialAnalyzerAction.evaluate((button) => {
    button.click();
    button.click();
  });
  await serialLaunchPage.waitForFunction(() => window.__pendingChildWindowCreates.length === 1);
  await serialLaunchPage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "bench-uart");
    window.__emitTauriEvent("portmate-session-profile-deleted", "bench-uart");
    window.__pendingChildWindowCreates.shift().resolve();
  });
  await serialLaunchPage.waitForFunction(({ destroys }) => (
    window.__invokeCalls.filter((call) => call.command === "plugin:window|destroy").length === destroys + 1
  ), serialLaunchBaseline);
  const serialLaunchState = await serialLaunchPage.evaluate((baseline) => ({
    creates: window.__invokeCalls.filter((call) => call.command === "plugin:webview|create_webview_window").length - baseline.creates,
    destroys: window.__invokeCalls.filter((call) => call.command === "plugin:window|destroy").length - baseline.destroys,
    sessions: window.__sessions.map((session) => session.profile.id),
    pending: window.__pendingChildWindowCreates.length,
    notices: document.querySelectorAll(".notice-dialog").length,
  }), serialLaunchBaseline);
  assert(serialLaunchState.creates === 1
    && serialLaunchState.destroys === 1
    && !serialLaunchState.sessions.includes("bench-uart")
    && serialLaunchState.pending === 0
    && serialLaunchState.notices === 0,
  `a duplicate or deleted serial-analyzer launch leaked a child window: ${JSON.stringify(serialLaunchState)}`);
  assert(serialLaunchErrors.length === 0,
    `serial-analyzer launch lifecycle browser exceptions: ${JSON.stringify(serialLaunchErrors)}`);
  await serialLaunchPage.close();

  const workspaceWindowPage = await context.newPage();
  const workspaceWindowErrors = [];
  workspaceWindowPage.on("pageerror", (error) => workspaceWindowErrors.push(error.message));
  await workspaceWindowPage.goto(`${appUrl}?workspaceWindow=1&windowId=workspace-ui-regression`);
  await workspaceWindowPage.locator(".wind-root").waitFor();
  await workspaceWindowPage.getByRole("textbox", { name: "筛选资源管理器会话", exact: true }).waitFor();
  const workspaceWindowInitial = await workspaceWindowPage.evaluate(() => ({
    tabs: document.querySelectorAll(".workspace-pane-tab").length,
    workspace: localStorage.getItem("portmate.workspace.v1"),
    panelsV1: localStorage.getItem("portmate.workspacePanels.v1"),
    panelsV2: localStorage.getItem("portmate.workspacePanels.v2"),
  }));
  assert(workspaceWindowInitial.tabs === 0,
    `a new workspace window inherited or auto-opened a main-window view: ${JSON.stringify(workspaceWindowInitial)}`);
  await workspaceWindowPage.getByRole("button", { name: "工作区", exact: true }).click();
  await workspaceWindowPage.getByRole("button", { name: "还原布局", exact: true }).click();
  await workspaceWindowPage.waitForFunction(() => document.querySelectorAll(".workspace-pane-tab").length === 0);
  assert(await workspaceWindowPage.locator(".notice-dialog").count() === 0,
    "restoring a workspace layout opened a redundant blocking notification");
  await workspaceWindowPage.locator(".tree-session", { hasText: "Edge Router" }).click();
  await workspaceWindowPage.locator(".workspace-pane-tab", { hasText: "Edge" }).waitFor();
  const workspaceWindowAfterOpen = await workspaceWindowPage.evaluate(() => ({
    tabs: document.querySelectorAll(".workspace-pane-tab").length,
    workspace: localStorage.getItem("portmate.workspace.v1"),
    panelsV1: localStorage.getItem("portmate.workspacePanels.v1"),
    panelsV2: localStorage.getItem("portmate.workspacePanels.v2"),
  }));
  assert(workspaceWindowAfterOpen.tabs === 1
    && workspaceWindowAfterOpen.workspace === workspaceWindowInitial.workspace
    && workspaceWindowAfterOpen.panelsV1 === workspaceWindowInitial.panelsV1
    && workspaceWindowAfterOpen.panelsV2 === workspaceWindowInitial.panelsV2,
  `workspace window persisted into the main workspace snapshot: ${JSON.stringify({ before: workspaceWindowInitial, after: workspaceWindowAfterOpen })}`);
  await workspaceWindowPage.locator(".tree-session", { hasText: "Bench UART" }).click();
  await workspaceWindowPage.locator(".workspace-pane-tab", { hasText: "Bench UART" }).waitFor();
  const detachedCreateBaseline = await workspaceWindowPage.evaluate(() => {
    window.__deferChildWindowCreates = true;
    return window.__invokeCalls.filter((call) => call.command === "plugin:webview|create_webview_window").length;
  });
  const detachedEdgeTab = workspaceWindowPage.locator(".workspace-pane-tab", { hasText: "Edge Router" });
  await detachedEdgeTab.click({ button: "right" });
  const detachedRaceMenu = workspaceWindowPage.locator(".workspace-view-context-menu");
  await detachedRaceMenu.getByRole("button", { name: "移到新窗口", exact: true }).evaluate((button) => {
    button.click();
    button.click();
  });
  await workspaceWindowPage.waitForFunction(() => window.__pendingChildWindowCreates.length === 1);
  await workspaceWindowPage.locator(".tree-session", { hasText: "Local Shell" }).click();
  await workspaceWindowPage.locator(".workspace-pane-tab", { hasText: "Local Shell" }).waitFor();
  await workspaceWindowPage.evaluate(() => window.__pendingChildWindowCreates.shift().resolve());
  await detachedEdgeTab.waitFor({ state: "detached" });
  const detachedRaceState = await workspaceWindowPage.evaluate((baseline) => ({
    createCalls: window.__invokeCalls.filter((call) => call.command === "plugin:webview|create_webview_window").length - baseline,
    destroyCalls: window.__invokeCalls.filter((call) => call.command === "plugin:window|destroy").length,
    sessions: [...document.querySelectorAll(".workspace-pane-tab-label")].map((label) => label.textContent?.trim()),
  }), detachedCreateBaseline);
  assert(detachedRaceState.createCalls === 1
    && detachedRaceState.destroyCalls === 0
    && JSON.stringify(detachedRaceState.sessions) === JSON.stringify(["Bench UART", "Local Shell"]),
  `slow detached-window creation overwrote a newer workspace layout: ${JSON.stringify(detachedRaceState)}`);

  await workspaceWindowPage.locator(".workspace-pane-tab", { hasText: "Bench UART" }).click({ button: "right" });
  await detachedRaceMenu.getByRole("button", { name: "移到新窗口", exact: true }).click();
  await workspaceWindowPage.waitForFunction(() => window.__pendingChildWindowCreates.length === 1);
  await workspaceWindowPage.locator(".workspace-pane-tab", { hasText: "Local Shell" }).click({ button: "right" });
  await detachedRaceMenu.getByRole("button", { name: "关闭视图", exact: true }).click();
  await workspaceWindowPage.locator(".workspace-pane-tab", { hasText: "Local Shell" }).waitFor({ state: "detached" });
  await workspaceWindowPage.evaluate(() => window.__pendingChildWindowCreates.shift().resolve());
  const detachedRollbackNotice = workspaceWindowPage.locator(".notice-dialog", { hasText: "主窗口必须保留一个视图" });
  await detachedRollbackNotice.waitFor();
  const detachedRollbackState = await workspaceWindowPage.evaluate(() => ({
    destroyCalls: window.__invokeCalls.filter((call) => call.command === "plugin:window|destroy").length,
    sessions: [...document.querySelectorAll(".workspace-pane-tab-label")].map((label) => label.textContent?.trim()),
  }));
  assert(detachedRollbackState.destroyCalls === 1
    && JSON.stringify(detachedRollbackState.sessions) === JSON.stringify(["Bench UART"]),
  `a detached-window rollback resurrected stale views or leaked its child: ${JSON.stringify(detachedRollbackState)}`);
  await detachedRollbackNotice.getByRole("button", { name: "确定", exact: true }).click();

  await workspaceWindowPage.locator(".tree-session", { hasText: "Local Shell" }).click();
  const cancelledDetachTab = workspaceWindowPage.locator(".workspace-pane-tab", { hasText: "Local Shell" });
  await cancelledDetachTab.click({ button: "right" });
  await detachedRaceMenu.getByRole("button", { name: "移到新窗口", exact: true }).click();
  await workspaceWindowPage.waitForFunction(() => window.__pendingChildWindowCreates.length === 1);
  await cancelledDetachTab.click({ button: "right" });
  await detachedRaceMenu.getByRole("button", { name: "关闭视图", exact: true }).click();
  await cancelledDetachTab.waitFor({ state: "detached" });
  await workspaceWindowPage.evaluate(() => window.__pendingChildWindowCreates.shift().resolve());
  await workspaceWindowPage.waitForFunction(() => (
    window.__invokeCalls.filter((call) => call.command === "plugin:window|destroy").length === 2
  ));
  const cancelledDetachState = await workspaceWindowPage.evaluate(() => ({
    notices: document.querySelectorAll(".notice-dialog").length,
    sessions: [...document.querySelectorAll(".workspace-pane-tab-label")].map((label) => label.textContent?.trim()),
  }));
  assert(cancelledDetachState.notices === 0
    && JSON.stringify(cancelledDetachState.sessions) === JSON.stringify(["Bench UART"]),
  `closing a view did not cancel its older detached-window request: ${JSON.stringify(cancelledDetachState)}`);

  await workspaceWindowPage.evaluate(() => {
    const base = {
      action: "reattach",
      ownerWindowId: "workspace-ui-regression",
      title: "",
      color: "",
      keyMode: "remote",
    };
    window.__emitTauriEvent("portmate-detached-pane-command", {
      ...base,
      windowId: "reattach-edge-window",
      paneId: "reattach-edge-pane",
      viewId: "reattach-edge-view",
      sessionId: "edge-router",
    });
    window.__emitTauriEvent("portmate-detached-pane-command", {
      ...base,
      windowId: "reattach-local-window",
      paneId: "reattach-local-pane",
      viewId: "reattach-local-view",
      sessionId: "local-shell",
    });
  });
  await workspaceWindowPage.waitForFunction(() => document.querySelectorAll(".workspace-pane-tab").length === 3);
  const sequentialReattachState = await workspaceWindowPage.evaluate(() => ({
    groups: document.querySelectorAll(".terminal-pane").length,
    sessions: [...document.querySelectorAll(".workspace-pane-tab-label")].map((label) => label.textContent?.trim()),
  }));
  assert(sequentialReattachState.groups === 3
    && JSON.stringify(sequentialReattachState.sessions) === JSON.stringify(["Bench UART", "Edge Router", "Local Shell"]),
  `same-frame detached returns overwrote each other: ${JSON.stringify(sequentialReattachState)}`);
  const initialReattachAcks = await workspaceWindowPage.evaluate(() => window.__invokeCalls.filter((call) => (
    call.command === "plugin:event|emit_to"
      && call.args.event === "portmate-detached-pane-result"
      && call.args.payload?.ok === true
      && ["reattach-edge-window", "reattach-local-window"].includes(call.args.target?.label)
  )).length);
  assert(initialReattachAcks === 2,
    `successful detached returns were not acknowledged exactly once: ${initialReattachAcks}`);

  await workspaceWindowPage.evaluate(() => {
    window.__emitTauriEvent("portmate-detached-pane-command", {
      action: "reattach",
      windowId: "reattach-edge-window",
      ownerWindowId: "workspace-ui-regression",
      paneId: "reattach-edge-pane",
      viewId: "reattach-edge-view",
      sessionId: "edge-router",
      title: "",
      color: "",
      keyMode: "remote",
    });
  });
  await workspaceWindowPage.waitForTimeout(50);
  assert(await workspaceWindowPage.locator(".workspace-pane-tab").count() === 3,
    "a repeated detached return duplicated its workspace view");
  await workspaceWindowPage.evaluate(() => {
    window.__emitTauriEvent("portmate-detached-pane-command", {
      action: "reattach",
      windowId: "reattach-edge-window",
      ownerWindowId: "workspace-ui-regression",
      paneId: "reattach-edge-pane",
      viewId: "reattach-edge-view",
      sessionId: "local-shell",
      title: "",
      color: "",
      keyMode: "remote",
    });
  });
  const rejectedReattachNotice = workspaceWindowPage.locator(".notice-dialog", { hasText: "返回的视图标识与当前工作区冲突" });
  await rejectedReattachNotice.waitFor();
  const rejectedReattachState = await workspaceWindowPage.evaluate(() => ({
    tabs: document.querySelectorAll(".workspace-pane-tab").length,
    acknowledgement: window.__invokeCalls.filter((call) => (
      call.command === "plugin:event|emit_to"
        && call.args.event === "portmate-detached-pane-result"
        && call.args.target?.label === "reattach-edge-window"
        && call.args.payload?.ok === false
    )).at(-1)?.args.payload,
  }));
  assert(rejectedReattachState.tabs === 3
    && rejectedReattachState.acknowledgement?.error?.includes("视图标识"),
  `a rejected detached return was not preserved and acknowledged: ${JSON.stringify(rejectedReattachState)}`);
  await rejectedReattachNotice.getByRole("button", { name: "确定", exact: true }).click();
  await workspaceWindowPage.screenshot({ path: `${screenshotPrefix}-workspace-window.png`, fullPage: true });
  await workspaceWindowPage.evaluate(() => {
    window.__workspaceWindowPopupCalls = [];
    Object.defineProperty(window, "open", {
      configurable: true,
      value: (path, name, features) => {
        window.__workspaceWindowPopupCalls.push({ path, name, features });
        return { focus: () => {} };
      },
    });
    delete window.__TAURI_INTERNALS__;
  });
  await workspaceWindowPage.getByRole("button", { name: "会话", exact: true }).click();
  await workspaceWindowPage.getByRole("button", { name: "新建工作区窗口", exact: true }).click();
  await workspaceWindowPage.waitForFunction(() => window.__workspaceWindowPopupCalls.length === 1);
  const workspaceWindowPopup = await workspaceWindowPage.evaluate(() => window.__workspaceWindowPopupCalls[0]);
  assert(typeof workspaceWindowPopup?.name === "string"
    && /^workspace-[A-Za-z0-9_-]+$/.test(workspaceWindowPopup.name)
    && typeof workspaceWindowPopup?.path === "string"
    && workspaceWindowPopup.path.includes("workspaceWindow=1")
    && workspaceWindowPopup.path.includes(`windowId=${workspaceWindowPopup.name}`),
  `new workspace window used an invalid browser route: ${JSON.stringify(workspaceWindowPopup)}`);
  assert(workspaceWindowErrors.length === 0,
    `workspace window browser exceptions: ${JSON.stringify(workspaceWindowErrors)}`);
  await workspaceWindowPage.close();

  const layoutSyncPage = await context.newPage();
  const layoutSyncErrors = [];
  layoutSyncPage.on("pageerror", (error) => layoutSyncErrors.push(error.message));
  await layoutSyncPage.goto(`${appUrl}?workspaceWindow=1&windowId=workspace-layout-sync-regression`);
  await layoutSyncPage.locator(".workspace-dock-content.panel-explorer .tree-session").first().click();
  const layoutSyncTab = layoutSyncPage.locator(".workspace-pane-tab").first();
  await layoutSyncTab.waitFor();
  const layoutSyncBaseLabel = (await layoutSyncTab.locator(".workspace-pane-tab-label").textContent())?.trim();
  await layoutSyncTab.click({ button: "right" });
  const duplicateLayoutView = layoutSyncPage.locator(".workspace-view-context-menu")
    .getByRole("button", { name: "复制视图", exact: true });
  await duplicateLayoutView.waitFor();
  await duplicateLayoutView.evaluate((button) => {
    button.click();
    button.click();
  });
  await layoutSyncPage.waitForFunction(() => document.querySelectorAll(".workspace-pane-tab").length === 3);
  const layoutSyncState = await layoutSyncPage.locator(".workspace-pane-tab").evaluateAll((tabs) => ({
    ids: tabs.map((tab) => tab.getAttribute("data-view-id")),
    labels: tabs.map((tab) => tab.querySelector(".workspace-pane-tab-label")?.textContent?.trim()),
  }));
  assert(new Set(layoutSyncState.ids).size === 3
    && layoutSyncState.labels.includes(`${layoutSyncBaseLabel} 副本`)
    && layoutSyncState.labels.includes(`${layoutSyncBaseLabel} 副本 2`),
  `same-frame workspace mutations did not compose against the latest layout: ${JSON.stringify(layoutSyncState)}`);
  assert(layoutSyncErrors.length === 0,
    `same-frame workspace mutation browser exceptions: ${JSON.stringify(layoutSyncErrors)}`);
  await layoutSyncPage.close();

  const lockSyncMainPage = await context.newPage();
  const lockSyncWorkspacePage = await context.newPage();
  const lockSyncErrors = [];
  lockSyncMainPage.on("pageerror", (error) => lockSyncErrors.push(`main: ${error.message}`));
  lockSyncWorkspacePage.on("pageerror", (error) => lockSyncErrors.push(`workspace: ${error.message}`));
  await lockSyncMainPage.goto(appUrl);
  await lockSyncWorkspacePage.goto(`${appUrl}?workspaceWindow=1&windowId=workspace-lock-regression`);
  await Promise.all([
    lockSyncMainPage.getByRole("textbox", { name: "筛选资源管理器会话", exact: true }).waitFor(),
    lockSyncWorkspacePage.getByRole("textbox", { name: "筛选资源管理器会话", exact: true }).waitFor(),
  ]);
  await lockSyncMainPage.evaluate(() => window.localStorage.removeItem("portmate.screenLock.v1"));
  await lockSyncMainPage.keyboard.press("Control+Alt+L");
  await lockSyncMainPage.locator(".screen-lock-overlay").waitFor();
  await lockSyncWorkspacePage.locator(".screen-lock-overlay").waitFor();
  await lockSyncWorkspacePage.getByRole("button", { name: "返回工作台", exact: true }).click();
  await Promise.all([
    lockSyncMainPage.locator(".screen-lock-overlay").waitFor({ state: "detached" }),
    lockSyncWorkspacePage.locator(".screen-lock-overlay").waitFor({ state: "detached" }),
  ]);
  assert(lockSyncErrors.length === 0, `cross-workspace screen lock browser exceptions: ${JSON.stringify(lockSyncErrors)}`);
  await lockSyncMainPage.close();
  await lockSyncWorkspacePage.close();

  const profileSyncPage = await context.newPage();
  const profileSyncErrors = [];
  profileSyncPage.on("pageerror", (error) => profileSyncErrors.push(error.message));
  await profileSyncPage.goto(`${appUrl}?workspaceWindow=1&windowId=workspace-profile-sync-regression`);
  const profileSyncTree = profileSyncPage.locator(".workspace-dock-content.panel-explorer .tree-session");
  await profileSyncTree.filter({ hasText: "Edge Router" }).waitFor();
  await profileSyncPage.evaluate(() => {
    const index = window.__sessions.findIndex((session) => session.profile.id === "edge-router");
    const updated = structuredClone(window.__sessions[index]);
    updated.profile.name = "Updated edge profile";
    window.__sessions[index] = updated;
    window.__emitTauriEvent("portmate-session-profile-updated", updated);
  });
  await profileSyncTree.filter({ hasText: "Updated edge profile" }).waitFor();
  await profileSyncTree.filter({ hasText: "Updated edge profile" }).click();
  await profileSyncPage.locator(".terminal-pane.active .terminal-host").waitFor();
  await profileSyncPage.evaluate(() => {
    window.__deferTerminalTextExports = true;
    window.__pendingTerminalTextExports = [];
  });
  const profileSyncTerminal = profileSyncPage.locator(".terminal-pane.active .terminal-host");
  await profileSyncTerminal.click({ button: "right", position: { x: 40, y: 40 } });
  await profileSyncPage.locator(".terminal-context-menu .context-menu-row", { hasText: /^导出终端文本$/ }).click();
  await profileSyncPage.waitForFunction(() => window.__pendingTerminalTextExports.length === 1);
  await profileSyncTree.filter({ hasText: "Updated edge profile" }).click({ button: "right" });
  const staleProfileSaveAction = profileSyncPage.getByRole("button", { name: /保存会话\(S\)/ });
  await staleProfileSaveAction.waitFor();
  const staleProfileSaveBaseline = await profileSyncPage.evaluate(() => (
    window.__invokeCalls.filter((call) => call.command === "save_session_profile").length
  ));
  const staleProfileSaveClicked = await profileSyncPage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
    const staleSaveButton = [...document.querySelectorAll('[aria-label="会话菜单"] button')]
      .find((button) => button.textContent?.includes("保存会话(S)"));
    staleSaveButton?.click();
    window.__emitTauriEvent("portmate-detached-pane-command", {
      action: "reattach",
      requestId: "deleted-profile-return-request",
      windowId: "deleted-profile-window",
      ownerWindowId: "workspace-profile-sync-regression",
      paneId: "deleted-profile-return-pane",
      viewId: "deleted-profile-return-view",
      sessionId: "edge-router",
      title: "Updated edge profile",
      color: "",
      keyMode: "remote",
    });
    return Boolean(staleSaveButton);
  });
  await profileSyncTree.filter({ hasText: "Updated edge profile" }).waitFor({ state: "detached" });
  const deletedProfileReturnNotice = profileSyncPage.locator(".notice-dialog", { hasText: "原会话已不存在" });
  await deletedProfileReturnNotice.waitFor();
  await profileSyncPage.waitForFunction(() => window.__invokeCalls.some((call) => (
    call.command === "plugin:event|emit_to"
      && call.args.event === "portmate-detached-pane-result"
      && call.args.payload?.requestId === "deleted-profile-return-request"
  )));
  const deletedProfileReturnState = await profileSyncPage.evaluate(() => ({
    tabs: [...document.querySelectorAll(".workspace-pane-tab-label")].map((label) => label.textContent?.trim()),
    sessions: window.__sessions.map((session) => session.profile.id),
    profileSaveCalls: window.__invokeCalls.filter((call) => call.command === "save_session_profile").length,
    acknowledgement: window.__invokeCalls.findLast((call) => (
      call.command === "plugin:event|emit_to"
        && call.args.event === "portmate-detached-pane-result"
        && call.args.payload?.requestId === "deleted-profile-return-request"
    ))?.args.payload,
  }));
  assert(!deletedProfileReturnState.tabs.some((label) => label?.includes("Updated edge profile"))
    && !deletedProfileReturnState.sessions.includes("edge-router")
    && staleProfileSaveClicked
    && deletedProfileReturnState.profileSaveCalls === staleProfileSaveBaseline
    && deletedProfileReturnState.acknowledgement?.ok === false
    && deletedProfileReturnState.acknowledgement?.error?.includes("原会话已不存在"),
    `same-frame Profile deletion accepted a detached return for the deleted session: ${JSON.stringify(deletedProfileReturnState)}`);
  await deletedProfileReturnNotice.getByRole("button", { name: "确定", exact: true }).click();
  await profileSyncPage.evaluate(() => {
    window.__deferTerminalTextExports = false;
    window.__pendingTerminalTextExports.shift().resolve();
  });
  await profileSyncPage.waitForTimeout(100);
  assert(await profileSyncPage.locator(".notice-dialog").count() === 0,
    "a terminal export response that arrived after Profile deletion produced a stale notice");
  assert(profileSyncErrors.length === 0,
    `workspace Profile event synchronization browser exceptions: ${JSON.stringify(profileSyncErrors)}`);
  await profileSyncPage.close();

  const cacheRecoveryContext = await browser.newContext({ viewport: { width: 960, height: 680 } });
  await cacheRecoveryContext.addInitScript(() => {
    localStorage.clear();
    localStorage.setItem("portmate.sessions", JSON.stringify({ version: 1, sessions: [null] }));
    Storage.prototype.setItem = () => { throw new DOMException("storage denied", "SecurityError"); };
  });
  const cacheRecoveryErrors = [];
  const cacheRecoveryMain = await cacheRecoveryContext.newPage();
  cacheRecoveryMain.on("pageerror", (error) => cacheRecoveryErrors.push(`main: ${error.message}`));
  await cacheRecoveryMain.goto(appUrl);
  await cacheRecoveryMain.locator(".wind-root").waitFor();

  const cacheRecoveryDetached = await cacheRecoveryContext.newPage();
  cacheRecoveryDetached.on("pageerror", (error) => cacheRecoveryErrors.push(`detached: ${error.message}`));
  await cacheRecoveryDetached.goto(`${appUrl}?detachedPane=1&windowId=cache-recovery&paneId=cache-pane&viewId=cache-view&sessionId=missing&title=&color=&keyMode=remote`);
  await cacheRecoveryDetached.locator(".detached-pane-root").waitFor();
  assert((await cacheRecoveryDetached.locator(".detached-pane-toolbar").textContent())?.includes("会话不可用"),
    "detached window did not conservatively discard an invalid session cache");

  const cacheRecoveryAnalyzer = await cacheRecoveryContext.newPage();
  cacheRecoveryAnalyzer.on("pageerror", (error) => cacheRecoveryErrors.push(`analyzer: ${error.message}`));
  await cacheRecoveryAnalyzer.goto(`${appUrl}?serialAnalyzer=1&windowId=cache-analyzer&sessionId=missing`);
  await cacheRecoveryAnalyzer.locator(".serial-analyzer-root").waitFor();
  assert((await cacheRecoveryAnalyzer.locator(".serial-analyzer-missing").textContent())?.includes("串口会话不可用"),
    "serial analyzer did not conservatively discard an invalid session cache");
  assert(cacheRecoveryErrors.length === 0,
    `invalid session cache or denied preference writes caused browser exceptions: ${JSON.stringify(cacheRecoveryErrors)}`);
  await cacheRecoveryContext.close();

  await page.setViewportSize({ width: 1440, height: 900 });
  const openSshImportSaveStart = await page.evaluate(() => window.__invokeCalls.length);
  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "导入会话" }).click();
  const savingOpenSshImport = page.locator(".session-config-import-dialog");
  await savingOpenSshImport.waitFor();
  await savingOpenSshImport.getByRole("textbox", { name: "OpenSSH 配置内容", exact: true }).fill(`Host saved-profile
  HostName saved.example.test
  User deploy
  Port 2202
  HostKeyAlias saved-device
  IdentityFile ~/.ssh/id_saved
  ServerAliveInterval 45
  ServerAliveCountMax 5
  TCPKeepAlive no
  IdentitiesOnly no
  ForwardAgent yes
  ProxyJump ops@bastion.example.test:2222`);
  await page.evaluate(() => {
    window.__deferSessionProfileSaves = true;
    window.__pendingSessionProfileSaves = [];
  });
  await savingOpenSshImport.evaluate((dialog) => {
    const buttons = [...dialog.querySelectorAll("button")];
    const importButton = buttons.find((button) => button.textContent?.trim() === "导入");
    importButton?.click();
    importButton?.click();
    buttons.find((button) => button.textContent?.trim() === "PuTTY")?.click();
    buttons.find((button) => button.textContent?.trim() === "取消")?.click();
  });
  await page.waitForFunction(() => window.__pendingSessionProfileSaves.length === 1);
  assert(await savingOpenSshImport.getByRole("button", { name: "导入", exact: true }).isDisabled()
    && await savingOpenSshImport.getByRole("button", { name: "PuTTY", exact: true }).isDisabled()
    && await savingOpenSshImport.getByRole("button", { name: "取消", exact: true }).isDisabled()
    && await savingOpenSshImport.locator(".dialog-title > button").isDisabled(),
  "a pending Session import did not lock duplicate submit, mode switch, or close actions");
  await page.evaluate(() => {
    window.__deferSessionProfileSaves = false;
    window.__pendingSessionProfileSaves.shift().resolve();
  });
  await savingOpenSshImport.locator(".dialog-note", { hasText: "已导入 1 个会话" }).waitFor();
  const openSshImportLifecycleState = await page.evaluate((start) => {
    const call = window.__invokeCalls.slice(start).find((item) => item.command === "save_session_profile");
    const profile = call?.args?.profile;
    return {
      saveCalls: window.__invokeCalls.slice(start).filter((item) => item.command === "save_session_profile").length,
      expectedProfile: call?.args && Object.hasOwn(call.args, "expectedProfile")
        ? call.args.expectedProfile
        : "missing",
      profile: profile ? {
        name: profile.name,
        kind: profile.kind,
        connection: {
          kind: profile.connection.kind,
          endpoint: profile.connection.endpoint,
          username: profile.connection.username,
          keepaliveEnabled: profile.connection.keepaliveEnabled,
          keepaliveIntervalSeconds: profile.connection.keepaliveIntervalSeconds,
          keepaliveMaxMissed: profile.connection.keepaliveMaxMissed,
          tcpKeepaliveEnabled: profile.connection.tcpKeepaliveEnabled,
          hostKeyAlias: profile.connection.hostKeyPolicy.alias,
          identitiesOnly: profile.connection.identityPolicy.identitiesOnly,
          forwarding: profile.connection.agentPolicy.forwarding,
          identityPaths: profile.connection.identityRefs.map((identity) => identity.path),
          jumps: profile.connection.jumps,
        },
      } : null,
    };
  }, openSshImportSaveStart);
  assert(openSshImportLifecycleState.saveCalls === 1
    && openSshImportLifecycleState.expectedProfile === null
    && JSON.stringify(openSshImportLifecycleState.profile) === JSON.stringify({
      name: "saved-profile",
      kind: "ssh",
      connection: {
        kind: "ssh",
        endpoint: { host: "saved.example.test", port: 2202 },
        username: "deploy",
        keepaliveEnabled: true,
        keepaliveIntervalSeconds: 45,
        keepaliveMaxMissed: 5,
        tcpKeepaliveEnabled: false,
        hostKeyAlias: "saved-device",
        identitiesOnly: false,
        forwarding: true,
        identityPaths: ["~/.ssh/id_saved"],
        jumps: [{
          host: "bastion.example.test",
          port: 2222,
          username: "ops",
          passwordSecretRef: null,
          passphraseSecretRef: null,
          identityRef: null,
          hostKeyPolicy: null,
        }],
      },
    }),
  `OpenSSH config import did not save the expected Profile: ${JSON.stringify(openSshImportLifecycleState)}`);
  await savingOpenSshImport.getByRole("button", { name: "取消", exact: true }).click();
  await savingOpenSshImport.waitFor({ state: "detached" });

  const puttyImportSaveStart = await page.evaluate(() => window.__invokeCalls.length);
  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "导入会话" }).click();
  const savingPuttyImport = page.locator(".session-config-import-dialog");
  await savingPuttyImport.waitFor();
  await savingPuttyImport.getByRole("button", { name: "PuTTY", exact: true }).click();
  await savingPuttyImport.getByRole("textbox", { name: "PuTTY 配置内容", exact: true }).fill(`Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Saved%20PuTTY]
"HostName"="saved-putty.example.test"
"PortNumber"=dword:0000089a
"Protocol"="ssh"
"UserName"="deploy"
"TCPKeepalives"=dword:00000001
"TryAgent"=dword:00000001
"AgentFwd"=dword:00000001
"ProxyMethod"=dword:00000003
"ProxyHost"="proxy.example.test"
"ProxyPort"=dword:00001f90
"ProxyUsername"="relay"
"ProxyPassword"="stored-secret"
"PublicKeyFile"="C:\\\\Users\\\\operator\\\\id_saved.ppk"`);
  await savingPuttyImport.getByRole("button", { name: "导入", exact: true }).click();
  await savingPuttyImport.locator(".dialog-note", { hasText: "已导入 1 个会话" }).waitFor();
  const puttyImportLifecycleState = await page.evaluate((start) => {
    const call = window.__invokeCalls.slice(start).find((item) => item.command === "save_session_profile");
    const profile = call?.args?.profile;
    return {
      saveCalls: window.__invokeCalls.slice(start).filter((item) => item.command === "save_session_profile").length,
      expectedProfile: call?.args && Object.hasOwn(call.args, "expectedProfile")
        ? call.args.expectedProfile
        : "missing",
      profile: profile ? {
        name: profile.name,
        kind: profile.kind,
        connection: {
          kind: profile.connection.kind,
          endpoint: profile.connection.endpoint,
          username: profile.connection.username,
          tcpKeepaliveEnabled: profile.connection.tcpKeepaliveEnabled,
          proxy: profile.connection.proxy,
          agentPolicy: profile.connection.agentPolicy,
          identityRefs: profile.connection.identityRefs,
        },
      } : null,
      serializedProfile: JSON.stringify(profile),
    };
  }, puttyImportSaveStart);
  assert(puttyImportLifecycleState.saveCalls === 1
    && puttyImportLifecycleState.expectedProfile === null
    && JSON.stringify(puttyImportLifecycleState.profile) === JSON.stringify({
      name: "Saved PuTTY",
      kind: "ssh",
      connection: {
        kind: "ssh",
        endpoint: { host: "saved-putty.example.test", port: 2202 },
        username: "deploy",
        tcpKeepaliveEnabled: true,
        proxy: {
          enabled: true,
          kind: "http-connect",
          host: "proxy.example.test",
          port: 8080,
          username: "relay",
          passwordSecretRef: null,
        },
        agentPolicy: {
          enabled: true,
          forwarding: true,
          offerMode: "after-profile-keys",
        },
        identityRefs: [],
      },
    })
    && !puttyImportLifecycleState.serializedProfile.includes("stored-secret")
    && !puttyImportLifecycleState.serializedProfile.includes("id_saved.ppk"),
  `PuTTY import did not save a safe, mapped Profile: ${JSON.stringify(puttyImportLifecycleState)}`);
  await savingPuttyImport.getByRole("button", { name: "取消", exact: true }).click();
  await savingPuttyImport.waitFor({ state: "detached" });

  const shellImportSaveStart = await page.evaluate(() => window.__invokeCalls.length);
  await page.locator(".menu-trigger", { hasText: "会话" }).click();
  await page.locator(".menu-popover button", { hasText: "导入会话" }).click();
  const savingShellImport = page.locator(".session-config-import-dialog");
  await savingShellImport.waitFor();
  await savingShellImport.getByRole("button", { name: "Shell", exact: true }).click();
  await savingShellImport.getByRole("textbox", { name: "Shell 列表内容", exact: true }).fill("/usr/bin/zsh");
  await savingShellImport.getByRole("button", { name: "导入", exact: true }).click();
  await savingShellImport.locator(".dialog-note", { hasText: "已导入 1 个会话" }).waitFor();
  const shellImportLifecycleState = await page.evaluate((start) => {
    const call = window.__invokeCalls.slice(start).find((item) => item.command === "save_session_profile");
    const profile = call?.args?.profile;
    return {
      saveCalls: window.__invokeCalls.slice(start).filter((item) => item.command === "save_session_profile").length,
      expectedProfile: call?.args && Object.hasOwn(call.args, "expectedProfile")
        ? call.args.expectedProfile
        : "missing",
      profile: profile ? {
        name: profile.name,
        kind: profile.kind,
        connection: profile.connection,
      } : null,
    };
  }, shellImportSaveStart);
  assert(shellImportLifecycleState.saveCalls === 1
    && shellImportLifecycleState.expectedProfile === null
    && JSON.stringify(shellImportLifecycleState.profile) === JSON.stringify({
      name: "zsh",
      kind: "shell",
      connection: { kind: "shell", program: "/usr/bin/zsh", args: [], cwd: null },
    }),
  `local Shell import did not save the expected Profile: ${JSON.stringify(shellImportLifecycleState)}`);
  await savingShellImport.getByRole("button", { name: "取消", exact: true }).click();
  await savingShellImport.waitFor({ state: "detached" });

  await openTerminalSettings();
  await page.locator(".terminal-settings-dialog .settings-tabs > button", { hasText: "命令历史" }).click();
  await page.locator(".terminal-settings-dialog .settings-secondary-button", { hasText: "清除" }).click();
  await page.waitForFunction(() => window.__invokeCalls.some((call) => call.command === "clear_command_history")
    && window.__commandHistory.entries.length === 0
    && localStorage.getItem("portmate.commandHistory") === null);
  await page.locator(".terminal-settings-dialog .dialog-actions button", { hasText: "取消" }).click();
  await page.locator(".terminal-settings-dialog").waitFor({ state: "detached" });

  console.log(JSON.stringify({
    migratedPanels: initial.panels,
    filters: ["resource tag/endpoint", "normalized history"],
    commandHistory: {
      migratedRevision: migratedCommandHistory.snapshot.revision,
      recordedRevision: recordedCommandHistory.revision,
      invalidRejected: true,
      cleared: true,
    },
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
    terminalTheme: { ...lightThemeState, centerPixel: lightTerminalPixel },
    detachedTheme: detachedThemeState,
    mcp: {
      tabs: mcpTabs,
      exportedRecordIds: exportCall.args.request.recordIds,
      approvalResponses: approvalCalls,
      approvalDesktop: desktopApprovalBounds,
      approvalMobile: mobileApprovalBounds,
      mobile: mobileMcpBounds,
    },
    customScripts: {
      desktop: customScriptBounds,
      mobile: mobileCustomScriptBounds,
      persisted: await page.evaluate(() => window.__customScripts.length),
    },
    dockResize: {
      resized: resizedDockLayout,
      restored: restoredLayout,
      reset: resetDockLayout,
    },
    startupHydration: startupHydrationState,
    startupDomainHydration: startupDomainState,
    grantLifecycle: grantLifecycleState,
    oneKeyLifecycle: oneKeyLifecycleState,
    hostKeyLifecycle: hostKeyLifecycleState,
    hostKeyProfileCopy: hostKeyProfileCopyState,
    hostKeyScan: hostKeyScanState,
    profileLifecycle: profileLifecycleState,
    privateKeyImportLifecycle: privateKeyImportLifecycleState,
    openSshImportLifecycle: openSshImportLifecycleState,
    puttyImportLifecycle: puttyImportLifecycleState,
    shellImportLifecycle: shellImportLifecycleState,
    connectionCredentialLifecycle: {
      partialWrite: partialCredentialState,
      failedProfileSave: failedProfileCredentialState,
    },
    sessionDraftSecretLifecycle: {
      cancelled: cancelledDraftSecretState,
      committed: committedDraftSecretState,
    },
    vaultLifecycle: vaultLifecycleState,
    profileRecovery: {
      renamed: renamedProfileRecoveryState,
      empty: emptyProfileRecoveryState,
    },
    sessionCacheRecovery: ["main", "detached", "serial-analyzer", "storage-write-denied"],
    workspaceWindow: {
      tabs: workspaceWindowAfterOpen.tabs,
      popup: workspaceWindowPopup,
    },
    terminalWrites,
    desktop,
    mobile,
    screenshots: [
      `${screenshotPrefix}-search.png`,
      `${screenshotPrefix}-search-mobile.png`,
      `${screenshotPrefix}-view-split-drop.png`,
      `${screenshotPrefix}-settings.png`,
      `${screenshotPrefix}-terminal-export-settings-mobile.png`,
      `${screenshotPrefix}-transfer.png`,
      `${screenshotPrefix}-transfer-load.png`,
      `${screenshotPrefix}-transfer-load-mobile.png`,
      `${screenshotPrefix}-tunnel.png`,
      `${screenshotPrefix}-tunnel-mobile.png`,
      `${screenshotPrefix}-sysmon-sidebar.png`,
      `${screenshotPrefix}-host-key-scan.png`,
      `${screenshotPrefix}-ssh-health.png`,
      `${screenshotPrefix}-file-manager.png`,
      `${screenshotPrefix}-workspace-window.png`,
      `${screenshotPrefix}-sender.png`,
      `${screenshotPrefix}-sender-advanced.png`,
      `${screenshotPrefix}-session-create-shell.png`,
      `${screenshotPrefix}-session-create-shell-mobile.png`,
      `${screenshotPrefix}-session-create.png`,
      `${screenshotPrefix}-session-settings.png`,
      `${screenshotPrefix}-session-settings-mobile.png`,
      `${screenshotPrefix}-terminal-theme-settings.png`,
      `${screenshotPrefix}-terminal-light-theme.png`,
      `${screenshotPrefix}-terminal-timestamps.png`,
      `${screenshotPrefix}-terminal-byte-desktop.png`,
      `${screenshotPrefix}-terminal-byte-narrow.png`,
      `${screenshotPrefix}-detached-theme.png`,
      `${screenshotPrefix}-detached-health.png`,
      `${screenshotPrefix}-serial-analyzer.png`,
      `${screenshotPrefix}-custom-scripts.png`,
      `${screenshotPrefix}-custom-scripts-mobile.png`,
      `${screenshotPrefix}-mcp-grants.png`,
      `${screenshotPrefix}-mcp-grant-expiry.png`,
      `${screenshotPrefix}-mcp-grant-expiry-mobile.png`,
      `${screenshotPrefix}-mcp-http.png`,
      `${screenshotPrefix}-mcp-http-mobile.png`,
      `${screenshotPrefix}-mcp-audit.png`,
      `${screenshotPrefix}-mcp-audit-mobile.png`,
      `${screenshotPrefix}-mcp-approval.png`,
      `${screenshotPrefix}-mcp-approval-mobile.png`,
      `${screenshotPrefix}-profile-delete.png`,
      `${screenshotPrefix}-openssh-import.png`,
      `${screenshotPrefix}-openssh-import-mobile.png`,
      `${screenshotPrefix}-putty-import.png`,
      `${screenshotPrefix}-shell-import.png`,
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
