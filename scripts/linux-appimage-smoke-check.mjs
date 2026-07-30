import { spawnSync } from "node:child_process";
import {
  accessSync,
  constants,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

if (process.platform !== "linux") throw new Error("The AppImage smoke check requires a Linux host");
if (!process.env.DISPLAY?.trim()) throw new Error("The AppImage smoke check requires an active X11 display");

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const packageJson = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8"));
const architecture = process.arch === "x64" ? "amd64" : process.arch;
const appImage = resolve(
  process.env.PORTMATE_APPIMAGE_PATH?.trim()
    ?? join(projectRoot, "target", "release", "bundle", "appimage", `PortMate_${packageJson.version}_${architecture}.AppImage`),
);
assertExecutable(appImage);

const testRoot = mkdtempSync(join(tmpdir(), "portmate-appimage-smoke-"));
const extractRoot = join(testRoot, "package");
mkdirSync(extractRoot, { recursive: true, mode: 0o700 });

try {
  run(appImage, ["--appimage-extract"], { cwd: extractRoot, quiet: true, maxBuffer: 64 * 1024 * 1024 });
  const appRun = join(extractRoot, "squashfs-root", "AppRun");
  assertExecutable(appRun);

  const first = runNativeSmoke(appRun, "first");
  const second = runNativeSmoke(appRun, "second");
  if (first.lifecycle.store.bytes !== second.lifecycle.store.bytes
    || first.lifecycle.store.sha256 !== second.lifecycle.store.sha256) {
    throw new Error("The packaged application changed its Store during a no-op restart");
  }
  if (first.lifecycle.endpointCredentialSha256 === second.lifecycle.endpointCredentialSha256) {
    throw new Error("The packaged application reused its IPC credential across restarts");
  }
  const endpointPath = join(testRoot, "data", "dev.portmate.desktop", "portmate-ipc.json");
  if (existsSync(endpointPath)) throw new Error("The packaged application left its IPC endpoint after restart smoke");

  console.log(JSON.stringify({
    appImage,
    launches: 2,
    store: second.lifecycle.store,
    endpointCredentialRotated: true,
    endpointAddressRotated: first.lifecycle.endpointAddress !== second.lifecycle.endpointAddress,
    first: summarize(first),
    second: summarize(second),
  }, null, 2));
} finally {
  rmSync(testRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}

function runNativeSmoke(appRun, label) {
  const result = spawnSync(process.execPath, [join(projectRoot, "scripts", "linux-desktop-smoke-check.mjs")], {
    cwd: projectRoot,
    env: {
      ...process.env,
      PORTMATE_NATIVE_SMOKE_BINARY: appRun,
      PORTMATE_NATIVE_SMOKE_ROOT: testRoot,
    },
    encoding: "utf8",
    maxBuffer: 2 * 1024 * 1024,
    timeout: 240_000,
  });
  if (result.error || result.status !== 0) {
    const diagnostic = [result.stdout, result.stderr].filter(Boolean).join("\n").trim().slice(-12_000);
    throw new Error(`${label} packaged launch failed${diagnostic ? `:\n${diagnostic}` : ""}`, { cause: result.error });
  }
  let report;
  try {
    report = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`${label} packaged launch returned an invalid smoke report`, { cause: error });
  }
  if (report.launch !== "packaged"
    || report.lifecycle?.gracefulExit !== true
    || report.lifecycle?.endpointPublished !== true
    || report.lifecycle?.endpointRemoved !== true
    || !/^[0-9a-f]{64}$/.test(report.lifecycle?.store?.sha256)
    || !/^[0-9a-f]{64}$/.test(report.lifecycle?.endpointCredentialSha256)) {
    throw new Error(`${label} packaged launch omitted required lifecycle evidence`);
  }
  return report;
}

function summarize(report) {
  return {
    window: report.window,
    renderer: report.renderer,
    pixels: report.pixels,
    endpointUsesTokenRef: report.lifecycle.endpointUsesTokenRef,
    endpointAddress: report.lifecycle.endpointAddress,
  };
}

function assertExecutable(path) {
  const metadata = statSync(path);
  if (!metadata.isFile()) throw new Error(`Expected executable file: ${path}`);
  accessSync(path, constants.X_OK);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? projectRoot,
    env: process.env,
    encoding: "utf8",
    maxBuffer: options.maxBuffer ?? 16 * 1024 * 1024,
  });
  if (!options.quiet && result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error || result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status ?? 1}`, { cause: result.error });
  }
}
