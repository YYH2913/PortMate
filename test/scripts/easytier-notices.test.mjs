import { mkdtempSync, readFileSync, rmSync, writeFileSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { copyEasyTierNotices, easyTierNotices, verifyEasyTierNotices } from "../../scripts/easytier-notices.mjs";

const roots = [];
afterEach(() => roots.splice(0).forEach(path => rmSync(path, { recursive: true, force: true })));

describe("EasyTier redistribution notices", () => {
  it("maps all upstream license and attribution texts into every native package", () => {
    const config = JSON.parse(readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url)));
    for (const name of easyTierNotices) expect(config.bundle.resources[`../licenses/${name}`]).toBe(`THIRD_PARTY_LICENSES/${name}`);
    const notice = readFileSync(new URL("../../licenses/EasyTier-NOTICE.txt", import.meta.url), "utf8");
    for (const text of ["2.6.4", "8428a89d2dabc94c97d370ec607c6ca142473626", "NOT included", "https://npcap.com/", "lawful", "independent", "LGPL"]) expect(notice).toContain(text);
    expect(notice).toContain("without making another disk copy");
    expect(notice).toContain("downloading, installing or");
    const windows = readFileSync(new URL("../../src-tauri/src/easytier/npcap/windows.rs", import.meta.url), "utf8");
    expect(windows).toContain("constitutes agreement to its EULA");
    expect(windows).not.toContain("std::io::copy");
    for (const language of ["en", "zh", "fr", "es", "ru", "ar"]) {
      const messages = JSON.parse(readFileSync(new URL(`../../src/locales/${language}.json`, import.meta.url), "utf8"));
      expect(messages["npcap-license-limits"]).toBeTruthy();
      expect(messages["npcap-license-limits"]).not.toMatch(/does not accept|下载不代表|не означает|ne vaut pas|no acepta|لا يعني/);
    }
  });
  it("copies exact notices and rejects deletion, tampering and symlink substitutions", () => {
    const root = mkdtempSync(join(tmpdir(), "portmate-notices-")); roots.push(root);
    const directory = join(root, "THIRD_PARTY_LICENSES");
    expect(copyEasyTierNotices(directory).files).toEqual(easyTierNotices);
    expect(verifyEasyTierNotices(directory).files.length).toBe(6);
    const path = join(directory, easyTierNotices[0]);
    writeFileSync(path, "modified"); expect(() => verifyEasyTierNotices(directory)).toThrow("modified");
    copyEasyTierNotices(directory); rmSync(path); expect(() => verifyEasyTierNotices(directory)).toThrow();
    if (process.platform !== "win32") { const other = join(root, "other"); writeFileSync(other, "other"); symlinkSync(other, path); expect(() => verifyEasyTierNotices(directory)).toThrow("linked"); }
  });
});
