import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createMavenRunner } from "./mcp-jvm-tools.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifestRoot = join(projectRoot, "scripts", "mcp-java-client-check");
const matrix = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-java-client-versions.json"), "utf8"));
const tools = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-jvm-tool-versions.json"), "utf8"));
if (!Array.isArray(matrix) || !matrix.length || matrix.some((entry) => (
  typeof entry !== "object"
  || !/^\d+\.\d+\.\d+$/.test(entry.version)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
))) {
  throw new Error("scripts/mcp-java-client-versions.json must contain exact SDK and protocol versions");
}

const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(
  projectRoot,
  "target",
  "debug",
  process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
);
if (!existsSync(binary)) throw new Error(`MCP Java client check binary does not exist: ${binary}`);

const runMaven = await createMavenRunner({
  projectRoot,
  manifestRoot,
  distribution: tools.maven,
});
for (const entry of matrix) {
  runMaven([
    "--batch-mode",
    "--no-transfer-progress",
    `-Dmaven.repo.local=${join(projectRoot, "target", "mcp-java-maven-repository")}`,
    `-Dmcp.sdk.version=${entry.version}`,
    `-Dportmate.mcp.protocol.version=${entry.protocolVersion}`,
    `-Dportmate.mcp.binary=${resolve(binary)}`,
    "clean",
    "compile",
    "exec:java",
  ], { timeout: 180_000 });
}
