import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import packageJson from "../package.json";
import AboutDialog from "./AboutDialog";

describe("about dialog", () => {
  it("shows release metadata, license, project links, and open-source acknowledgements", () => {
    const html = renderToStaticMarkup(<AboutDialog onClose={() => {}} />);

    expect(html).toContain("关于 PortMate");
    expect(html).toContain(`v${packageJson.version}`);
    expect(html).toContain("Apache License 2.0");
    expect(html).toContain("PortMate Contributors");
    expect(html).toContain("项目链接");
    expect(html).toContain("问题反馈");
    expect(html).toContain("使用的开源代码");
    expect(html).toContain("JetBrains Mono");
    expect(html).toContain("SIL OFL-1.1");
    expect(html).toContain("warp-tech/russh");
    expect(html).toContain("AspectUnk/russh-sftp");
    expect(html).toContain("https://github.com/YYH2913/PortMate/issues");
  });
});
