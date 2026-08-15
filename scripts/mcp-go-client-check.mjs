import { cpSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const moduleRoot = join(projectRoot, "scripts", "mcp-go-client-check");
const matrix = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-go-client-versions.json"), "utf8"));
const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(projectRoot, "target", "debug", process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp");

if (!Array.isArray(matrix) || !matrix.length || matrix.some((entry) => (
  typeof entry !== "object"
  || !/^\d+\.\d+\.\d+$/.test(entry.version)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
))) {
  throw new Error("scripts/mcp-go-client-versions.json must contain exact SDK and protocol versions");
}

if (!existsSync(binary)) {
  throw new Error(`MCP Go client check binary does not exist: ${binary}`);
}

for (const { version: sdkVersion, protocolVersion } of matrix) {
  const environmentRoot = join(projectRoot, "target", `mcp-go-sdk-${sdkVersion}`);
  mkdirSync(environmentRoot, { recursive: true });
  cpSync(join(moduleRoot, "main.go"), join(environmentRoot, "main.go"));
  const lockRoot = join(moduleRoot, "locks", sdkVersion);
  const goModSource = join(lockRoot, "go.mod");
  const goSumSource = join(lockRoot, "go.sum");
  if (!existsSync(goModSource) || !existsSync(goSumSource)) {
    throw new Error(`MCP Go SDK ${sdkVersion} lock files do not exist: ${lockRoot}`);
  }
  const goMod = readFileSync(goModSource, "utf8");
  if (!goMod.includes(`require github.com/modelcontextprotocol/go-sdk v${sdkVersion}`)) {
    throw new Error(`MCP Go SDK ${sdkVersion} go.mod pins a different SDK version`);
  }
  cpSync(goModSource, join(environmentRoot, "go.mod"));
  cpSync(goSumSource, join(environmentRoot, "go.sum"));

  run("go", ["run", "-mod=readonly", ".", "-binary", binary], environmentRoot, {
    ...process.env,
    PORTMATE_MCP_GO_SDK_VERSION: sdkVersion,
    PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION: protocolVersion,
  });
}

function run(command, args, cwd, env = process.env) {
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
    timeout: 300_000,
  });
  if (result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}
