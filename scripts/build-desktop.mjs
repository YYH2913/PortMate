import { chmodSync, existsSync, lstatSync, readdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { prepareAppImageBuildEnvironment } from "./appimage-runtime.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
rmSync(join(projectRoot, "target", "release", "bundle"), { recursive: true, force: true });
if (process.platform !== "win32") {
  process.umask(0o022);
  chmodSync(join(projectRoot, "LICENSE"), 0o644);
  for (const entry of readdirSync(join(projectRoot, "src-tauri", "icons"))) {
    const path = join(projectRoot, "src-tauri", "icons", entry);
    if (lstatSync(path).isFile()) chmodSync(path, 0o644);
  }
  for (const binary of ["portmate", "portmate-mcp"]) {
    const path = join(projectRoot, "target", "release", binary);
    if (existsSync(path)) chmodSync(path, 0o755);
  }
}

const appImageRuntime = prepareAppImageBuildEnvironment(process.env, {
  tempRoot: join(projectRoot, "target"),
});
if (appImageRuntime.source === "tauri-cache") {
  console.log(`Reusing the cached Tauri AppImage runtime: ${appImageRuntime.runtimeFile}`);
} else if (appImageRuntime.source === "environment") {
  console.log(`Using LDAI_RUNTIME_FILE: ${appImageRuntime.runtimeFile}`);
} else if (process.platform === "linux") {
  console.log("No cached AppImage plugin was found; linuxdeploy may download its runtime.");
}

try {
  run(process.execPath, [
    join(projectRoot, "node_modules", "@tauri-apps", "cli", "tauri.js"),
    "build",
    "--config",
    "src-tauri/tauri.bundle.conf.json",
  ], appImageRuntime.env);
  run(
    process.execPath,
    [join(projectRoot, "scripts", "finalize-linux-bundle.mjs")],
    appImageRuntime.env,
  );
} catch (error) {
  if (Number.isSafeInteger(error?.exitCode)) {
    process.exitCode = error.exitCode;
  } else {
    throw error;
  }
} finally {
  appImageRuntime.cleanup();
}

function run(command, args, env) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    const error = new Error(`${command} failed with exit code ${result.status ?? 1}`);
    error.exitCode = result.status ?? 1;
    throw error;
  }
}
