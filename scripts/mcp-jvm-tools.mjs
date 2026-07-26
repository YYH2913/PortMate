import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

export async function createMavenRunner({ projectRoot, manifestRoot, distribution }) {
  if (
    typeof distribution !== "object"
    || !/^\d+\.\d+\.\d+$/.test(distribution.version)
    || !/^[a-f0-9]{128}$/.test(distribution.sha512)
  ) {
    throw new Error("scripts/mcp-jvm-tool-versions.json must pin an exact Maven version and SHA-512");
  }

  const java = process.env.PORTMATE_JAVA?.trim() || "java";
  const javaProbe = run(java, ["-version"], { capture: true, allowFailure: true, timeout: 10_000 });
  const javaVersion = `${javaProbe.stdout}\n${javaProbe.stderr}`.match(/version "(\d+)(?:\.|-)/)?.[1];
  if (javaProbe.status !== 0 || !javaVersion || Number(javaVersion) < 17) {
    throw new Error("MCP JVM SDK checks require JDK 17 or newer (set PORTMATE_JAVA to override discovery)");
  }
  const compilerProbe = run(java, ["-m", "jdk.compiler/com.sun.tools.javac.Main", "-version"], {
    capture: true,
    allowFailure: true,
    timeout: 10_000,
  });
  if (compilerProbe.status !== 0) {
    throw new Error("MCP JVM SDK checks require the jdk.compiler module from a full JDK");
  }

  const mavenRoot = await ensureMaven(projectRoot, distribution);
  const runMaven = (args, options = {}) => {
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
    ], { cwd: options.cwd ?? manifestRoot, ...options });
  };

  const mavenVersion = runMaven(["-version"], { capture: true, timeout: 15_000 });
  if (!mavenVersion.stdout.startsWith(`Apache Maven ${distribution.version}`)) {
    throw new Error(
      `Expected Maven ${distribution.version}, found ${mavenVersion.stdout.split(/\r?\n/, 1)[0]}`,
    );
  }
  return runMaven;
}

async function ensureMaven(projectRoot, { version, sha512 }) {
  const toolsRoot = join(projectRoot, "target", "mcp-jvm-sdk-tools");
  const mavenRoot = join(toolsRoot, `apache-maven-${version}`);
  if (hasMavenLauncher(mavenRoot)) return mavenRoot;

  mkdirSync(toolsRoot, { recursive: true });
  const archiveName = `apache-maven-${version}-bin.tar.gz`;
  const archive = join(toolsRoot, `${archiveName}.download`);
  const response = await fetch(
    `https://repo.maven.apache.org/maven2/org/apache/maven/apache-maven/${version}/${archiveName}`,
    { signal: AbortSignal.timeout(120_000) },
  );
  if (!response.ok) throw new Error(`Failed to download Maven ${version}: HTTP ${response.status}`);

  const body = Buffer.from(await response.arrayBuffer());
  const actual = createHash("sha512").update(body).digest("hex");
  if (actual !== sha512) {
    throw new Error(`Maven ${version} SHA-512 mismatch: expected ${sha512}, found ${actual}`);
  }
  writeFileSync(archive, body, { mode: 0o600 });
  rmSync(mavenRoot, { recursive: true, force: true });
  run("tar", ["-xzf", archive, "-C", toolsRoot], { cwd: projectRoot, timeout: 30_000 });
  renameSync(archive, join(toolsRoot, archiveName));
  if (!hasMavenLauncher(mavenRoot)) {
    throw new Error(`Maven archive omitted its Classworlds launcher: ${mavenRoot}`);
  }
  return mavenRoot;
}

function hasMavenLauncher(mavenRoot) {
  const boot = join(mavenRoot, "boot");
  return existsSync(join(mavenRoot, "bin", "m2.conf"))
    && existsSync(boot)
    && readdirSync(boot).some((name) => /^plexus-classworlds-[\d.]+\.jar$/.test(name));
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
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
