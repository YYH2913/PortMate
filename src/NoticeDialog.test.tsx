import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import NoticeDialog from "./NoticeDialog";

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
