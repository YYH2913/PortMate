import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createMavenRunner } from "./mcp-jvm-tools.mjs";
import { createMcpClientFixture } from "./mcp-client-fixture.mjs";
import { resolveCargoTargetDirectory, resolveMcpBinary } from "./mcp-binary-path.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const fixture = createMcpClientFixture(["official-kotlin-sdk-stdio-check", "official-kotlin-sdk-http-check"]);
const manifestRoot = join(projectRoot, "test", "tooling", "mcp-kotlin-client-check");
const targetRoot = resolveCargoTargetDirectory(projectRoot);
const matrix = JSON.parse(readFileSync(join(projectRoot, "test", "tooling", "mcp-kotlin-client-versions.json"), "utf8"));
const tools = JSON.parse(readFileSync(join(projectRoot, "test", "tooling", "mcp-jvm-tool-versions.json"), "utf8"));
const versionPattern = /^\d+\.\d+\.\d+$/;
if (!Array.isArray(matrix) || !matrix.length || matrix.some((entry) => (
  typeof entry !== "object"
  || !versionPattern.test(entry.version)
  || !versionPattern.test(entry.kotlinVersion)
  || !versionPattern.test(entry.ktorVersion)
  || !versionPattern.test(entry.coroutinesVersion)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
))) {
  throw new Error("test/tooling/mcp-kotlin-client-versions.json must contain exact SDK, compiler, Ktor, and protocol versions");
}

const binary = resolveMcpBinary(projectRoot);
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
    `-Dmaven.repo.local=${join(targetRoot, "mcp-kotlin-maven-repository")}`,
    `-Dmcp.sdk.version=${entry.version}`,
    `-Dkotlin.version=${entry.kotlinVersion}`,
    `-Dktor.version=${entry.ktorVersion}`,
    `-Dkotlinx.coroutines.version=${entry.coroutinesVersion}`,
    `-Dportmate.mcp.protocol.version=${entry.protocolVersion}`,
    `-Dportmate.mcp.binary=${resolve(binary)}`,
    "clean",
    "compile",
    "exec:java",
  ], { timeout: 240_000, env: { ...process.env, ...fixture.environment } });
}
