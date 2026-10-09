import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

function filesBelow(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? filesBelow(path) : [path];
  });
}

const productionRoots = [
  "src", "src-tauri/src", "scripts",
  ...readdirSync(join(projectRoot, "crates")).map(name => `crates/${name}/src`),
].filter(path => existsSync(join(projectRoot, path)));

describe("centralized test layout", () => {
  it("keeps test files and Vitest imports out of production source trees", () => {
    const misplaced = [];
    for (const root of productionRoots) {
      for (const path of filesBelow(join(projectRoot, root))) {
        if (/(?:^|[/\\])tests?(?:[/\\]|\.rs$)|\.(?:test|spec)\.(?:tsx?|m?js)$|_tests?\.rs$/.test(path)) misplaced.push(path);
        if (/\.(?:tsx?|m?js)$/.test(path) && /from\s+["']vitest(?:\/[^"']*)?["']/.test(readFileSync(path, "utf8"))) misplaced.push(path);
      }
    }
    expect(misplaced).toEqual([]);
  });

  it("keeps Rust test bodies external while allowing cfg(test) wiring", () => {
    const inline = [];
    for (const root of productionRoots) {
      for (const path of filesBelow(join(projectRoot, root)).filter(path => path.endsWith(".rs"))) {
        const source = readFileSync(path, "utf8");
        if (/^\s*#\[\s*(?:test|tokio::test|async_std::test)\b/m.test(source)
          || /#\[cfg\((?:all\()?test\b[^\n]*\)\]\s*(?:#\[[^\n]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{/.test(source)) inline.push(path);
      }
    }
    expect(inline).toEqual([]);
  });

  it("routes npm test commands to test rather than the old layout", () => {
    const scripts = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8")).scripts;
    expect(scripts.test).toContain("tsc -p test/tsconfig.json");
    expect(scripts.test).toContain("vitest run --config test/vitest.config.ts");
    for (const [name, command] of Object.entries(scripts)) {
      if (name.startsWith("test:")) expect(command, name).not.toMatch(/(?:node|bash) scripts\//);
    }
    expect(existsSync(join(projectRoot, "tests"))).toBe(false);
    expect(existsSync(join(projectRoot, "src-tauri", "tests"))).toBe(false);
    for (const name of readdirSync(join(projectRoot, "crates"))) {
      expect(existsSync(join(projectRoot, "crates", name, "tests")), name).toBe(false);
    }
  });

  it("resolves test runner and compatibility fixture paths from the repository root", () => {
    const scripts = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8")).scripts;
    for (const [name, command] of Object.entries(scripts)) {
      if (!name.startsWith("test:")) continue;
      for (const match of command.matchAll(/(?:node|bash)\s+(test\/[^\s]+)/g)) {
        expect(existsSync(join(projectRoot, match[1])), `${name}: ${match[1]}`).toBe(true);
      }
    }
    for (const path of filesBelow(join(projectRoot, "test", "compat"))) {
      if (!path.endsWith("-matrix.json")) continue;
      const entries = JSON.parse(readFileSync(path, "utf8"));
      for (const entry of entries) {
        if (!entry.dockerfile) continue;
        expect(entry.dockerfile).toMatch(/^test\/compat\//);
        expect(existsSync(join(projectRoot, entry.dockerfile)), entry.dockerfile).toBe(true);
      }
    }
  });
});
