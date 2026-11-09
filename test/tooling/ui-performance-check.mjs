import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const checks = [
  ["terminal timestamps", "terminal-compat-check.mjs", "PORTMATE_UI_TIMESTAMP_PERF_ONLY"],
  ["long terminal history", "terminal-compat-check.mjs", "PORTMATE_UI_LONG_SESSION_ONLY"],
  ["full RX/TX cache in text, Hex and split", "terminal-compat-check.mjs", "PORTMATE_UI_BYTE_LOAD_ONLY", { PORTMATE_UI_PRODUCTION: "1", PORTMATE_UI_FORCE_WEBGL: "0" }],
  ["sustained high-rate text RX/TX after screen clear", "terminal-compat-check.mjs", "PORTMATE_UI_BYTE_LOAD_ONLY", { PORTMATE_UI_TEXT_RXTX_ONLY: "1", PORTMATE_UI_PRODUCTION: "1", PORTMATE_UI_FORCE_WEBGL: "0" }],
  ["sustained RX/TX P99, input integrity and burst recovery", "terminal-compat-check.mjs", "PORTMATE_UI_TAIL_LATENCY_ONLY", { PORTMATE_UI_PRODUCTION: "1", PORTMATE_UI_FORCE_WEBGL: "0" }],
  ["production dual-pane TUI and keyboard", "terminal-compat-check.mjs", "PORTMATE_UI_INTERACTIVE_FLUIDITY_ONLY", { PORTMATE_UI_PRODUCTION: "1", PORTMATE_UI_FORCE_WEBGL: "0" }],
  ["WebGL context lifecycle (simulated driver)", "terminal-compat-check.mjs", "PORTMATE_UI_RENDERER_LIFECYCLE_ONLY", { PORTMATE_UI_FORCE_WEBGL: "1", PORTMATE_UI_PRODUCTION: "0" }],
  ["background startup and SSH order", "workspace-ui-check.mjs", "PORTMATE_UI_STARTUP_PERF_ONLY"],
];
for (const [label, script, flag, environment] of checks) {
  console.log(`Checking ${label}`);
  const result = spawnSync(process.execPath, [`test/tooling/${script}`], {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, ...environment, [flag]: "1" },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
