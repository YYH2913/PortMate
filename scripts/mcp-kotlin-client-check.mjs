import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createMavenRunner } from "./mcp-jvm-tools.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifestRoot = join(projectRoot, "scripts", "mcp-kotlin-client-check");
const matrix = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-kotlin-client-versions.json"), "utf8"));
const tools = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-jvm-tool-versions.json"), "utf8"));
const versionPattern = /^\d+\.\d+\.\d+$/;
if (!Array.isArray(matrix) || !matrix.length || matrix.some((entry) => (
  typeof entry !== "object"
  || !versionPattern.test(entry.version)
  || !versionPattern.test(entry.kotlinVersion)
  || !versionPattern.test(entry.ktorVersion)
  || !versionPattern.test(entry.coroutinesVersion)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
))) {
  throw new Error("scripts/mcp-kotlin-client-versions.json must contain exact SDK, compiler, Ktor, and protocol versions");
}

const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(
  projectRoot,
  "target",
  "debug",
  process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
);
if (!existsSync(binary)) throw new Error(`MCP Kotlin client check binary does not exist: ${binary}`);

const runMaven = await createMavenRunner({
  projectRoot,
  manifestRoot,
  distribution: tools.maven,
});
for (const entry of matrix) {
  runMaven([
    "--batch-mode",
    "--no-transfer-progress",
    `-Dmaven.repo.local=${join(projectRoot, "target", "mcp-kotlin-maven-repository")}`,
    `-Dmcp.sdk.version=${entry.version}`,
    `-Dkotlin.version=${entry.kotlinVersion}`,
    `-Dktor.version=${entry.ktorVersion}`,
    `-Dkotlinx.coroutines.version=${entry.coroutinesVersion}`,
    `-Dportmate.mcp.protocol.version=${entry.protocolVersion}`,
    `-Dportmate.mcp.binary=${resolve(binary)}`,
    "compile",
    "exec:java",
  ], { timeout: 240_000 });
}
