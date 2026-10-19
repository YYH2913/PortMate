import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

export function nativeTestExecutables(messages) {
  const executables = new Set();
  for (const line of messages.split(/\r?\n/u)) {
    let message;
    try { message = JSON.parse(line); } catch { continue; }
    if (message.reason === "compiler-artifact"
        && message.profile?.test === true
        && typeof message.executable === "string"
        && message.executable.toLowerCase().endsWith(".exe")) {
      executables.add(message.executable);
    }
  }
  return [...executables];
}

export function nativeTestDiagnosticBuildArguments(args) {
  const separator = args.indexOf("--");
  return [
    ...args.slice(0, separator < 0 ? undefined : separator),
    "--no-run", "--message-format=json-render-diagnostics",
  ];
}

export function collectWindowsLoaderDiagnostics(args, projectRoot) {
  const directory = resolve(projectRoot, "target/native-ci/windows-loader");
  mkdirSync(directory, { recursive: true });
  // The original test already failed; this cached no-run build only discovers
  // binaries for read-only loader diagnostics and never changes its exit code.
  const build = spawnSync("cargo", nativeTestDiagnosticBuildArguments(args), {
    cwd: projectRoot, encoding: "utf8", windowsHide: true,
    timeout: 300_000, maxBuffer: 32 * 1024 * 1024,
  });
  writeFileSync(resolve(directory, "build.stderr.log"), build.stderr ?? "");
  if (build.error || build.status !== 0) {
    throw new Error(`loader diagnostic no-run build failed: ${build.error?.message ?? build.status}`);
  }
  const executables = nativeTestExecutables(build.stdout);
  if (executables.length === 0) throw new Error("no Windows test executables found for loader diagnostics");
  const manifest = resolve(directory, "executables.json");
  writeFileSync(manifest, JSON.stringify(executables, null, 2));
  const script = fileURLToPath(new URL("./windows-loader-diagnostics.ps1", import.meta.url));
  const probe = spawnSync("pwsh", [
    "-NoProfile", "-File", script, "-Manifest", manifest, "-OutputDirectory", directory,
  ], { cwd: projectRoot, stdio: "inherit", windowsHide: true, timeout: 120_000 });
  if (probe.error || probe.status !== 0) {
    throw new Error(`Windows loader diagnostic failed: ${probe.error?.message ?? probe.status}`);
  }
}

export function prepareWindowsNativeTestManifests(args, projectRoot) {
  // tauri-build links its application resource only into bin targets, not the
  // library unit-test executable that also imports TaskDialogIndirect.
  const build = spawnSync("cargo", nativeTestDiagnosticBuildArguments(args), {
    cwd: projectRoot, encoding: "utf8", windowsHide: true,
    maxBuffer: 32 * 1024 * 1024,
  });
  if (build.stderr) process.stderr.write(build.stderr);
  if (build.error || build.status !== 0) {
    throw new Error(`Windows test prebuild failed: ${build.error?.message ?? build.status}`);
  }
  const executables = nativeTestExecutables(build.stdout);
  if (executables.length === 0) throw new Error("no native Windows test executables were built");
  const manifest = readFileSync(new URL("../fixtures/windows-native-test.manifest", import.meta.url), "utf8");
  for (const executable of executables) {
    writeFileSync(`${executable}.manifest`, manifest);
    console.log(`Prepared Common Controls v6 test activation manifest: ${executable}.manifest`);
  }
}
