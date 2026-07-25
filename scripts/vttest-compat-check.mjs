import { spawn, spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer } from "node:net";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

if (process.platform !== "linux") {
  throw new Error("The vttest/full-screen program matrix requires a Linux Docker host");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const matrix = JSON.parse(readFileSync(resolve(projectRoot, "tests/compat/terminal-program-matrix.json"), "utf8"));
const chromeExecutable = process.env.PORTMATE_CHROME ?? "/usr/bin/google-chrome";
const screenshotPrefix = process.env.PORTMATE_VTTEST_SCREENSHOT_PREFIX ?? "/tmp/portmate-vttest-compat";
const maxPtyBytes = 16 * 1024 * 1024;
const vttestSuites = [
  { name: "vt100-cursor", choices: [1], marker: "Test of leading zeros in ESC sequences.", minimumHolds: 6 },
  { name: "vt100-screen", choices: [2], marker: "Graphic rendition test pattern:", minimumHolds: 12 },
  { name: "vt100-character-sets", choices: [3], marker: "Selected as G0 (with SI)", minimumHolds: 1 },
  { name: "vt100-double-size", choices: [4], marker: "double-sized", minimumHolds: 4 },
  { name: "vt100-device-status", choices: [6, 3], marker: "Device Status Report 5", minimumHolds: 1, requireReply: true },
  { name: "vt100-primary-attributes", choices: [6, 4], marker: "Device Attributes report", minimumHolds: 1, requireReply: true },
  { name: "vt102-insert-delete", choices: [8], marker: "Insert/Delete", minimumHolds: 6 },
  { name: "vt220-visible-cursor", choices: [11, 1, 2, 2], marker: "Visible/Invisible Cursor", minimumHolds: 1 },
  { name: "vt220-erase-character", choices: [11, 1, 2, 3], marker: "Erase Char", minimumHolds: 1 },
  { name: "vt220-protected-areas", choices: [11, 1, 2, 4], marker: "Protected-Areas", minimumHolds: 1 },
  { name: "iso-6429-cursor", choices: [11, 5, "*"], marker: "ISO-6429 (ECMA-48) Cursor-Movement", minimumHolds: 6 },
  { name: "iso-6429-color", choices: [11, 6, "*"], marker: "Display color test-pattern", minimumHolds: 6 },
  { name: "xterm-alternate-screen", choices: [11, 8, 7, "*"], marker: "XTERM Alternate-Screen features", minimumHolds: 3 },
];
const fullScreenPrograms = [
  {
    name: "vim",
    command: "vim -Nu NONE -n /opt/portmate/fullscreen-probe.txt",
    marker: "PORTMATE FULLSCREEN PROBE",
    exit: ":q!\r",
  },
  {
    name: "less",
    command: "less -R /opt/portmate/fullscreen-probe.txt",
    marker: "PORTMATE FULLSCREEN PROBE",
    exit: "q",
  },
  {
    name: "top",
    command: "top -d 0.1",
    marker: /Tasks:|Mem:/,
    exit: "q",
    restoresPrimaryScreen: false,
  },
  {
    name: "dialog",
    command: "dialog --title 'PortMate matrix' --msgbox 'PORTMATE DIALOG PROBE' 8 44",
    marker: "PORTMATE DIALOG PROBE",
    exit: "\r",
    restoresPrimaryScreen: false,
    restoresCursor: false,
  },
];

if (!Array.isArray(matrix) || matrix.length < 4) {
  throw new Error("terminal program matrix must contain at least four entries");
}
run("docker", ["info", "--format", "{{.ServerVersion}}"], { quiet: true });

for (const entry of matrix) {
  validateEntry(entry);
  entry.image = `portmate-compat-terminal-${entry.name}:local`;
  const buildArgs = Object.entries(entry.buildArgs).flatMap(([name, value]) => ["--build-arg", `${name}=${value}`]);
  run("docker", ["build", "--tag", entry.image, "--file", resolve(projectRoot, entry.dockerfile), ...buildArgs, projectRoot], { quiet: true });
}

const port = await reservePort();
const appUrl = `http://127.0.0.1:${port}/tests/compat/terminal-harness.html`;
let viteOutput = "";
const vite = spawn(process.execPath, [
  "node_modules/vite/bin/vite.js",
  "--host", "127.0.0.1",
  "--port", String(port),
  "--strictPort",
], { cwd: projectRoot, stdio: ["ignore", "pipe", "pipe"] });
vite.stdout.on("data", (chunk) => { viteOutput += chunk.toString(); });
vite.stderr.on("data", (chunk) => { viteOutput += chunk.toString(); });

let browser;
let activePty = null;
const results = [];
try {
  await waitForServer(appUrl, () => viteOutput);
  browser = await chromium.launch({
    executablePath: chromeExecutable,
    headless: true,
    args: ["--no-sandbox", "--enable-unsafe-swiftshader"],
  });
  const context = await browser.newContext({ viewport: { width: 1120, height: 720 } });
  const page = await context.newPage();
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.exposeBinding("portmatePtyInput", (_source, data) => {
    if (!activePty?.stdin.writable) return;
    activePty.terminalReplyBytes += Buffer.byteLength(data);
    activePty.stdin.write(data);
  });
  await page.goto(appUrl);
  await page.waitForFunction(() => Boolean(globalThis.__portmateTerminalHarness));

  for (const entry of matrix) {
    const versions = inspectVersions(entry);
    const programs = [];
    for (const spec of fullScreenPrograms) {
      const screenshot = entry === matrix[0] && spec.name === "vim"
        ? `${screenshotPrefix}-fullscreen-vim.png`
        : null;
      programs.push(await runFullScreenProgram(page, entry, spec, screenshot));
    }

    const suites = [];
    if (entry.vttestPattern) {
      for (const suite of vttestSuites) {
        const screenshot = entry === matrix[0] && suite.name === "vt100-screen"
          ? `${screenshotPrefix}-vt100-screen.png`
          : null;
        suites.push(await runVttestSuite(page, entry, suite, screenshot));
      }
    }
    results.push({ name: entry.name, versions, programs, vttest: suites });
  }

  assert(pageErrors.length === 0, `terminal harness browser exceptions: ${JSON.stringify(pageErrors)}`);
  await context.close();
} finally {
  if (activePty?.stdin.writable) activePty.stdin.end();
  await browser?.close().catch(() => {});
  vite.kill("SIGTERM");
  await waitForExit(vite, 2_000);
}

const versionCount = new Set(results.filter((result) => result.versions.vttest).map((result) => result.versions.vttest)).size;
assert(versionCount === matrix.filter((entry) => entry.vttestPattern).length,
  `vttest matrix did not exercise distinct builds: ${JSON.stringify(results.map((result) => result.versions.vttest))}`);
console.log(JSON.stringify({
  verifiedTerminalSystems: results,
  screenshots: [
    `${screenshotPrefix}-fullscreen-vim.png`,
    `${screenshotPrefix}-vt100-screen.png`,
  ],
}, null, 2));

async function runFullScreenProgram(page, entry, spec, screenshot) {
  const primaryMarker = `PORTMATE PRIMARY SCREEN ${entry.name} ${spec.name}`;
  await resetTerminal(page, primaryMarker);
  const processState = spawnPty(entry, spec.command, 12_000);
  activePty = processState;
  try {
    const activeSnapshot = await waitForRenderedMarker(page, processState, spec.marker, 8_000);
    assert(activeSnapshot.nonEmptyLines >= 2,
      `${entry.name}/${spec.name} produced a blank terminal frame: ${JSON.stringify(activeSnapshot)}`);
    if (screenshot) await page.screenshot({ path: screenshot, fullPage: true });
    processState.stdin.write(spec.exit);
    await processState.done;
    await processState.writeQueue;
    assert(processState.status === 0,
      `${entry.name}/${spec.name} exited with ${processState.status ?? processState.signal}: ${processState.stderr}`);
    assert(processState.bytes > 0 && processState.bytes <= maxPtyBytes,
      `${entry.name}/${spec.name} emitted an invalid byte count: ${processState.bytes}`);
    const restored = await terminalSnapshot(page);
    const restoresPrimaryScreen = spec.restoresPrimaryScreen !== false;
    if (restoresPrimaryScreen) {
      assert(restored.text.includes(primaryMarker),
        `${entry.name}/${spec.name} did not restore the primary screen: ${restored.serialized.slice(-2_000)}`);
    } else if (spec.restoresCursor !== false) {
      assert(processState.raw.includes("\x1b[?25h"),
        `${entry.name}/${spec.name} did not restore the cursor after updating the primary screen`);
    } else {
      assert(processState.raw.includes("\x1b[0m"),
        `${entry.name}/${spec.name} did not reset terminal attributes on exit`);
    }
    return {
      name: spec.name,
      bytes: processState.bytes,
      terminalReplyBytes: processState.terminalReplyBytes,
      alternateScreen: activeSnapshot.alternate,
      restoredPrimaryScreen: restoresPrimaryScreen,
      restoredCursor: restoresPrimaryScreen || spec.restoresCursor !== false,
    };
  } finally {
    activePty = null;
    processState.cleanup();
  }
}

async function runVttestSuite(page, entry, suite, screenshot) {
  await resetTerminal(page, `PORTMATE VTTEST ${entry.name} ${suite.name}`);
  const processState = spawnPty(entry, "vttest -q 24x80.132", 25_000);
  activePty = processState;
  const choices = [...suite.choices];
  const handledPrompts = new Set();
  const checkpoints = [];
  let holdCount = 0;
  let menuCount = 0;
  let screenshotCaptured = false;
  let automationQueue = Promise.resolve();

  const scanPrompts = () => {
    const start = Math.max(0, processState.raw.length - processState.lastChunkLength - 96);
    const tail = processState.raw.slice(start);
    const expression = /Push <RETURN>|Enter choice number \(0 - \d+\):/g;
    for (const match of tail.matchAll(expression)) {
      const absoluteIndex = start + match.index;
      if (handledPrompts.has(absoluteIndex)) continue;
      handledPrompts.add(absoluteIndex);
      automationQueue = automationQueue.then(async () => {
        await processState.writeQueue;
        const snapshot = await terminalSnapshot(page);
        checkpoints.push({
          alternate: snapshot.alternate,
          nonEmptyLines: snapshot.nonEmptyLines,
          text: snapshot.text.slice(-1_000),
        });
        if (screenshot && !screenshotCaptured && processState.raw.includes(suite.marker)) {
          await page.screenshot({ path: screenshot, fullPage: true });
          screenshotCaptured = true;
        }
        if (match[0] === "Push <RETURN>") {
          holdCount += 1;
          processState.stdin.write("\r");
        } else {
          menuCount += 1;
          processState.stdin.write(`${choices.length ? choices.shift() : 0}\r`);
        }
      });
    }
  };
  processState.onOutput = scanPrompts;
  scanPrompts();

  try {
    await processState.done;
    await automationQueue;
    await processState.writeQueue;
    assert(processState.status === 0,
      `${entry.name}/${suite.name} exited with ${processState.status ?? processState.signal}: ${processState.stderr}\n${processState.raw.slice(-2_000)}`);
    assert(choices.length === 0,
      `${entry.name}/${suite.name} did not reach every planned menu: ${JSON.stringify(choices)}`);
    assert(processState.raw.includes(suite.marker),
      `${entry.name}/${suite.name} did not exercise ${JSON.stringify(suite.marker)}\n${processState.raw.slice(-2_000)}`);
    assert(holdCount >= suite.minimumHolds,
      `${entry.name}/${suite.name} stopped after ${holdCount} holds, expected at least ${suite.minimumHolds}\n${processState.raw.slice(-2_000)}`);
    assert(menuCount >= suite.choices.length + 1,
      `${entry.name}/${suite.name} did not unwind through its menus: ${menuCount}\n${processState.raw.slice(-2_000)}`);
    assert(checkpoints.some((checkpoint) => checkpoint.nonEmptyLines >= 2),
      `${entry.name}/${suite.name} only rendered blank checkpoints`);
    if (suite.requireReply) {
      assert(processState.terminalReplyBytes > 0,
        `${entry.name}/${suite.name} did not receive any terminal-generated report`);
    }
    if (screenshot) {
      assert(screenshotCaptured, `${entry.name}/${suite.name} did not capture its target render frame`);
    }
    return {
      name: suite.name,
      bytes: processState.bytes,
      holds: holdCount,
      menus: menuCount,
      terminalReplyBytes: processState.terminalReplyBytes,
      renderedCheckpoints: checkpoints.length,
    };
  } finally {
    activePty = null;
    processState.cleanup();
  }
}

function spawnPty(entry, command, timeoutMs) {
  const container = `portmate-terminal-compat-${randomUUID()}`;
  const child = spawn("docker", [
    "run", "--rm", "--interactive", "--name", container,
    "--env", "TERM=xterm-256color",
    "--env", "LANG=C",
    "--env", "LC_ALL=C",
    "--entrypoint", "script",
    entry.image,
    "-qfec", `stty rows 24 cols 80; exec ${command}`, "/dev/null",
  ], { cwd: projectRoot, stdio: ["pipe", "pipe", "pipe"] });
  const state = {
    child,
    stdin: child.stdin,
    raw: "",
    stderr: "",
    bytes: 0,
    terminalReplyBytes: 0,
    lastChunkLength: 0,
    status: null,
    signal: null,
    timedOut: false,
    writeQueue: Promise.resolve(),
    onOutput: null,
    cleanup: () => run("docker", ["rm", "--force", container], { quiet: true, allowFailure: true }),
  };
  const timer = setTimeout(() => {
    state.timedOut = true;
    child.kill("SIGTERM");
  }, timeoutMs);
  state.done = new Promise((resolveDone, rejectDone) => {
    child.once("error", rejectDone);
    child.stderr.on("data", (chunk) => { state.stderr += chunk.toString(); });
    child.stdout.on("data", (chunk) => {
      state.bytes += chunk.length;
      if (state.timedOut) {
        rejectDone(new Error(`${entry.name} PTY command timed out after ${timeoutMs} ms: ${command}\n${state.raw.slice(-2_000)}`));
      } else if (state.bytes > maxPtyBytes) {
        child.kill("SIGTERM");
        return;
      }
      const text = chunk.toString("utf8");
      state.lastChunkLength = text.length;
      state.raw += text;
      state.writeQueue = state.writeQueue.then(() => globalThis.__terminalPage.evaluate((data) => (
        globalThis.__portmateTerminalHarness.write(data)
      ), text));
      state.onOutput?.();
    });
    child.once("close", (status, signal) => {
      clearTimeout(timer);
      state.status = status;
      state.signal = signal;
      if (state.bytes > maxPtyBytes) {
        rejectDone(new Error(`${entry.name} PTY output exceeded ${maxPtyBytes} bytes`));
      } else {
        resolveDone();
      }
    });
  });
  return state;
}

async function resetTerminal(page, marker) {
  globalThis.__terminalPage = page;
  await page.evaluate((primaryMarker) => {
    globalThis.__portmateTerminalHarness.reset();
    return globalThis.__portmateTerminalHarness.write(`${primaryMarker}\r\n`);
  }, marker);
}

async function waitForRenderedMarker(page, processState, marker, timeoutMs) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    await processState.writeQueue;
    const rawMatch = typeof marker === "string" ? processState.raw.includes(marker) : marker.test(processState.raw);
    const snapshot = await terminalSnapshot(page);
    const renderedMatch = typeof marker === "string" ? snapshot.text.includes(marker) : marker.test(snapshot.text);
    if (rawMatch && renderedMatch) return snapshot;
    if (processState.status !== null) break;
    await new Promise((resolveWait) => setTimeout(resolveWait, 40));
  }
  throw new Error(`timed out waiting for terminal marker ${String(marker)}; raw=${processState.raw.slice(-2_000)}`);
}

function terminalSnapshot(page) {
  return page.evaluate(() => globalThis.__portmateTerminalHarness.snapshot());
}

function inspectVersions(entry) {
  const versions = {
    vim: dockerShell(entry, "vim --version | sed -n '1p'").trim(),
    less: dockerShell(entry, "less --version | sed -n '1p'").trim(),
    top: dockerShell(entry, "top -V 2>&1 | sed -n '1p'").trim(),
    dialog: dockerShell(entry, "dialog --version 2>&1 | sed -n '1p'").trim(),
    vttest: null,
  };
  if (entry.vttestPattern) {
    versions.vttest = dockerShell(entry, "vttest -V").trim();
    assert(new RegExp(entry.vttestPattern).test(versions.vttest),
      `${entry.name} returned unexpected vttest version ${JSON.stringify(versions.vttest)}`);
  }
  return versions;
}

function dockerShell(entry, command) {
  return run("docker", ["run", "--rm", "--entrypoint", "sh", entry.image, "-lc", command], { capture: true }).stdout;
}

function validateEntry(entry) {
  if (!entry || typeof entry.name !== "string" || !/^[a-z0-9.-]+$/.test(entry.name)) {
    throw new Error(`Invalid terminal matrix entry: ${JSON.stringify(entry)}`);
  }
  if (typeof entry.dockerfile !== "string" || !entry.dockerfile.startsWith("tests/compat/")) {
    throw new Error(`Invalid terminal matrix Dockerfile: ${JSON.stringify(entry)}`);
  }
  if (entry.vttestPattern !== null && typeof entry.vttestPattern !== "string") {
    throw new Error(`Invalid vttest version pattern for ${entry.name}`);
  }
  for (const [name, value] of Object.entries(entry.buildArgs ?? {})) {
    if (!/^[A-Z][A-Z0-9_]*$/.test(name) || typeof value !== "string" || !/^[a-zA-Z0-9._-]+$/.test(value)) {
      throw new Error(`Invalid terminal matrix build argument in ${entry.name}`);
    }
  }
}

async function reservePort() {
  const server = createServer();
  await new Promise((resolveListen, rejectListen) => {
    server.once("error", rejectListen);
    server.listen(0, "127.0.0.1", resolveListen);
  });
  const address = server.address();
  const selected = typeof address === "object" && address ? address.port : 0;
  await new Promise((resolveClose, rejectClose) => server.close((error) => error ? rejectClose(error) : resolveClose()));
  if (!selected) throw new Error("failed to reserve a Vite port");
  return selected;
}

async function waitForServer(url, output) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // Vite is still starting.
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 100));
  }
  throw new Error(`timed out waiting for ${url}\n${output()}`);
}

function waitForExit(child, timeoutMs) {
  if (child.exitCode !== null) return Promise.resolve();
  return new Promise((resolveExit) => {
    const timer = setTimeout(resolveExit, timeoutMs);
    child.once("exit", () => {
      clearTimeout(timer);
      resolveExit();
    });
  });
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: process.env,
    encoding: "utf8",
    stdio: options.capture || options.quiet ? ["ignore", "pipe", "pipe"] : "inherit",
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.error && !options.allowFailure) throw result.error;
  if (result.status !== 0 && !options.allowFailure) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status ?? 1}${details ? `\n${details}` : ""}`);
  }
  return result;
}
