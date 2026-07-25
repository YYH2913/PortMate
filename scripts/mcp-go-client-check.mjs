import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const moduleRoot = join(projectRoot, "scripts", "mcp-go-client-check");
const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(projectRoot, "target", "debug", process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp");

if (!existsSync(binary)) {
  throw new Error(`MCP Go client check binary does not exist: ${binary}`);
}

const result = spawnSync("go", ["run", ".", "-binary", binary], {
  cwd: moduleRoot,
  env: process.env,
  encoding: "utf8",
  maxBuffer: 16 * 1024 * 1024,
});
if (result.stdout) process.stdout.write(result.stdout);
if (result.stderr) process.stderr.write(result.stderr);
if (result.error) throw result.error;
if (result.status !== 0) {
  throw new Error(`go run MCP client check failed with exit code ${result.status}`);
}
