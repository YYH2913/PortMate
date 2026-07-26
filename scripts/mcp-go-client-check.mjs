import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
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
  const goMod = [
    `module github.com/portmate/portmate-mcp-client-check-${sdkVersion.replaceAll(".", "-")}`,
    "",
    "go 1.25.0",
    "",
    `require github.com/modelcontextprotocol/go-sdk v${sdkVersion}`,
    "",
  ].join("\n");
  const goModPath = join(environmentRoot, "go.mod");
  if (!existsSync(goModPath) || readFileSync(goModPath, "utf8") !== goMod) {
    writeFileSync(goModPath, goMod, "utf8");
  }

  run("go", ["mod", "tidy"], environmentRoot);
  run("go", ["run", ".", "-binary", binary], environmentRoot, {
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
  });
  if (result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}
