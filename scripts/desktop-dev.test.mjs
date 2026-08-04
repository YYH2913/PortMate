import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(import.meta.dirname, "..");

describe("desktop development startup", () => {
  it("prepares the sidecar before Tauri starts the frontend", () => {
    const packageJson = JSON.parse(readFileSync(resolve(projectRoot, "package.json"), "utf8"));
    const tauriConfig = JSON.parse(readFileSync(resolve(projectRoot, "src-tauri/tauri.conf.json"), "utf8"));
    const launcher = readFileSync(resolve(projectRoot, "scripts/desktop-dev.mjs"), "utf8");

    expect(packageJson.scripts.desktop).toBe("node scripts/desktop-dev.mjs");
    expect(tauriConfig.build.beforeDevCommand).toBe("npm run dev");
    expect(launcher.indexOf('["run", "sidecar:dev"]')).toBeGreaterThanOrEqual(0);
    expect(launcher.indexOf('"dev",')).toBeGreaterThan(launcher.indexOf('["run", "sidecar:dev"]'));
  });
});
