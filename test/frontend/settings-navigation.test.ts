import { afterEach, describe, expect, it } from "vitest";
import { getLanguageState, setLanguagePreference } from "../../src/i18n";
import { findSettings } from "../../src/settings-navigation";
import { flattenSessionTree, sessionSettingTrees } from "../../src/session-settings-state";

const initial = getLanguageState().preference;
afterEach(() => setLanguagePreference(initial));
describe("settings field navigation", () => {
  it("finds controls in their own scope and uses stable untranslated identifiers", () => {
    setLanguagePreference("zh");
    const pages = flattenSessionTree(sessionSettingTrees.Serial);
    expect(findSettings(pages, "终端 字号")).toContainEqual({ page: "terminal-appearance", field: "font-size-z" });
    expect(findSettings(pages, "波特率")).toEqual([{ page: "serial", field: "baud-rate-b" }]);
    expect(findSettings(["autocomplete", "appearance"], "配色")).toEqual([{ page: "appearance", field: "multicolor-commands-and-output" }]);
    setLanguagePreference("en");
    expect(findSettings(pages, "baud")).toEqual([{ page: "serial", field: "baud-rate-b" }]);
  });
  it("does not list unavailable protocols or empty searches", () => {
    expect(findSettings(flattenSessionTree(sessionSettingTrees.Serial), "SFTP")).toEqual([]);
    expect(findSettings(flattenSessionTree(sessionSettingTrees.SSH), "sftp")).toContainEqual({page:"transfers", field:"SFTP:"});
    expect(findSettings(["appearance"], " ")).toEqual([]);
  });
});
