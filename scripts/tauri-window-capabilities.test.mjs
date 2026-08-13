import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function readProjectFile(path) {
  return readFileSync(join(projectRoot, path), "utf8");
}

function readCapability(name) {
  return JSON.parse(readProjectFile(`src-tauri/capabilities/${name}.json`));
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

  it("keeps the generated analyzer label inside the capability scope", () => {
    const app = readProjectFile("src/App.tsx");
    const analyzer = readProjectFile("src/SerialAnalyzerApp.tsx");
    expect(app).toContain('.replace(/^pane-/, "serial-analyzer-")');
    expect(analyzer).toContain("getCurrentWebviewWindow().close()");
  });
});
