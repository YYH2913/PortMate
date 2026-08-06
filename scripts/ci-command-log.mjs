import { createWriteStream, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { finished } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import crossSpawn from "cross-spawn";

export function parseLoggedCommandArguments(argv) {
  if (!Array.isArray(argv) || argv.length < 2) {
    throw new Error("Usage: node scripts/ci-command-log.mjs <log-path> <command> [args...]");
  }
  const [rawLogPath, rawCommand, ...args] = argv;
  if (typeof rawLogPath !== "string" || !rawLogPath.trim() || rawLogPath.includes("\0")) {
    throw new Error("CI command log path must be a non-empty path without NUL");
  }
  if (typeof rawCommand !== "string" || !rawCommand.trim() || rawCommand.includes("\0")) {
    throw new Error("CI command must be a non-empty string without NUL");
  }
  if (args.some((arg) => typeof arg !== "string" || arg.includes("\0"))) {
    throw new Error("CI command arguments must be strings without NUL");
  }
  return {
    logPath: resolve(rawLogPath),
    command: rawCommand,
    args,
  };
}

export async function runLoggedCommand(argv, options = {}) {
  const { logPath, command, args } = parseLoggedCommandArguments(argv);
  mkdirSync(dirname(logPath), { recursive: true });
  const log = createWriteStream(logPath, { flags: "w", mode: 0o600 });
  const child = crossSpawn(command, args, {
    cwd: options.cwd ?? process.cwd(),
    env: options.env ?? process.env,
    stdio: ["inherit", "pipe", "pipe"],
    windowsHide: true,
  });

  const signalHandlers = ["SIGINT", "SIGTERM"].map((signal) => [
    signal,
    () => {
      if (child.exitCode === null && child.signalCode === null) child.kill(signal);
    },
  ]);
  for (const [signal, handler] of signalHandlers) process.once(signal, handler);

  for (const [stream, destination] of [
    [child.stdout, process.stdout],
    [child.stderr, process.stderr],
  ]) {
    stream.on("data", (chunk) => {
      destination.write(chunk);
      log.write(chunk);
    });
  }

  let result;
  try {
    result = await new Promise((resolveResult) => {
      let spawnError;
      child.once("error", (error) => {
        spawnError = error;
      });
      child.once("close", (code, signal) => resolveResult({ code, signal, error: spawnError }));
    });
    if (result.error) {
      const diagnostic = `Failed to start ${command}: ${result.error.message}\n`;
      process.stderr.write(diagnostic);
      log.write(diagnostic);
    }
  } finally {
    for (const [signal, handler] of signalHandlers) process.off(signal, handler);
    log.end();
    await finished(log);
  }
  return result;
}

async function main() {
  const result = await runLoggedCommand(process.argv.slice(2));
  if (result.error || result.signal) {
    process.exitCode = 1;
  } else {
    process.exitCode = result.code ?? 1;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
