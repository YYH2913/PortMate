import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { version as packageVersion } from "../../package.json";
import { setLanguagePreference } from "../../src/i18n";
import AboutDialog, {
  AboutExternalLink,
  resolveAboutVersion,
  shouldInterceptExternalLinkClick,
} from "../../src/AboutDialog";

describe("about dialog", () => {
  it("shows product description, capabilities, license, project links, and open-source acknowledgements", () => {
    setLanguagePreference("zh");
    const html = renderToStaticMarkup(<AboutDialog onClose={() => {}} />);

    expect(html).toContain("关于 PortMate");
    expect(html).toContain(`v${packageVersion}`);
    expect(html).toContain("Apache License 2.0");
    expect(html).toContain("YYH2913");
    expect(html).toContain("https://github.com/YYH2913");
    expect(html).toContain("核心能力");
    expect(html).toContain("多协议连接");
    expect(html).toContain("Profile 级信任隔离");
    expect(html).toContain("一体化工作区");
    expect(html).toContain("可选 MCP Bridge");
    expect(html).toContain("Beta 公测版");
    expect(html).not.toContain("Alpha");
    expect(html).toContain("Linux、Windows 与 macOS");
    expect(html).toContain("GitHub 主页");
    expect(html).toContain("github.com/YYH2913");
    expect(html).toContain("项目链接");
    expect(html).toContain("问题反馈");
    expect(html).toContain("开源许可");
    expect(html).not.toContain("版权所有");
    expect(html).toContain("开源致谢");
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
    expect(html).toContain(`复制版本 v${packageVersion}`);
    expect(html).toContain("about-hero-icon");
    expect(html).toContain('href="https://github.com/YYH2913/PortMate"');
    expect(html).toContain('href="https://github.com/YYH2913"');
  });

  it("renders English beta copy, Arabic RTL path isolation, and scoped heading ids", () => {
    setLanguagePreference("en");
    const html = renderToStaticMarkup(<AboutDialog onClose={() => {}} onCheckUpdates={() => {}} />);

    expect(html).toContain("About PortMate");
    expect(html).toContain("Public beta");
    expect(html).not.toContain("Alpha");
    expect(html).toContain("Core capabilities");
    expect(html).toContain("Project links");
    expect(html).toContain("Open-source acknowledgements");
    expect(html).toContain("PortMate does not include a built-in AI assistant");
    expect(html).toContain("Bundled monospace terminal font");

    const labelledByIds = [...html.matchAll(/aria-labelledby="([^"]+)"/g)].map((match) => match[1]);
    expect(labelledByIds).toHaveLength(4);
    expect(new Set(labelledByIds).size).toBe(4);
    for (const id of labelledByIds) {
      expect(html).toContain(`id="${id}"`);
    }

    setLanguagePreference("ar");
    const arHtml = renderToStaticMarkup(<AboutDialog onClose={() => {}} />);
    expect(arHtml).toContain("\u2066~/.ssh/known_hosts\u2069");
    expect(arHtml).toContain("بيئة تشغيل متعددة المنصات لتطبيقات سطح المكتب");
    expect(arHtml).toContain("محاكاة الطرفية وعرضها");
  });

  it("enforces exact external link destinations, security attributes, and unsafe protocol fallback", () => {
    setLanguagePreference("en");
    const html = renderToStaticMarkup(<AboutDialog onClose={() => {}} />);
    const anchorTags = [...html.matchAll(/<a\s+[^>]*>/g)].map((match) => match[0]);

    expect(anchorTags).toHaveLength(15);
    const urls = anchorTags.map((tag) => {
      expect(tag).toContain('target="_blank"');
      expect(tag).toContain('rel="noopener noreferrer"');
      const href = tag.match(/href="([^"]+)"/)?.[1];
      const dataUrl = tag.match(/data-url="([^"]+)"/)?.[1];
      expect(href).toMatch(/^https:\/\//);
      expect(dataUrl).toBe(href);
      return href;
    });

    expect(urls).toEqual([
      "https://github.com/YYH2913",
      "https://github.com/YYH2913/PortMate",
      "https://github.com/YYH2913",
      "https://github.com/YYH2913/PortMate#readme",
      "https://github.com/YYH2913/PortMate/issues",
      "https://github.com/YYH2913/PortMate/releases",
      "https://github.com/YYH2913/PortMate/blob/main/LICENSE",
      "https://github.com/tauri-apps/tauri",
      "https://github.com/facebook/react",
      "https://github.com/xtermjs/xterm.js",
      "https://www.libssh.org/",
      "https://github.com/warp-tech/russh",
      "https://github.com/AspectUnk/russh-sftp",
      "https://github.com/JetBrains/JetBrainsMono",
      "https://github.com/YYH2913/PortMate/blob/main/THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt",
    ]);

    const unsafeHtml = renderToStaticMarkup(
      <AboutExternalLink url="javascript:alert(1)" className="about-link">Unsafe</AboutExternalLink>,
    );
    expect(unsafeHtml).not.toContain("<a");
    expect(unsafeHtml).toContain("<span");
    expect(unsafeHtml).toContain("Unsafe");
  });

  it("renders the update action button and enables it only when onCheckUpdates is provided", () => {
    setLanguagePreference("en");
    const disabledHtml = renderToStaticMarkup(<AboutDialog onClose={() => {}} />);
    const enabledHtml = renderToStaticMarkup(<AboutDialog onClose={() => {}} onCheckUpdates={() => {}} />);

    expect(disabledHtml).toMatch(/<button[^>]*class="about-primary"[^>]*disabled=""[^>]*>.*Check for updates<\/button>/);
    expect(enabledHtml).toMatch(/<button[^>]*class="about-primary"[^>]*>.*Check for updates<\/button>/);
    expect(enabledHtml).not.toMatch(/<button[^>]*class="about-primary"[^>]*disabled/);
  });

  it("normalizes version strings and filters external link click interception", () => {
    expect(resolveAboutVersion(" 0.1.16 ")).toBe("0.1.16");
    expect(resolveAboutVersion("")).toBe("unknown");
    expect(resolveAboutVersion("   ")).toBe("unknown");
    expect(resolveAboutVersion(null)).toBe("unknown");
    expect(resolveAboutVersion(123)).toBe("unknown");

    const plainLeftClick = {
      defaultPrevented: false,
      button: 0,
      metaKey: false,
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
    };
    expect(shouldInterceptExternalLinkClick(plainLeftClick)).toBe(true);
    expect(shouldInterceptExternalLinkClick({ ...plainLeftClick, defaultPrevented: true })).toBe(false);
    expect(shouldInterceptExternalLinkClick({ ...plainLeftClick, button: 1 })).toBe(false);
    expect(shouldInterceptExternalLinkClick({ ...plainLeftClick, metaKey: true })).toBe(false);
    expect(shouldInterceptExternalLinkClick({ ...plainLeftClick, ctrlKey: true })).toBe(false);
    expect(shouldInterceptExternalLinkClick({ ...plainLeftClick, shiftKey: true })).toBe(false);
    expect(shouldInterceptExternalLinkClick({ ...plainLeftClick, altKey: true })).toBe(false);
  });
});
