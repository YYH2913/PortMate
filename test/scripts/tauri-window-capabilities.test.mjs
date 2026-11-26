import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

function readProjectFile(path) {
  return readFileSync(join(projectRoot, path), "utf8");
}

function readCapability(name) {
  return JSON.parse(readProjectFile(`src-tauri/capabilities/${name}.json`));
}

function stripCommentsPreservingStrings(source) {
  let out = "";
  let i = 0;
  let quote = null;
  while (i < source.length) {
    const ch = source[i];
    const next = source[i + 1];
    if (quote) {
      out += ch;
      if (ch === "\\" && i + 1 < source.length) {
        out += next;
        i += 2;
        continue;
      }
      if (ch === quote) quote = null;
      i += 1;
      continue;
    }
    if (ch === "'" || ch === "\"" || ch === "`") {
      quote = ch;
      out += ch;
      i += 1;
      continue;
    }
    if (ch === "/" && next === "/") {
      out += " ";
      i += 2;
      while (i < source.length && source[i] !== "\n") i += 1;
      continue;
    }
    if (ch === "/" && next === "*") {
      out += " ";
      i += 2;
      while (i < source.length && !(source[i] === "*" && source[i + 1] === "/")) i += 1;
      i += 2;
      continue;
    }
    out += ch;
    i += 1;
  }
  return out;
}

describe("Tauri child window capabilities", () => {
  it("grants serial analyzer windows backend IPC and window close only", () => {
    const capability = readCapability("serial-analyzer");
    expect(capability).toMatchObject({
      identifier: "serial-analyzer",
      windows: ["serial-analyzer-*"],
      permissions: ["core:default", "core:window:allow-close"],
    });
    expect(capability.permissions).toHaveLength(2);
  });

  it("grants owner workspace windows destroy permission and keeps generated schema in sync", () => {
    const generated = JSON.parse(readProjectFile("src-tauri/gen/schemas/capabilities.json"));
    const expectedOwners = [
      ["default", ["main"]],
      ["workspace-window", ["workspace-*"]],
    ];
    for (const [name, windows] of expectedOwners) {
      const capability = readCapability(name);
      const { $schema: _schema, ...expected } = capability;
      expect(capability.windows).toEqual(windows);
      expect(capability.permissions).toContain("core:window:allow-destroy");
      expect(generated[name]).toEqual({ local: true, ...expected });
    }
    for (const childName of ["serial-analyzer", "detached-pane"]) {
      const capability = readCapability(childName);
      const { $schema: _schema, ...expected } = capability;
      expect(generated[childName]).toEqual({ local: true, ...expected });
    }
  });

  it("keeps the generated analyzer label inside the capability scope and never intercepts close requests", () => {
    // Sanity check: string literals containing "://" survive comment stripping, comments do not,
    // and block comments separate adjacent tokens rather than merging them.
    const probe = stripCommentsPreservingStrings(
      'child.listen("tauri://close-requested", fn); WINDOW_CLOSE_REQUESTED/**/as TauriEvent; // onCloseRequested',
    );
    expect(probe).toContain("tauri://close-requested");
    expect(probe).toMatch(/\bWINDOW_CLOSE_REQUESTED\b/);
    expect(probe).not.toContain("onCloseRequested");

    const app = readProjectFile("src/App.tsx");
    const analyzer = readProjectFile("src/SerialAnalyzerApp.tsx");
    const geometryCode = stripCommentsPreservingStrings(readProjectFile("src/window-geometry.ts"));
    expect(app).toContain('.replace(/^pane-/, "serial-analyzer-")');
    expect(analyzer).toContain("getCurrentWebviewWindow().close()");
    expect(geometryCode).not.toMatch(/\bonCloseRequested\b/);
    expect(geometryCode).not.toMatch(/\bWINDOW_CLOSE_REQUESTED\b/);
    expect(geometryCode).not.toContain("tauri://close-requested");
  });
});
