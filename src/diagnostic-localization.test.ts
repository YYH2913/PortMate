import { describe, expect, it } from "vitest";
import { createDiagnosticLocalizer } from "./diagnostic-localization";
import templates from "./locales/native-message-templates.json";
import en from "./locales/en.json";
import zh from "./locales/zh.json";

const messages: Record<string, string> = {
  disconnected: "Session is not connected",
  path: "Cannot read {0}",
  wrapped: "Request failed: {0}",
  pair: "Source {0}; target {1}",
};
const localize = createDiagnosticLocalizer([
  { id: "disconnected", source: "会话尚未连接" },
  { id: "path", source: "无法读取 {0}" },
  { id: "wrapped", source: "请求失败: {0}", causeIndices: [0] },
  { id: "pair", source: "从 {0} 到 {1}" },
], (id, values = []) => messages[id].replace(/\{(\d+)\}/g, (_, index) => String(values[Number(index)])));

describe("native diagnostic presentation", () => {
  it("keeps native presentation templates aligned with the locale catalogs", () => {
    const parameters = (value: string) => [...value.matchAll(/\{\d+\}/g)].map(match => match[0]).sort();
    for (const template of templates) {
      expect(Object.hasOwn(en, template.id)).toBe(true);
      expect((zh as Record<string, string>)[template.id]).toBe(template.source);
      expect(parameters((en as Record<string, string>)[template.id])).toEqual(parameters(template.source));
    }
  });
  it("localizes known messages without changing captured names or paths", () => {
    expect(localize("会话尚未连接")).toBe("Session is not connected");
    expect(localize("无法读取 /tmp/会话尚未连接/{1}.txt")).toBe("Cannot read /tmp/会话尚未连接/{1}.txt");
    expect(localize("从 中文源 到 العربية target")).toBe("Source 中文源; target العربية target");
  });

  it("only recurses into explicitly identified error causes", () => {
    expect(localize("请求失败: 会话尚未连接")).toBe("Request failed: Session is not connected");
    expect(localize("无法读取 会话尚未连接")).toBe("Cannot read 会话尚未连接");
  });

  it("preserves unknown output and ambiguous formatted values", () => {
    for (const text of ["device output: 会话尚未连接", "unknown OS error", "从 路径 到 内部 到 目标"]) {
      expect(localize(text)).toBe(text);
    }
  });

  it("bounds long input and preserves line breaks", () => {
    const long = "无法读取 " + "x".repeat(9_000);
    expect(localize(long)).toBe(long);
    expect(localize("会话尚未连接\nunknown\n会话尚未连接")).toBe("Session is not connected\nunknown\nSession is not connected");
  });
});
