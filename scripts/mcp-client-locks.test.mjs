import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { cargoLockPinsPackage } from "./cargo-lock-state.mjs";

const projectRoot = resolve(import.meta.dirname, "..");

describe("MCP client dependency locks", () => {
  it("tracks and validates every TypeScript SDK lock", () => {
    for (const { version } of matrix("mcp-typescript-client-versions.json")) {
      const lock = jsonLock("mcp-typescript-client-check", version, "package-lock.json");
      expect(lock.packages?.[""]?.dependencies?.["@modelcontextprotocol/sdk"]).toBe(version);
    }
    expect(script("mcp-typescript-client-check.mjs")).not.toContain("--package-lock-only");
  });

  it("tracks and validates every Python SDK requirements snapshot", () => {
    for (const { version } of matrix("mcp-python-client-versions.json")) {
      const requirements = textLock("mcp-python-client-check", version, "requirements.txt");
      expect(requirements.split(/\r?\n/)).toContain(`mcp==${version}`);
    }
    expect(script("mcp-python-client-check.mjs")).toContain('"freeze", "--exclude", "pip"');
  });

  it("tracks and validates every Go SDK module graph", () => {
    for (const { version } of matrix("mcp-go-client-versions.json")) {
      const goMod = textLock("mcp-go-client-check", version, "go.mod");
      expect(goMod).toContain(`require github.com/modelcontextprotocol/go-sdk v${version}`);
      expect(textLock("mcp-go-client-check", version, "go.sum").trim()).not.toBe("");
    }
    const source = script("mcp-go-client-check.mjs");
    expect(source).toContain('"-mod=readonly"');
    expect(source).not.toContain('"mod", "tidy"');
  });

  it("tracks and validates every Rust SDK Cargo lock", () => {
    for (const { version } of matrix("mcp-rust-client-versions.json")) {
      const lock = textLock("mcp-rust-client-check", version, "Cargo.lock");
      expect(cargoLockPinsPackage(lock, "rmcp", version)).toBe(true);
      expect(cargoLockPinsPackage(lock.replaceAll("\n", "\r\n"), "rmcp", version)).toBe(true);
      expect(cargoLockPinsPackage(lock, "rmcp", `${version}-mismatch`)).toBe(false);
    }
    expect(script("mcp-rust-client-check.mjs")).not.toContain("generate-lockfile");
  });

  it("tracks and validates every C# SDK NuGet lock", () => {
    const value = JSON.parse(readFileSync(
      join(projectRoot, "scripts", "mcp-csharp-client-versions.json"),
      "utf8",
    ));
    for (const { version } of value.sdks) {
      const lock = jsonLock("mcp-csharp-client-check", version, "packages.lock.json");
      expect(lock.dependencies?.["net10.0"]?.["ModelContextProtocol.Core"]?.resolved)
        .toBe(version);
    }
    const source = script("mcp-csharp-client-check.mjs");
    expect(source).toContain('"--locked-mode"');
    expect(source).not.toContain("--force-evaluate");
  });

  it("pins every external Ruby dependency used by the SDK matrix", () => {
    const requiredVersions = [
      "faradayVersion",
      "eventStreamParserVersion",
      "jsonSchemerVersion",
      "faradayNetHttpVersion",
      "hanaVersion",
      "regexpParserVersion",
      "simpleidnVersion",
      "netHttpVersion",
    ];
    for (const entry of matrix("mcp-ruby-client-versions.json")) {
      for (const field of requiredVersions) expect(entry[field]).toMatch(/^\d+\.\d+\.\d+$/);
    }
  });

  it("runs each external SDK matrix once in Native CI", () => {
    const workflow = readFileSync(join(projectRoot, ".github", "workflows", "native-ci.yml"), "utf8");
    for (const sdk of [
      "typescript", "python", "go", "rust", "ruby", "java", "kotlin", "csharp", "swift",
    ]) {
      expect(workflow.match(new RegExp(`npm run test:mcp-${sdk}-client`, "g"))).toHaveLength(1);
    }
  });
});

function matrix(name) {
  const value = JSON.parse(readFileSync(join(projectRoot, "scripts", name), "utf8"));
  return Array.isArray(value) ? value : value.sdks;
}

function textLock(client, version, name) {
  const path = join(projectRoot, "scripts", client, "locks", version, name);
  expect(existsSync(path), path).toBe(true);
  return readFileSync(path, "utf8");
}

function jsonLock(client, version, name) {
  return JSON.parse(textLock(client, version, name));
}

function script(name) {
  return readFileSync(join(projectRoot, "scripts", name), "utf8");
}
