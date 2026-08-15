import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ruby = process.env.PORTMATE_RUBY?.trim() || "ruby";
const matrix = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-ruby-client-versions.json"), "utf8"));
const versionPattern = /^\d+\.\d+\.\d+$/;
if (!Array.isArray(matrix) || !matrix.length || matrix.some((entry) => (
  typeof entry !== "object"
  || !versionPattern.test(entry.version)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
  || !versionPattern.test(entry.faradayVersion)
  || !versionPattern.test(entry.eventStreamParserVersion)
  || !versionPattern.test(entry.jsonSchemerVersion)
  || !versionPattern.test(entry.faradayNetHttpVersion)
  || !versionPattern.test(entry.hanaVersion)
  || !versionPattern.test(entry.regexpParserVersion)
  || !versionPattern.test(entry.simpleidnVersion)
  || !versionPattern.test(entry.netHttpVersion)
))) {
  throw new Error("scripts/mcp-ruby-client-versions.json must contain exact SDK, dependency, and protocol versions");
}

const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(
  projectRoot,
  "target",
  "debug",
  process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
);
if (!existsSync(binary)) {
  throw new Error(`MCP Ruby client check binary does not exist: ${binary}`);
}

run(ruby, ["-e", "require 'rubygems'; raise 'Ruby 3.2 or newer is required' if Gem::Version.new(RUBY_VERSION) < Gem::Version.new('3.2.0')"], {
  timeout: 10_000,
});

for (const entry of matrix) {
  const environmentRoot = join(projectRoot, "target", `mcp-ruby-sdk-${entry.version}`);
  const required = [
    ["hana", entry.hanaVersion],
    ["regexp_parser", entry.regexpParserVersion],
    ["simpleidn", entry.simpleidnVersion],
    ["net-http", entry.netHttpVersion],
    ["faraday-net_http", entry.faradayNetHttpVersion],
    ["json_schemer", entry.jsonSchemerVersion],
    ["faraday", entry.faradayVersion],
    ["event_stream_parser", entry.eventStreamParserVersion],
    ["mcp", entry.version],
  ];
  const lockMarker = `${JSON.stringify(required)}\n`;
  const lockMarkerPath = join(environmentRoot, ".portmate-dependency-lock.json");
  if (!existsSync(lockMarkerPath) || readFileSync(lockMarkerPath, "utf8") !== lockMarker) {
    rmSync(environmentRoot, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
  }
  mkdirSync(environmentRoot, { recursive: true });
  const environment = {
    ...process.env,
    GEM_HOME: environmentRoot,
    GEM_PATH: environmentRoot,
    GEM_SPEC_CACHE: join(environmentRoot, "spec-cache"),
  };
  for (const [name, version] of required) {
    const installed = run(ruby, [
      "-e",
      `require 'rubygems'; print Gem::Specification.find_by_name('${name}', '= ${version}').version`,
    ], { env: environment, capture: true, allowFailure: true, timeout: 10_000 });
    if (installed.status !== 0 || installed.stdout.trim() !== version) {
      run(ruby, [
        "-S",
        "gem",
        "install",
        "--no-document",
        "--clear-sources",
        "--source",
        "https://rubygems.org",
        "--version",
        version,
        name,
      ], { env: environment, timeout: 120_000 });
    }
  }
  writeFileSync(lockMarkerPath, lockMarker, { encoding: "utf8", mode: 0o600 });

  run(ruby, [join(projectRoot, "scripts", "mcp-ruby-client-check.rb")], {
    env: {
      ...environment,
      PORTMATE_MCP_RUBY_SDK_VERSION: entry.version,
      PORTMATE_MCP_RUBY_FARADAY_VERSION: entry.faradayVersion,
      PORTMATE_MCP_RUBY_EVENT_STREAM_VERSION: entry.eventStreamParserVersion,
      PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION: entry.protocolVersion,
      PORTMATE_MCP_BINARY: binary,
    },
    timeout: 90_000,
  });
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: options.capture ? ["ignore", "pipe", "pipe"] : "inherit",
    maxBuffer: 16 * 1024 * 1024,
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
