import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fontRoot = join(projectRoot, "src", "assets", "fonts");
const licensePath = join(projectRoot, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt");
const expectedFonts = new Map([
  ["JetBrainsMono-Regular.woff2", "a9cb1cd82332b23a47e3a1239d25d13c86d16c4220695e34b243effa999f45f2"],
  ["JetBrainsMono-SemiBold.woff2", "918edad542a1da608fd2ba8daebaff9ac802309103fe760eed465b8b4e47faf1"],
  ["JetBrainsMono-Bold.woff2", "c503cc5ec5f8b2c7666b7ecda1adf44bd45f2e6579b2eba0fc292150416588a2"],
]);

describe("bundled terminal font assets", () => {
  it("pins the complete upstream JetBrains Mono 2.304 webfonts", () => {
    for (const [fileName, expectedHash] of expectedFonts) {
      const contents = readFileSync(join(fontRoot, fileName));
      expect(contents.subarray(0, 4).toString("ascii")).toBe("wOF2");
      expect(createHash("sha256").update(contents).digest("hex")).toBe(expectedHash);
    }
  });

  it("ships the upstream OFL license as a native bundle resource", () => {
    const license = readFileSync(licensePath, "utf8");
    expect(license).toContain("Copyright 2020 The JetBrains Mono Project Authors");
    expect(license).toContain("SIL OPEN FONT LICENSE Version 1.1");

    const tauriConfig = JSON.parse(
      readFileSync(join(projectRoot, "src-tauri", "tauri.conf.json"), "utf8"),
    );
    expect(tauriConfig.bundle.resources["../THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt"])
      .toBe("THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt");
  });
});
