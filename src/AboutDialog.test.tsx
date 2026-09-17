import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import packageJson from "../package.json";
import { setLanguagePreference } from "./i18n";
import AboutDialog from "./AboutDialog";

describe("about dialog", () => {
  it("shows product description, capabilities, license, project links, and open-source acknowledgements", () => {
    setLanguagePreference("zh");
    const html = renderToStaticMarkup(<AboutDialog onClose={() => {}} />);

    expect(html).toContain("关于 PortMate");
    expect(html).toContain(`v${packageJson.version}`);
    expect(html).toContain("Apache License 2.0");
    expect(html).toContain("YYH2913");
    expect(html).toContain("https://github.com/YYH2913");
    expect(html).toContain("PortMate 覆盖的能力");
    expect(html).toContain("Profile 级信任隔离");
    expect(html).toContain("可选 MCP Bridge");
    expect(html).toContain("Alpha 预览");
    expect(html).toContain("Linux、Windows 与 macOS");
    expect(html).toContain("GitHub 主页");
    expect(html).toContain("github.com/YYH2913");
    expect(html).toContain("项目链接");
    expect(html).toContain("问题反馈");
    expect(html).toContain("开源许可");
    expect(html).not.toContain("版权所有");
    expect(html).toContain("使用的开源代码");
    expect(html).toContain("JetBrains Mono");
    expect(html).toContain("SIL OFL-1.1");
    expect(html).toContain("Linux GSSAPI SSH 传输");
    expect(html).toContain("warp-tech/russh");
    expect(html).toContain("AspectUnk/russh-sftp");
    expect(html).toContain("https://github.com/YYH2913/PortMate/issues");
    expect(html).toContain("MCP 是可选控制面");
    expect(html).toContain("PortMate 不内置 AI 助手");
    expect(html).toContain("portmate-mcp");
    expect(html).toContain("~/.ssh/known_hosts");
  });
});
