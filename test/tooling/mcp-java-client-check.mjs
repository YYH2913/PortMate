import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createMavenRunner } from "./mcp-jvm-tools.mjs";
import { createMcpClientFixture } from "./mcp-client-fixture.mjs";
import { resolveCargoTargetDirectory, resolveMcpBinary } from "./mcp-binary-path.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const fixture = createMcpClientFixture(["official-java-sdk-stdio-check", "official-java-sdk-http-check"]);
const manifestRoot = join(projectRoot, "test", "tooling", "mcp-java-client-check");
const targetRoot = resolveCargoTargetDirectory(projectRoot);
const matrix = JSON.parse(readFileSync(join(projectRoot, "test", "tooling", "mcp-java-client-versions.json"), "utf8"));
const tools = JSON.parse(readFileSync(join(projectRoot, "test", "tooling", "mcp-jvm-tool-versions.json"), "utf8"));
if (!Array.isArray(matrix) || !matrix.length || matrix.some((entry) => (
  typeof entry !== "object"
  || !/^\d+\.\d+\.\d+$/.test(entry.version)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
))) {
  throw new Error("test/tooling/mcp-java-client-versions.json must contain exact SDK and protocol versions");
}

const binary = resolveMcpBinary(projectRoot);
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
    `-Dmaven.repo.local=${join(targetRoot, "mcp-java-maven-repository")}`,
    `-Dmcp.sdk.version=${entry.version}`,
    `-Dportmate.mcp.protocol.version=${entry.protocolVersion}`,
    `-Dportmate.mcp.binary=${resolve(binary)}`,
    "clean",
    "compile",
    "exec:java",
  ], { timeout: 180_000, env: { ...process.env, ...fixture.environment } });
}
