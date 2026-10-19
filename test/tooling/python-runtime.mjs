import { spawnSync } from "node:child_process";

export function findMcpPython({
  projectRoot,
  environment = process.env,
  platform = process.platform,
  run = spawnSync,
}) {
  const configured = environment.PORTMATE_PYTHON?.trim();
  const candidates = configured
    ? [{ command: configured, args: [] }]
    : platform === "win32"
      // setup-python and an activated virtual environment select Python via
      // PATH. py -3 instead picks the newest globally registered interpreter.
      ? [{ command: "python", args: [] }, { command: "py", args: ["-3"] }]
      : [{ command: "python3", args: [] }, { command: "python", args: [] }];
  for (const candidate of candidates) {
    const probe = run(candidate.command, [
      ...candidate.args, "-c", "import sys; raise SystemExit(sys.version_info < (3, 10))",
    ], {
      cwd: projectRoot,
      env: environment,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 10_000,
    });
    if (!probe.error && !probe.signal && probe.status === 0) return candidate;
  }
  throw new Error("MCP Python SDK checks require Python 3.10 or newer (set PORTMATE_PYTHON to override discovery)");
}
