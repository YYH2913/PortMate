import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";
import {
  compatibilityUsesCachedImages,
  filterCompatibilityEntries,
  prepareCompatibilityImage,
} from "./compat-docker-images.mjs";

if (process.platform !== "linux") {
  throw new Error("The tmux version matrix currently requires a Linux Docker host");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const commandTimeoutMs = 300_000;
const useCachedImages = compatibilityUsesCachedImages();
const fieldSeparator = "|PORTMATE:8f41c2d7:|";
const allEntries = JSON.parse(readFileSync(resolve(projectRoot, "tests/compat/tmux-version-matrix.json"), "utf8"));
if (!Array.isArray(allEntries) || allEntries.length < 4) throw new Error("tmux version matrix must contain at least four entries");
allEntries.forEach(validateEntry);
const matrix = filterCompatibilityEntries(allEntries);

run("docker", ["info", "--format", "{{.ServerVersion}}"], { quiet: true });
run("cargo", ["build", "--locked", "-p", "portmate", "--bin", "tmux-compat-probe"]);
const probe = resolve(projectRoot, "target/debug", process.platform === "win32" ? "tmux-compat-probe.exe" : "tmux-compat-probe");
const results = [];

for (const entry of matrix) {
  const image = `portmate-compat-${entry.name}:local`;
  const buildArgs = Object.entries(entry.buildArgs).flatMap(([name, value]) => ["--build-arg", `${name}=${value}`]);
  await prepareCompatibilityImage({
    run,
    image,
    useCachedImages,
    buildArgs: ["build", "--tag", image, "--file", resolve(projectRoot, entry.dockerfile), ...buildArgs, projectRoot],
  });
  const container = `portmate-tmux-compat-${randomUUID()}`;
  try {
    run("docker", ["run", "--detach", "--rm", "--name", container, image], { quiet: true });
    const version = dockerExec(container, "tmux -V").trim();
    if (!new RegExp(entry.versionPattern).test(version)) {
      throw new Error(`${entry.name} returned unexpected version ${JSON.stringify(version)}`);
    }

    dockerExec(container, [
      "tmux -L portmate -f /dev/null new-session -d -s 'lab alpha' -n main 'sleep 300'",
      "tmux -L portmate split-window -d -h -t 'lab alpha:0' 'sleep 300'",
      "tmux -L portmate set-option -w -t 'lab alpha:0' synchronize-panes on",
      "tmux -L portmate select-layout -t 'lab alpha:0' tiled",
      "tmux -L portmate rename-window -t 'lab alpha:0' 'primary pane'",
      "tmux -L portmate new-session -d -s aux -n monitor 'sleep 300'",
      "tmux -L portmate rename-session -t 'lab alpha' 'lab-renamed'",
      "tmux -L portmate resize-pane -t 'lab-renamed:0.0' -R 1",
    ].join(" && "));

    const control = dockerExec(container,
      "(sleep 0.2; tmux -L portmate new-window -d -t lab-renamed -n control 'sleep 300') & "
      + "tail -f /dev/null | timeout 2 tmux -L portmate -C attach-session -t lab-renamed; "
      + "status=$?; test $status -eq 0 -o $status -eq 124");
    const sessions = dockerExec(container,
      `tmux -L portmate list-sessions -F '#{session_name}${fieldSeparator}#{session_windows}${fieldSeparator}#{session_attached}${fieldSeparator}#{session_created}'`);
    const windows = dockerExec(container,
      `tmux -L portmate list-windows -a -F '#{session_name}${fieldSeparator}#{window_index}${fieldSeparator}#{window_id}${fieldSeparator}#{window_name}${fieldSeparator}#{window_panes}${fieldSeparator}#{window_active}'`);
    const panes = dockerExec(container,
      `tmux -L portmate list-panes -a -F '#{session_name}${fieldSeparator}#{window_index}${fieldSeparator}#{pane_index}${fieldSeparator}#{pane_id}${fieldSeparator}#{pane_active}${fieldSeparator}#{pane_current_command}${fieldSeparator}#{pane_title}${fieldSeparator}#{pane_synchronized}'`);
    const parsed = run(probe, [], {
      capture: true,
      input: JSON.stringify({ sessions, windows, panes, control }),
    });
    const state = JSON.parse(parsed.stdout);
    assert(state.sessions.length === 2, `${entry.name} session parsing failed: ${parsed.stdout}`);
    assert(state.sessions.some((session) => session.name === "lab-renamed" && session.windows === 2),
      `${entry.name} session rename/window count failed: ${parsed.stdout}`);
    assert(state.windows.some((window) => window.session === "lab-renamed" && window.name === "primary pane" && window.panes === 2),
      `${entry.name} window parsing failed: ${parsed.stdout}`);
    assert(state.windows.some((window) => window.session === "lab-renamed" && window.name === "primary pane" && window.synchronized),
      `${entry.name} synchronize-panes parsing failed: ${parsed.stdout}`);
    assert(state.panes.filter((pane) => pane.session === "lab-renamed").length >= 3,
      `${entry.name} pane parsing failed: ${parsed.stdout}`);
    assert(state.controlChanged && state.lastControlEvent,
      `${entry.name} control-mode notifications were not recognized: ${JSON.stringify({ control, state })}`);
    assert(state.protocolCommandCount === 21 && state.boundedErrorCharacters === 515,
      `${entry.name} shared protocol helpers were not exercised: ${parsed.stdout}`);
    results.push({
      name: entry.name,
      version,
      sessions: state.sessions.length,
      windows: state.windows.length,
      panes: state.panes.length,
      controlEvent: state.lastControlEvent,
    });
  } finally {
    run("docker", ["rm", "--force", container], { quiet: true, allowFailure: true });
  }
}

assert(new Set(results.map((result) => result.version)).size === matrix.length,
  `tmux matrix did not exercise distinct versions: ${JSON.stringify(results)}`);
console.log(JSON.stringify({ verifiedTmuxVersions: results }, null, 2));

function dockerExec(container, command) {
  return run("docker", ["exec", container, "sh", "-lc", command], { capture: true }).stdout;
}

function validateEntry(entry) {
  if (!entry || typeof entry.name !== "string" || !/^[a-z0-9.-]+$/.test(entry.name)) {
    throw new Error(`Invalid tmux matrix entry: ${JSON.stringify(entry)}`);
  }
  if (typeof entry.dockerfile !== "string" || !entry.dockerfile.startsWith("tests/compat/")) {
    throw new Error(`Invalid tmux Dockerfile: ${JSON.stringify(entry)}`);
  }
  if (typeof entry.versionPattern !== "string") throw new Error(`Invalid tmux version pattern for ${entry.name}`);
  for (const [name, value] of Object.entries(entry.buildArgs ?? {})) {
    if (!/^[A-Z][A-Z0-9_]*$/.test(name) || typeof value !== "string" || !/^[a-zA-Z0-9._-]+$/.test(value)) {
      throw new Error(`Invalid tmux build argument in ${entry.name}`);
    }
  }
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: process.env,
    encoding: "utf8",
    input: options.input,
    stdio: options.capture || options.quiet ? [options.input === undefined ? "ignore" : "pipe", "pipe", "pipe"] : "inherit",
    maxBuffer: 16 * 1024 * 1024,
    timeout: options.timeout ?? commandTimeoutMs,
  });
  if (result.error && !options.allowFailure) {
    if (result.error.code === "ETIMEDOUT") {
      throw new Error(`${command} ${args.join(" ")} exceeded its ${options.timeout ?? commandTimeoutMs} ms timeout`);
    }
    throw result.error;
  }
  if (result.status !== 0 && !options.allowFailure) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status ?? 1}${details ? `\n${details}` : ""}`);
  }
  return result;
}
