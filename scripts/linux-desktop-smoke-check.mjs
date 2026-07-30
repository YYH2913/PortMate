import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  accessSync,
  constants,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const STARTUP_TIMEOUT_MS = boundedInteger(process.env.PORTMATE_NATIVE_SMOKE_TIMEOUT_MS, 180_000, 10_000, 600_000);
const RENDER_TIMEOUT_MS = 30_000;
const IPC_STARTUP_TIMEOUT_MS = 30_000;
const GRACEFUL_EXIT_TIMEOUT_MS = 15_000;
const POLL_INTERVAL_MS = 250;
const MAX_CAPTURE_BYTES = 128 * 1024 * 1024;
const MAX_LOG_BYTES = 256 * 1024;
const MIN_WINDOW_WIDTH = 480;
const MIN_WINDOW_HEIGHT = 320;
const MIN_UNIQUE_COLORS = 64;
const MIN_LUMA_DEVIATION = 4;

if (process.platform !== "linux") {
  throw new Error("The native desktop smoke check requires a Linux host");
}
if (!process.env.DISPLAY?.trim()) {
  throw new Error("The native desktop smoke check requires an active X11 display in DISPLAY");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const xwininfo = executablePath("xwininfo");
const xwd = executablePath("xwd");
const wmctrl = executablePath("wmctrl");
const configuredBinary = process.env.PORTMATE_NATIVE_SMOKE_BINARY?.trim();
const launchCommand = configuredBinary ? resolve(configuredBinary) : executablePath("npm");
const launchArguments = configuredBinary ? [] : ["run", "desktop:clean"];
const launchKind = configuredBinary ? "packaged" : "development";
if (configuredBinary) assertExecutablePath(launchCommand);
const existingWindowIds = new Set(listPortMateWindows(xwininfo).map((window) => window.id));
const dmiIdentity = readDmiIdentity();
const expectsVmwareFallback = dmiIdentity.some((value) => value.toLowerCase().includes("vmware"));
const configuredRoot = process.env.PORTMATE_NATIVE_SMOKE_ROOT?.trim();
const testRoot = configuredRoot ? resolve(configuredRoot) : mkdtempSync(join(tmpdir(), "portmate-native-smoke-"));
const ownsTestRoot = !configuredRoot;
if (configuredRoot) assertTemporaryTestRoot(testRoot);
const xdgDirectories = Object.fromEntries([
  ["XDG_CACHE_HOME", "cache"],
  ["XDG_CONFIG_HOME", "config"],
  ["XDG_DATA_HOME", "data"],
  ["XDG_RUNTIME_DIR", "runtime"],
  ["XDG_STATE_HOME", "state"],
].map(([name, directory]) => [name, join(testRoot, directory)]));
for (const path of Object.values(xdgDirectories)) mkdirSync(path, { recursive: true, mode: 0o700 });
const environment = { ...process.env, ...xdgDirectories, GDK_BACKEND: "x11" };
delete environment.WEBKIT_DISABLE_DMABUF_RENDERER;
const appDataDirectory = join(xdgDirectories.XDG_DATA_HOME, "dev.portmate.desktop");
const endpointPath = join(appDataDirectory, "portmate-ipc.json");
const storePath = join(appDataDirectory, "portmate-store.sqlite3");

let output = "";
let interruptedSignal = null;
let stopPromise = null;
let desktopResult = null;
const desktop = spawn(launchCommand, launchArguments, {
  cwd: configuredBinary ? testRoot : projectRoot,
  detached: true,
  env: environment,
  stdio: ["ignore", "pipe", "pipe"],
});
const desktopExit = new Promise((resolveExit) => {
  const settle = (result) => {
    desktopResult ??= result;
    resolveExit(desktopResult);
  };
  desktop.once("error", (error) => settle({ error }));
  desktop.once("exit", (code, signal) => settle({ code, signal }));
});
for (const stream of [desktop.stdout, desktop.stderr]) {
  stream.on("data", (chunk) => {
    output = boundedAppend(output, chunk.toString());
  });
}

const handleSignal = (signal) => {
  interruptedSignal ??= signal;
  void stopDesktop();
};
process.once("SIGINT", handleSignal);
process.once("SIGTERM", handleSignal);

let failure = null;
try {
  const deadline = Date.now() + STARTUP_TIMEOUT_MS;
  const window = await waitForWindow(deadline);
  const rendered = await waitForRenderedPixels(window, Math.min(deadline, Date.now() + RENDER_TIMEOUT_MS));
  const endpoint = await waitForIpcEndpoint(Math.min(deadline, Date.now() + IPC_STARTUP_TIMEOUT_MS));
  if (expectsVmwareFallback && !output.includes("disabled WebKit DMABUF rendering for VMware compatibility")) {
    throw new Error("The VMware desktop started without the expected WebKit DMABUF fallback marker");
  }

  const artifactPath = process.env.PORTMATE_NATIVE_SMOKE_XWD?.trim();
  if (artifactPath) writeFileSync(resolve(artifactPath), rendered.capture);
  closeNativeWindow(window.id);
  await waitForGracefulExit(Date.now() + GRACEFUL_EXIT_TIMEOUT_MS);
  await waitForEndpointRemoval(Date.now() + 5_000);
  const store = inspectPersistedStore();
  console.log(JSON.stringify({
    launch: launchKind,
    window: { id: window.id, width: window.width, height: window.height },
    renderer: expectsVmwareFallback ? "automatic VMware DMABUF fallback" : "host default",
    dmiIdentity,
    pixels: rendered.stats,
    lifecycle: {
      gracefulExit: true,
      endpointPublished: true,
      endpointRemoved: true,
      endpointUsesTokenRef: typeof endpoint.tokenRef === "string",
      endpointAddress: endpoint.addr,
      endpointCredentialSha256: createHash("sha256")
        .update(endpoint.tokenRef ?? endpoint.token)
        .digest("hex"),
      store,
    },
    artifact: artifactPath ? resolve(artifactPath) : null,
  }, null, 2));
} catch (error) {
  const diagnostic = stripAnsi(output).slice(-12_000);
  failure = new Error(`${error instanceof Error ? error.message : String(error)}\n\nDesktop output:\n${diagnostic}`);
} finally {
  process.off("SIGINT", handleSignal);
  process.off("SIGTERM", handleSignal);
  await stopDesktop();
  if (ownsTestRoot) rmSync(testRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
if (interruptedSignal) process.kill(process.pid, interruptedSignal);
if (failure) throw failure;

async function waitForWindow(deadline) {
  while (Date.now() < deadline) {
    if (interruptedSignal) throw new Error(`Native desktop smoke check interrupted by ${interruptedSignal}`);
    if (desktopResult) throw desktopExitError(desktopResult);
    const candidates = listPortMateWindows(xwininfo)
      .filter((window) => !existingWindowIds.has(window.id))
      .filter((window) => window.width >= MIN_WINDOW_WIDTH && window.height >= MIN_WINDOW_HEIGHT)
      .sort((left, right) => right.width * right.height - left.width * left.height);
    if (candidates[0]) return candidates[0];
    await delay(POLL_INTERVAL_MS);
  }
  throw new Error(`PortMate did not create a usable native window within ${STARTUP_TIMEOUT_MS} ms`);
}

async function waitForRenderedPixels(window, deadline) {
  let lastFailure = "no capture was available";
  while (Date.now() < deadline) {
    if (interruptedSignal) throw new Error(`Native desktop smoke check interrupted by ${interruptedSignal}`);
    if (desktopResult) throw desktopExitError(desktopResult);
    const result = spawnSync(xwd, ["-silent", "-id", window.id], {
      encoding: null,
      maxBuffer: MAX_CAPTURE_BYTES,
    });
    if (result.status === 0 && result.stdout?.length) {
      try {
        const stats = inspectXwd(result.stdout);
        const failure = renderFailure(stats);
        if (!failure) return { capture: result.stdout, stats };
        lastFailure = failure;
      } catch (error) {
        lastFailure = error instanceof Error ? error.message : String(error);
      }
    } else {
      lastFailure = result.stderr?.toString().trim() || `xwd exited with ${result.status}`;
    }
    await delay(POLL_INTERVAL_MS);
  }
  throw new Error(`PortMate's native window never produced a rendered frame: ${lastFailure}`);
}

async function waitForIpcEndpoint(deadline) {
  let lastFailure = "the endpoint was not published";
  while (Date.now() < deadline) {
    if (interruptedSignal) throw new Error(`Native desktop smoke check interrupted by ${interruptedSignal}`);
    if (desktopResult) throw desktopExitError(desktopResult);
    if (existsSync(endpointPath)) {
      try {
        const endpoint = JSON.parse(readFileSync(endpointPath, "utf8"));
        if (endpoint.storePath !== storePath) throw new Error("IPC endpoint points outside the isolated Store");
        if (!/^127\.0\.0\.1:\d+$/.test(endpoint.addr)) throw new Error("IPC endpoint is not loopback TCP");
        const hasTokenRef = typeof endpoint.tokenRef === "string" && endpoint.tokenRef.startsWith("keychain:ipc-");
        const hasFallbackToken = typeof endpoint.token === "string" && endpoint.token.length >= 16;
        if (hasTokenRef === hasFallbackToken) throw new Error("IPC endpoint must contain exactly one token representation");
        return endpoint;
      } catch (error) {
        lastFailure = error instanceof Error ? error.message : String(error);
      }
    }
    await delay(POLL_INTERVAL_MS);
  }
  throw new Error(`PortMate did not publish a valid IPC endpoint: ${lastFailure}`);
}

function closeNativeWindow(windowId) {
  const result = spawnSync(wmctrl, ["-i", "-c", windowId], { encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw new Error(result.stderr?.trim() || `wmctrl could not close ${windowId}`);
  }
}

async function waitForGracefulExit(deadline) {
  while (Date.now() < deadline) {
    if (desktopResult) {
      if (desktopResult.error) throw desktopResult.error;
      if (desktopResult.code !== 0 || desktopResult.signal) throw desktopExitError(desktopResult);
      return;
    }
    await delay(POLL_INTERVAL_MS);
  }
  throw new Error(`PortMate did not exit after WM_DELETE_WINDOW within ${GRACEFUL_EXIT_TIMEOUT_MS} ms`);
}

async function waitForEndpointRemoval(deadline) {
  while (Date.now() < deadline) {
    if (!existsSync(endpointPath)) return;
    await delay(POLL_INTERVAL_MS);
  }
  throw new Error("PortMate left its MCP IPC endpoint behind after a normal window close");
}

function inspectPersistedStore() {
  const metadata = statSync(storePath);
  if (!metadata.isFile() || metadata.size <= 0) throw new Error("PortMate did not persist a non-empty Store");
  return {
    bytes: metadata.size,
    sha256: createHash("sha256").update(readFileSync(storePath)).digest("hex"),
  };
}

function inspectXwd(buffer) {
  if (!Buffer.isBuffer(buffer) || buffer.length < 100) throw new Error("truncated XWD header");
  const field = (index) => buffer.readUInt32BE(index * 4);
  const headerSize = field(0);
  const version = field(1);
  const format = field(2);
  const width = field(4);
  const height = field(5);
  const byteOrder = field(7);
  const bitsPerPixel = field(11);
  const bytesPerLine = field(12);
  const redMask = field(14);
  const greenMask = field(15);
  const blueMask = field(16);
  const colorCount = field(19);
  if (version !== 7 || format !== 2) throw new Error(`unsupported XWD format ${format}, version ${version}`);
  if (!width || !height || width > 16_384 || height > 16_384) throw new Error(`invalid XWD dimensions ${width}x${height}`);
  if (![24, 32].includes(bitsPerPixel) || ![0, 1].includes(byteOrder)) {
    throw new Error(`unsupported XWD pixel layout: ${bitsPerPixel} bpp, byte order ${byteOrder}`);
  }
  if (!redMask || !greenMask || !blueMask) throw new Error("XWD true-color masks are missing");
  const bytesPerPixel = bitsPerPixel / 8;
  const pixelOffset = headerSize + colorCount * 12;
  const requiredBytes = pixelOffset + bytesPerLine * height;
  if (headerSize < 100 || requiredBytes > buffer.length || bytesPerLine < width * bytesPerPixel) {
    throw new Error("truncated XWD pixel data");
  }

  const step = Math.max(1, Math.ceil(Math.sqrt((width * height) / 400_000)));
  const colors = new Set();
  let samples = 0;
  let darkSamples = 0;
  let brightSamples = 0;
  let coloredSamples = 0;
  let lumaSum = 0;
  let lumaSquaredSum = 0;
  for (let y = 0; y < height; y += step) {
    for (let x = 0; x < width; x += step) {
      const offset = pixelOffset + y * bytesPerLine + x * bytesPerPixel;
      const pixel = readPixel(buffer, offset, bytesPerPixel, byteOrder);
      const red = channelValue(pixel, redMask);
      const green = channelValue(pixel, greenMask);
      const blue = channelValue(pixel, blueMask);
      const luma = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
      samples += 1;
      lumaSum += luma;
      lumaSquaredSum += luma * luma;
      if (luma < 16) darkSamples += 1;
      if (luma > 240) brightSamples += 1;
      if (Math.max(red, green, blue) - Math.min(red, green, blue) > 24) coloredSamples += 1;
      colors.add((red << 16) | (green << 8) | blue);
    }
  }
  const meanLuma = lumaSum / samples;
  return {
    width,
    height,
    samples,
    uniqueColors: colors.size,
    meanLuma: rounded(meanLuma),
    lumaDeviation: rounded(Math.sqrt(Math.max(0, lumaSquaredSum / samples - meanLuma ** 2))),
    darkRatio: rounded(darkSamples / samples),
    brightRatio: rounded(brightSamples / samples),
    coloredRatio: rounded(coloredSamples / samples),
  };
}

function renderFailure(stats) {
  if (stats.width < MIN_WINDOW_WIDTH || stats.height < MIN_WINDOW_HEIGHT) {
    return `window is only ${stats.width}x${stats.height}`;
  }
  if (stats.uniqueColors < MIN_UNIQUE_COLORS) return `only ${stats.uniqueColors} unique colors were rendered`;
  if (stats.lumaDeviation < MIN_LUMA_DEVIATION) return `luminance deviation is only ${stats.lumaDeviation}`;
  if (stats.brightRatio > 0.985) return `${(stats.brightRatio * 100).toFixed(2)}% of pixels are white`;
  if (stats.darkRatio > 0.995) return `${(stats.darkRatio * 100).toFixed(2)}% of pixels are black`;
  return null;
}

function listPortMateWindows(command) {
  const result = spawnSync(command, ["-root", "-tree"], { encoding: "utf8", maxBuffer: 4 * 1024 * 1024 });
  if (result.status !== 0) throw new Error(result.stderr?.trim() || `xwininfo exited with ${result.status}`);
  const windows = [];
  const pattern = /^\s*(0x[0-9a-f]+)\s+"[^"]*":\s+\("portmate"\s+"Portmate"\)\s+(\d+)x(\d+)/gim;
  for (const match of result.stdout.matchAll(pattern)) {
    windows.push({ id: match[1], width: Number(match[2]), height: Number(match[3]) });
  }
  return windows;
}

function readPixel(buffer, offset, bytesPerPixel, byteOrder) {
  if (bytesPerPixel === 4) return byteOrder === 0 ? buffer.readUInt32LE(offset) : buffer.readUInt32BE(offset);
  if (byteOrder === 0) return (buffer[offset] | (buffer[offset + 1] << 8) | (buffer[offset + 2] << 16)) >>> 0;
  return ((buffer[offset] << 16) | (buffer[offset + 1] << 8) | buffer[offset + 2]) >>> 0;
}

function channelValue(pixel, mask) {
  let shift = 0;
  while (shift < 32 && ((mask >>> shift) & 1) === 0) shift += 1;
  const maximum = mask >>> shift;
  const value = (pixel & mask) >>> shift;
  return Math.round(255 * value / maximum);
}

function readDmiIdentity() {
  return ["sys_vendor", "product_name", "board_vendor"]
    .map((name) => {
      try {
        return readFileSync(join("/sys/class/dmi/id", name), "utf8").trim();
      } catch {
        return "";
      }
    })
    .filter(Boolean);
}

function executablePath(name) {
  for (const directory of (process.env.PATH ?? "").split(delimiter)) {
    const candidate = join(directory, name);
    if (!existsSync(candidate)) continue;
    try {
      accessSync(candidate, constants.X_OK);
      return candidate;
    } catch {
      // Continue searching PATH.
    }
  }
  throw new Error(`The native desktop smoke check requires '${name}' in PATH`);
}

function assertExecutablePath(path) {
  const metadata = statSync(path);
  if (!metadata.isFile()) throw new Error(`Native desktop smoke binary is not a file: ${path}`);
  accessSync(path, constants.X_OK);
}

function assertTemporaryTestRoot(path) {
  const temporaryRoot = resolve(tmpdir());
  const fromTemporaryRoot = relative(temporaryRoot, path);
  if (!fromTemporaryRoot || fromTemporaryRoot === ".." || fromTemporaryRoot.startsWith(`..${sep}`)
    || isAbsolute(fromTemporaryRoot) || !/^portmate-(native|appimage)-smoke-/.test(fromTemporaryRoot)) {
    throw new Error(`PORTMATE_NATIVE_SMOKE_ROOT must be a dedicated PortMate directory below ${temporaryRoot}`);
  }
  mkdirSync(path, { recursive: true, mode: 0o700 });
}

function desktopExitError(result) {
  if (result.error) return result.error;
  return new Error(`The desktop process exited before rendering (code ${result.code}, signal ${result.signal})`);
}

function stopDesktop() {
  stopPromise ??= stopDesktopProcess();
  return stopPromise;
}

async function stopDesktopProcess() {
  if (!desktop.pid) return;
  signalProcessGroup("SIGTERM");
  const exited = await Promise.race([desktopExit.then(() => true), delay(5_000).then(() => false)]);
  if (!exited) {
    signalProcessGroup("SIGKILL");
    await Promise.race([desktopExit, delay(2_000)]);
  }
}

function signalProcessGroup(signal) {
  try {
    process.kill(-desktop.pid, signal);
  } catch (error) {
    if (error?.code !== "ESRCH") throw error;
  }
}

function boundedAppend(current, next) {
  const combined = current + next;
  return combined.length <= MAX_LOG_BYTES ? combined : combined.slice(-MAX_LOG_BYTES);
}

function boundedInteger(value, fallback, minimum, maximum) {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= minimum && parsed <= maximum ? parsed : fallback;
}

function rounded(value) {
  return Number(value.toFixed(6));
}

function stripAnsi(value) {
  return value.replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, "");
}

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}
