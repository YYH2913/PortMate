import { readFileSync, readdirSync } from "node:fs";
import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = new URL(`${entry.name}${entry.isDirectory() ? "/" : ""}`, directory);
    if (entry.isDirectory()) return entry.name.includes("tests") ? [] : sourceFiles(path);
    return entry.name.endsWith(".rs") ? [path] : [];
  });
}

describe("application-owned native message coverage", () => {
  it("requires translations when native diagnostic copy changes", () => {
    const templates = JSON.parse(readFileSync(new URL("../src/locales/native-message-templates.json", import.meta.url), "utf8"));
    const known = new Map(templates.map(item => [item.id, item.source]));
    const missing = [];
    for (const file of sourceFiles(new URL("../src-tauri/src/", import.meta.url))) {
      const code = readFileSync(file, "utf8").split("#[cfg(test)]")[0];
      for (const match of code.matchAll(/"(?:[^"\\]|\\.)*"/g)) {
        let source;
        try { source = JSON.parse(match[0]); } catch { continue; }
        // Raw protocol fixtures and short recognition tokens are not interface copy.
        if ((source.match(/\p{Script=Han}/gu) ?? []).length < 4 || source.length > 1500) continue;
        const id = "native-message-" + createHash("sha256").update(source).digest("hex").slice(0, 12);
        let index = 0;
        const normalized = source.replace(/\{\{|\}\}|\{[^{}]*\}/g, token => token === "{{" ? "{" : token === "}}" ? "}" : `{${index++}}`);
        if (known.get(id) !== normalized) missing.push({ file: file.pathname, source });
      }
    }
    expect(missing).toEqual([]);
  });
});
