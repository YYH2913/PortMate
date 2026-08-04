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
const isoNow = new Date(recordedAt).toISOString();

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
    port: "/dev/ttyUSB0",
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
    const deferStartupSessions = sessionStorage.getItem("portmate.workspaceUiCheck.deferStartupSessions") === "true";
    const deferStartupDomains = sessionStorage.getItem("portmate.workspaceUiCheck.deferStartupDomains") === "true";
    const recoverInactiveStartup = sessionStorage.getItem("portmate.workspaceUiCheck.recoverInactiveStartup") === "true";
    sessionStorage.removeItem("portmate.workspaceUiCheck.deferStartupSessions");
    sessionStorage.removeItem("portmate.workspaceUiCheck.deferStartupDomains");
    sessionStorage.removeItem("portmate.workspaceUiCheck.recoverInactiveStartup");
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
    window.__events = structuredClone(initialEvents);
    window.__oneKeys = [];
    window.__hostKeys = [];
    window.__hostKeySequence = 0;
    window.__hostKeyScanMode = "unknown";
    window.__deferHostKeyMutations = false;
    window.__pendingHostKeyMutations = [];
    window.__deferProfileMutations = false;
    window.__pendingProfileMutations = [];
    window.__profileMutationFailureMode = null;
    window.__secretSequence = 0;
    window.__secrets = {};
    window.__deferSecretWrites = false;
    window.__pendingSecretWrites = [];
    window.__failSecretWriteAt = 0;
    window.__failNextProfileSave = false;
    window.__portableVault = { exists: false, unlocked: false, path: "/tmp/portmate-test-vault.stronghold" };
    window.__deferVaultMutations = false;
    window.__pendingVaultMutations = [];
    window.__mcpGrants = structuredClone(initialMcpGrants);
    window.__transfers = [];
    window.__commandHistory = { entries: [], migrated: false, revision: 0 };
    window.__injectCommandHistoryStartupRace = true;
    window.__clipboardText = "";
    window.__closeSessionError = false;
    window.__deferSessionOpens = false;
    window.__pendingSessionOpens = [];
    window.__deferFileLoads = false;
    window.__pendingFileLoads = [];
    window.__deferFileProperties = false;
    window.__pendingFileProperties = [];
    window.__deferTailLogs = false;
    window.__pendingTailLogs = [];
    window.__failTailLogs = 0;
    window.__failOneKeyLists = 0;
    window.__oneKeySequence = 0;
    window.__deferOneKeyMutations = false;
    window.__pendingOneKeyMutations = [];
    window.__deferTunnelRefresh = false;
    window.__pendingTunnelRefresh = [];
    window.__deferSysmon = false;
    window.__pendingSysmon = [];
    window.__deferSessionLists = deferStartupSessions;
    window.__pendingSessionLists = [];
    window.__deferTransferLists = deferStartupDomains;
    window.__pendingTransferLists = [];
    window.__deferGrantLists = deferStartupDomains;
    window.__pendingGrantLists = [];
    window.__deferGrantMutations = false;
    window.__pendingGrantMutations = [];
    window.__logShards = [
      { path: "logs/a.txt", format: "txt", size: 32, modifiedAt: new Date().toISOString() },
      { path: "logs/b.jsonl", format: "jsonl", size: 48, modifiedAt: new Date().toISOString() },
    ];
    window.__deferLogPreviews = false;
    window.__pendingLogPreviews = [];
    window.__deferMcpHttpConfig = false;
    window.__pendingMcpHttpConfig = [];
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
          if (window.__failNextProfileSave) {
            window.__failNextProfileSave = false;
            throw new Error("simulated Profile save failure");
          }
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
          return saved;
        }
        if (command === "save_secret") {
          const result = { secretRef: `keychain:test-secret-${++window.__secretSequence}` };
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
        if (command === "list_serial_capture") {
          return { frames: [], reset: false, totalFrames: 0, capturedBytes: 0 };
        }
        if (command === "list_serial_capture_history") {
          return { frames: [], enabled: false, totalFrames: 0, capturedBytes: 0, droppedFrames: 0, unavailableFrames: 0 };
        }
        if (command === "list_log_shards") return window.__logShards;
        if (command === "read_log_shard") {
          if (!window.__deferLogPreviews) {
            return { path: args.path, content: `preview:${args.path}`, encoding: "utf8", bytesRead: 16, truncated: false };
          }
          return new Promise((resolve) => {
            window.__pendingLogPreviews.push({ args: structuredClone(args), resolve });
          });
        }
        if (command === "list_files") {
          if (!window.__deferFileLoads) return [];
          return new Promise((resolve) => {
            window.__pendingFileLoads.push({ args: structuredClone(args), resolve });
          });
        }
        if (command === "create_file" || command === "delete_paths" || command === "move_paths") return null;
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
          if (!window.__deferTunnelRefresh) return [];
          return new Promise((resolve) => {
            window.__pendingTunnelRefresh.push({ args: structuredClone(args), resolve });
          });
        }
        if (command === "list_sysmon_history" || command === "refresh_sysmon") {
          if (!window.__deferSysmon) return command === "list_sysmon_history" ? [] : null;
          return new Promise((resolve) => {
            window.__pendingSysmon.push({ command, args: structuredClone(args), resolve });
          });
        }
        if (command === "check_ssh_health") {
          return {
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
        }
        if (command === "list_mcp_grants") {
          const result = structuredClone(window.__mcpGrants);
          if (!window.__deferGrantLists) return result;
          return new Promise((resolve) => window.__pendingGrantLists.push({ result, resolve }));
        }
        if (command === "list_one_keys") {
          if (window.__failOneKeyLists > 0) {
            window.__failOneKeyLists -= 1;
            throw new Error("simulated list_one_keys failure");
          }
          return structuredClone(window.__oneKeys);
        }
        if (command === "save_one_key") {
          const request = args.request;
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
          const profile = args.profile;
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
          return {
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
        }
        if (command === "trust_scanned_host_key") {
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
        if (command === "get_profile_secret_migration_recovery") return null;
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
          return { keys: structuredClone(window.__hostKeys) };
        }
        if (command === "list_mcp_audit") return initialMcpAudit;
        if (command === "mcp_http_config") {
          if (!window.__deferMcpHttpConfig) return initialMcpHttpConfig;
          return new Promise((resolve) => window.__pendingMcpHttpConfig.push({ resolve }));
        }
        if (command === "list_mcp_approvals") return [];
        if (command === "respond_mcp_approval") return null;
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
        if (command === "open_session" || command === "open_session_with_one_key") {
          const index = window.__sessions.findIndex((item) => item.profile.id === args.sessionId);
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
          window.__sessions[index] = session;
          return structuredClone(session);
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
        if (command === "list_serial_ports") return [];
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
  await openSshImportDialog.getByRole("button", { name: "取消", exact: true }).click();
  await openSshImportDialog.waitFor({ state: "detached" });

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
  await page.screenshot({ path: `${screenshotPrefix}-tunnel.png`, fullPage: true });
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
  });
  await tunnelDialog.locator(".utility-actions button", { hasText: "取消" }).click();
  await tunnelDialog.waitFor({ state: "detached" });

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
    `Sysmon details did not follow the active session: ${sysmonText}`);
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

  const activeTerminalHost = page.locator(".terminal-pane.active .terminal-host");
  await activeTerminalHost.click({ button: "right", position: { x: 40, y: 40 } });
  const terminalContextMenu = page.locator(".terminal-context-menu");
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
    window.confirm = () => true;
  });
  await localFilePane.getByRole("button", { name: "删除", exact: true }).click();
  await page.waitForFunction((count) => window.__invokeCalls.filter((call) => call.command === "delete_paths").length === count + 1, deleteCallsBefore);
  const deleteRequest = await page.evaluate(() => window.__invokeCalls.filter((call) => call.command === "delete_paths").at(-1)?.args.request);
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
  await sender.getByRole("button", { name: "发送", exact: true }).click();
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
    const indicator = document.querySelector(".detached-pane-toolbar .tab-status");
    return {
      text: status?.textContent ?? "",
      title: status?.getAttribute("title") ?? "",
      live: status?.getAttribute("aria-live") ?? "",
      indicatorTitle: indicator?.getAttribute("title") ?? "",
      connectButtons: document.querySelectorAll('button[aria-label="连接会话"]').length,
      disconnectButtons: document.querySelectorAll('button[aria-label="断开会话"]').length,
    };
  });
  assert(detachedRuntimeHealth.text === detachedRuntimeHealth.title
    && detachedRuntimeHealth.text === detachedRuntimeHealth.indicatorTitle
    && detachedRuntimeHealth.text.startsWith("正在重连 · 原因: transport stalled ")
    && !detachedRuntimeHealth.text.includes("Invalid Date")
    && !detachedRuntimeHealth.text.includes("\n")
    && detachedRuntimeHealth.live === "polite",
  `detached terminal did not normalize its runtime health: ${JSON.stringify({ detachedHealth, detachedRuntimeHealth })}`);
  assert(Array.from(detachedRuntimeHealth.text.split("原因: ")[1]).length === 256
    && detachedRuntimeHealth.text.endsWith("...")
    && detachedRuntimeHealth.connectButtons === 0
    && detachedRuntimeHealth.disconnectButtons === 1,
  `detached reconnect action or diagnostic boundary is wrong: ${JSON.stringify(detachedRuntimeHealth)}`);
  const detachedEmitCallsBefore = await detachedPage.evaluate(() => window.__invokeCalls.length);
  await detachedPage.getByRole("button", { name: "断开会话", exact: true }).click();
  const detachedDisconnectCommand = await detachedPage.evaluate((start) => (
    window.__invokeCalls.slice(start).find((call) => call.command === "plugin:event|emit_to") ?? null
  ), detachedEmitCallsBefore);
  assert(detachedDisconnectCommand?.args?.target?.label === "main"
    && detachedDisconnectCommand.args.event === "portmate-detached-pane-command"
    && detachedDisconnectCommand.args.payload?.action === "disconnect"
    && detachedDisconnectCommand.args.payload?.sessionId === "local-shell",
  `detached reconnect control emitted the wrong command: ${JSON.stringify(detachedDisconnectCommand)}`);
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
    `serial analyzer exposed an internal runtime status: ${JSON.stringify(serialAnalyzerHealth)}`);
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
  await page.screenshot({ path: `${screenshotPrefix}-mcp-grants.png`, fullPage: true });

  await page.evaluate(() => { window.__deferMcpHttpConfig = true; });
  await mcpDialog.getByRole("tab", { name: "HTTP", exact: true }).click();
  await mcpDialog.locator(".mcp-http-view").waitFor();
  assert(await mcpDialog.locator(".mcp-content").count() === 0
    && await mcpDialog.locator(".mcp-audit-view").count() === 0,
  "MCP HTTP page renders inactive task content");
  await page.waitForFunction(() => window.__pendingMcpHttpConfig.length === 1);
  assert(await mcpDialog.getByRole("button", { name: "生成 Token", exact: true }).isDisabled(),
    "MCP token generation stayed enabled while HTTP configuration was loading");
  await mcpDialog.getByRole("tab", { name: "审计", exact: true }).click();
  await mcpDialog.getByRole("tab", { name: "HTTP", exact: true }).click();
  await page.waitForTimeout(100);
  assert(await page.evaluate(() => window.__pendingMcpHttpConfig.length) === 1,
    "switching back to MCP HTTP started an overlapping configuration request");
  await page.evaluate((config) => {
    for (const pending of window.__pendingMcpHttpConfig) pending.resolve(config);
    window.__pendingMcpHttpConfig = [];
    window.__deferMcpHttpConfig = false;
  }, mcpHttpConfig);
  await mcpDialog.getByRole("button", { name: "轮换 Token", exact: true }).waitFor();
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
  await mobileOpenSshImport.getByRole("button", { name: "取消", exact: true }).click();
  await mobileOpenSshImport.waitFor({ state: "detached" });

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
  await page.evaluate(() => {
    window.__deferTailLogs = true;
    window.__pendingTailLogs = [];
    window.__deferSessionLists = true;
    window.__pendingSessionLists = [];
  });
  await deleteTarget.click();
  await page.locator(".workspace-pane-tab", { hasText: "Bench UART" }).waitFor();
  await page.waitForFunction(() => window.__pendingTailLogs.some((request) => request.args.sessionId === "bench-uart"));
  await page.waitForFunction(() => window.__pendingSessionLists.length >= 1);
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
  await startupSessionDialog.locator(".dialog-field", { hasText: "名称:" }).locator("input").fill("Startup Race Profile");
  await startupSessionDialog.getByRole("button", { name: "保存", exact: true }).click();
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
    call.command === "open_session" && call.args.sessionId === "local-shell"
  )));
  const inactiveStartupState = await inactiveStartupPage.evaluate(() => ({
    status: window.__sessions.find((session) => session.profile.id === "local-shell")?.runtime.status ?? "missing",
    opens: window.__invokeCalls.filter((call) => call.command === "open_session" && call.args.sessionId === "local-shell").length,
    connectedOpens: window.__invokeCalls.filter((call) => (
      call.command === "open_session" && call.args.sessionId !== "local-shell"
    )).length,
  }));
  assert(inactiveStartupState.status === "connected"
    && inactiveStartupState.opens === 1
    && inactiveStartupState.connectedOpens === 0,
  `startup recovery did not connect only the configured inactive session: ${JSON.stringify(inactiveStartupState)}`);
  assert(inactiveStartupErrors.length === 0,
    `inactive startup recovery browser exceptions: ${JSON.stringify(inactiveStartupErrors)}`);
  await inactiveStartupPage.close();

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
  await lifecycleMcpDialog.getByRole("button", { name: "关闭 MCP Bridge", exact: true }).click();
  await lifecycleMcpDialog.waitFor({ state: "detached" });
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
  await firstOneKeyDialog.locator('button[title="保存 OneKey"]').click();
  await oneKeyLifecyclePage.waitForFunction(() => window.__pendingOneKeyMutations.length === 1);
  await firstOneKeyDialog.getByRole("button", { name: "关闭 OneKey 管理器", exact: true }).click();
  await firstOneKeyDialog.waitFor({ state: "detached" });

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
  assert(oneKeyLifecycleErrors.length === 0,
    `OneKey lifecycle browser exceptions: ${JSON.stringify(oneKeyLifecycleErrors)}`);
  await oneKeyLifecyclePage.close();

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
  await hostKeyLifecyclePage.evaluate(() => { window.__deferHostKeyMutations = true; });
  await firstKeyManager.locator(".key-actions").getByRole("button", { name: "导入", exact: true }).click();
  await hostKeyLifecyclePage.waitForFunction(() => window.__pendingHostKeyMutations.length === 1);
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
  await hostKeyEditPanel.getByRole("button", { name: "保存编辑", exact: true }).click();
  await hostKeyLifecyclePage.waitForFunction(() => (
    window.__invokeCalls.filter((call) => call.command === "update_host_key").length === 1
  ));
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
  await hostKeyScanResult.getByRole("button", { name: "加入 Profile", exact: true }).click();
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
  await firstProfileManager.getByRole("button", { name: "保存字段", exact: true }).click();
  await profileLifecyclePage.waitForFunction(() => window.__pendingProfileMutations.length === 1);
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
  assert(profileLifecycleErrors.length === 0,
    `Profile lifecycle browser exceptions: ${JSON.stringify(profileLifecycleErrors)}`);
  await profileLifecyclePage.close();

  const privateKeyImportLifecyclePage = await context.newPage();
  const privateKeyImportLifecycleErrors = [];
  privateKeyImportLifecyclePage.on("pageerror", (error) => privateKeyImportLifecycleErrors.push(error.message));
  await privateKeyImportLifecyclePage.goto(appUrl);
  await privateKeyImportLifecyclePage.locator(".tree-session", { hasText: "Edge Router" }).waitFor();
  await privateKeyImportLifecyclePage.getByRole("button", { name: "工具", exact: true }).click();
  await privateKeyImportLifecyclePage.getByRole("button", { name: "密钥管理器", exact: true }).click();
  const importingKeyManager = privateKeyImportLifecyclePage.locator(".key-dialog");
  const importPanel = importingKeyManager.locator(".key-import-panel");
  await importPanel.locator("summary").click();
  await importPanel.getByPlaceholder("Key label", { exact: true }).fill("Deferred imported key");
  await importPanel.getByPlaceholder("粘贴 OpenSSH private key", { exact: true }).fill([
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "test-key-body",
    "-----END OPENSSH PRIVATE KEY-----",
  ].join("\n"));
  await privateKeyImportLifecyclePage.evaluate(() => { window.__deferSecretWrites = true; });
  await importPanel.getByRole("button", { name: "导入到 Profile", exact: true }).click();
  await privateKeyImportLifecyclePage.waitForFunction(() => window.__pendingSecretWrites.length === 1);
  await importingKeyManager.getByRole("button", { name: "关闭密钥管理器", exact: true }).click();
  await importingKeyManager.waitFor({ state: "detached" });

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
  await partialCredentialDialog.getByLabel("保存登录密码到系统密钥库", { exact: true }).check();
  await partialCredentialDialog.getByLabel("私钥口令", { exact: true }).fill("saved passphrase");
  await partialCredentialDialog.getByLabel("保存私钥口令到系统密钥库", { exact: true }).check();
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
  await failedProfileCredentialDialog.getByLabel("保存登录密码到系统密钥库", { exact: true }).check();
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
    return window.__invokeCalls.length;
  });
  await lifecycleSession.dispatchEvent("contextmenu", { clientX: 120, clientY: 160 });
  const reconnectContextMenu = connectionLifecyclePage.locator(".portmate-context-menu:not(.workspace-view-context-menu):not(.terminal-context-menu)");
  await reconnectContextMenu.waitFor();
  const reconnectAction = reconnectContextMenu.locator("button", { hasText: "重新连接会话(R)" });
  assert(!await reconnectAction.isDisabled(), "connected session context menu disabled reconnect");
  await reconnectAction.click();
  await connectionLifecyclePage.waitForFunction(() => window.__pendingSessionOpens.length === 1);
  const reconnectHealth = await connectionLifecyclePage
    .getByRole("button", { name: "断开 Local Shell", exact: true })
    .getAttribute("title");
  const reconnectLifecycle = await connectionLifecyclePage.evaluate((start) => ({
    calls: window.__invokeCalls.slice(start)
      .filter((call) => ["close_session", "save_session_profile", "open_session"].includes(call.command))
      .map((call) => ({
        command: call.command,
        sessionId: call.args.sessionId ?? call.args.profile?.id ?? "",
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
  const cancelledDraftSecretSave = cancelledDraftSecretDialog.locator("button", { hasText: "保存到系统密钥库" });
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
  const committedDraftSecretSave = committedDraftSecretDialog.locator("button", { hasText: "保存到系统密钥库" });
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
  await profileSyncPage.evaluate(() => {
    window.__sessions = window.__sessions.filter((session) => session.profile.id !== "edge-router");
    window.__emitTauriEvent("portmate-session-profile-deleted", "edge-router");
  });
  await profileSyncTree.filter({ hasText: "Updated edge profile" }).waitFor({ state: "detached" });
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
  await savingOpenSshImport.getByRole("button", { name: "导入", exact: true }).click();
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
      `${screenshotPrefix}-transfer.png`,
      `${screenshotPrefix}-tunnel.png`,
      `${screenshotPrefix}-sysmon-sidebar.png`,
      `${screenshotPrefix}-host-key-scan.png`,
      `${screenshotPrefix}-ssh-health.png`,
      `${screenshotPrefix}-file-manager.png`,
      `${screenshotPrefix}-workspace-window.png`,
      `${screenshotPrefix}-sender.png`,
      `${screenshotPrefix}-sender-advanced.png`,
      `${screenshotPrefix}-session-settings.png`,
      `${screenshotPrefix}-session-settings-mobile.png`,
      `${screenshotPrefix}-terminal-theme-settings.png`,
      `${screenshotPrefix}-terminal-light-theme.png`,
      `${screenshotPrefix}-detached-theme.png`,
      `${screenshotPrefix}-detached-health.png`,
      `${screenshotPrefix}-serial-analyzer.png`,
      `${screenshotPrefix}-mcp-grants.png`,
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
