import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import NoticeDialog from "./NoticeDialog";
import { localizeDiagnostic, setLanguagePreference } from "./i18n";

describe("notice dialog links", () => {
  it("offers an explicit action for an HTTP link", () => {
    const html = renderToStaticMarkup(
      <NoticeDialog
        title="触发链接"
        message="https://example.test/path"
        link="https://example.test/path"
        onClose={() => {}}
      />,
    );
    expect(html).toContain("打开链接");
    expect(html).toContain("关闭");
  });

  it("renders an unsafe link as inert notice text", () => {
    const html = renderToStaticMarkup(
      <NoticeDialog
        title="触发链接"
        message="javascript:alert(1)"
        link="javascript:alert(1)"
        onClose={() => {}}
      />,
    );
    expect(html).toContain("javascript:alert(1)");
    expect(html).not.toContain("打开链接");
    expect(html).toContain("确定");
  });
});

describe("notice translation boundary", () => {
  it("never translates ordinary user notices even when they resemble a native error", () => {
    setLanguagePreference("en");
    const message = "OneKey 已被删除，请刷新后重试";
    const props = { title: "User notice", message, onClose: () => {} };
    expect(renderToStaticMarkup(createElement(NoticeDialog, props))).toContain(message);
    const diagnostic = renderToStaticMarkup(createElement(NoticeDialog, { ...props, diagnostic: true }));
    expect(diagnostic).toContain(localizeDiagnostic(message));
    expect(diagnostic).not.toContain(message);
    expect(props.message).toBe(message);
  });

  it("changes diagnostic presentation with language while preserving arbitrary path content", () => {
    const message = "目标目录路径已被非目录占用: /tmp/العربية/会话/{1}";
    setLanguagePreference("en");
    const english = localizeDiagnostic(message);
    setLanguagePreference("ar");
    const arabic = localizeDiagnostic(message);
    expect(arabic).not.toBe(english);
    expect(arabic).not.toContain("native-message-");
    expect(english).toContain("/tmp/العربية/会话/{1}");
    expect(arabic).toContain("/tmp/العربية/会话/{1}");
  });
});
