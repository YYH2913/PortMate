import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifestRoot = join(projectRoot, "scripts", "mcp-java-client-check");
const matrix = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-java-client-versions.json"), "utf8"));
const versionPattern = /^\d+\.\d+\.\d+$/;
if (
  typeof matrix !== "object"
  || typeof matrix.maven !== "object"
  || !versionPattern.test(matrix.maven.version)
  || !/^[a-f0-9]{128}$/.test(matrix.maven.sha512)
  || !Array.isArray(matrix.sdks)
  || !matrix.sdks.length
  || matrix.sdks.some((entry) => (
    typeof entry !== "object"
    || !versionPattern.test(entry.version)
    || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
  ))
) {
  throw new Error("scripts/mcp-java-client-versions.json must contain exact Maven, SDK, and protocol versions");
}

const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(
  projectRoot,
  "target",
  "debug",
  process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
);
if (!existsSync(binary)) {
  throw new Error(`MCP Java client check binary does not exist: ${binary}`);
}

const java = process.env.PORTMATE_JAVA?.trim() || "java";
const javaProbe = run(java, ["-version"], { capture: true, allowFailure: true, timeout: 10_000 });
const javaVersion = `${javaProbe.stdout}\n${javaProbe.stderr}`.match(/version "(\d+)(?:\.|-)/)?.[1];
if (javaProbe.status !== 0 || !javaVersion || Number(javaVersion) < 17) {
  throw new Error("MCP Java SDK checks require JDK 17 or newer (set PORTMATE_JAVA to override discovery)");
}
const compilerProbe = run(java, ["-m", "jdk.compiler/com.sun.tools.javac.Main", "-version"], {
  capture: true,
  allowFailure: true,
  timeout: 10_000,
});
if (compilerProbe.status !== 0) {
  throw new Error("MCP Java SDK checks require the jdk.compiler module from a full JDK");
}

const mavenRoot = await ensureMaven(matrix.maven);
const mavenVersion = runMaven(["-version"], { capture: true, timeout: 15_000 });
if (!mavenVersion.stdout.startsWith(`Apache Maven ${matrix.maven.version}`)) {
  throw new Error(`Expected Maven ${matrix.maven.version}, found ${mavenVersion.stdout.split(/\r?\n/, 1)[0]}`);
}

for (const entry of matrix.sdks) {
  runMaven([
    "--batch-mode",
    "--no-transfer-progress",
    `-Dmaven.repo.local=${join(projectRoot, "target", "mcp-java-maven-repository")}`,
    `-Dmcp.sdk.version=${entry.version}`,
    `-Dportmate.mcp.protocol.version=${entry.protocolVersion}`,
    `-Dportmate.mcp.binary=${resolve(binary)}`,
    "compile",
    "exec:java",
  ], { cwd: manifestRoot, timeout: 180_000 });
}

async function ensureMaven({ version, sha512 }) {
  const toolsRoot = join(projectRoot, "target", "mcp-java-sdk-tools");
  const mavenRoot = join(toolsRoot, `apache-maven-${version}`);
  if (hasMavenLauncher(mavenRoot)) return mavenRoot;

  mkdirSync(toolsRoot, { recursive: true });
  const archiveName = `apache-maven-${version}-bin.tar.gz`;
  const archive = join(toolsRoot, `${archiveName}.download`);
  const response = await fetch(
    `https://repo.maven.apache.org/maven2/org/apache/maven/apache-maven/${version}/${archiveName}`,
    { signal: AbortSignal.timeout(120_000) },
  );
  if (!response.ok) {
    throw new Error(`Failed to download Maven ${version}: HTTP ${response.status}`);
  }
  const body = Buffer.from(await response.arrayBuffer());
  const actual = createHash("sha512").update(body).digest("hex");
  if (actual !== sha512) {
    throw new Error(`Maven ${version} SHA-512 mismatch: expected ${sha512}, found ${actual}`);
  }
  writeFileSync(archive, body, { mode: 0o600 });
  rmSync(mavenRoot, { recursive: true, force: true });
  run("tar", ["-xzf", archive, "-C", toolsRoot], { timeout: 30_000 });
  renameSync(archive, join(toolsRoot, archiveName));
  if (!hasMavenLauncher(mavenRoot)) throw new Error(`Maven archive omitted its Classworlds launcher: ${mavenRoot}`);
  return mavenRoot;
}

function hasMavenLauncher(mavenRoot) {
  const boot = join(mavenRoot, "boot");
  return existsSync(join(mavenRoot, "bin", "m2.conf"))
    && existsSync(boot)
    && readdirSync(boot).some((name) => /^plexus-classworlds-[\d.]+\.jar$/.test(name));
}

function runMaven(args, options = {}) {
  const boot = join(mavenRoot, "boot");
  const launchers = readdirSync(boot).filter((name) => /^plexus-classworlds-[\d.]+\.jar$/.test(name));
  if (launchers.length !== 1) {
    throw new Error(`Expected one Maven Classworlds launcher in ${boot}, found ${launchers.length}`);
  }
  return run(java, [
    `-Dmaven.home=${mavenRoot}`,
    `-Dmaven.multiModuleProjectDirectory=${manifestRoot}`,
    `-Dclassworlds.conf=${join(mavenRoot, "bin", "m2.conf")}`,
    "-classpath",
    join(boot, launchers[0]),
    "org.codehaus.plexus.classworlds.launcher.Launcher",
    ...args,
  ], options);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: options.capture ? ["ignore", "pipe", "pipe"] : "inherit",
    maxBuffer: 32 * 1024 * 1024,
    timeout: options.timeout ?? 30_000,
  });
  if (result.error && !options.allowFailure) {
    if (result.error.code === "ETIMEDOUT") {
      throw new Error(`${command} exceeded its ${options.timeout ?? 30_000} ms timeout`);
    }
    throw result.error;
  }
  if (result.status !== 0 && !options.allowFailure) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} failed with exit code ${result.status ?? 1}${details ? `\n${details}` : ""}`);
  }
  return result;
}
