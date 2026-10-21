import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import ReleaseNotes from "../../src/ReleaseNotes";

const releaseUrl = "https://github.com/YYH2913/PortMate/releases/tag/0.1.13";

describe("release notes Markdown", () => {
  it("renders bilingual GitHub headings, lists, tables, quotes, tasks and code", () => {
    const notes = [
      "## Changes / 主要更新", "", "**Faster uploads** and *serial fixes*, ~~old behavior~~.", "",
      "- Keep `\\r\\n` endings", "- 中文说明", "", "1. Download", "2. Install", "",
      "| Platform | Package |", "| --- | --- |", "| Windows | MSI |", "",
      "> Existing sessions are preserved.", "", "- [x] Verified", "",
      "```sh", "printf 'hello\\r\\n'", "```",
    ].join("\n");
    const html = renderToStaticMarkup(<ReleaseNotes notes={notes} releaseUrl={releaseUrl} />);

    expect(html).toContain("<h2>Changes / 主要更新</h2>");
    expect(html).toContain("<strong>Faster uploads</strong>");
    expect(html).toContain("<em>serial fixes</em>");
    expect(html).toContain("<del>old behavior</del>");
    expect(html).toContain("<li>中文说明</li>");
    expect(html).toContain("<ol>");
    expect(html).toContain("<th>Platform</th>");
    expect(html).toContain("<td>MSI</td>");
    expect(html).toContain("<blockquote>");
    expect(html).toContain('type="checkbox"');
    expect(html).toContain('class="language-sh"');
    expect(html).toContain("<code>\\r\\n</code>");
  });

  it("keeps external links isolated and rejects executable URLs and raw HTML", () => {
    const notes = [
      "[Docs](https://github.com/YYH2913/PortMate#readme)", "",
      "[Release assets](../0.1.13)", "",
      "[Script](javascript:alert%281%29)", "[File](file:///C:/Windows/test.exe)",
      "[Data](data:text/html;base64,PHNjcmlwdD4=)", "",
      '<script>window.releaseNotesExecuted = true</script>', "",
      '<img src="x" onerror="alert(1)">', "",
      "![Screenshot](https://images.example.test/release.png)",
    ].join("\n");
    const html = renderToStaticMarkup(<ReleaseNotes notes={notes} releaseUrl={releaseUrl} />);

    expect(html).toContain('href="https://github.com/YYH2913/PortMate#readme"');
    expect(html).toContain('target="_blank" rel="noopener noreferrer"');
    expect(html).toContain('href="https://github.com/YYH2913/PortMate/releases/0.1.13"');
    expect(html).toContain('href="https://images.example.test/release.png"');
    expect(html).toContain(">Screenshot</a>");
    expect(html).toContain("Script");
    for (const unsafe of ["javascript:", "file:", "data:", "<script", "<img", "onerror=", "releaseNotesExecuted"]) {
      expect(html).not.toContain(unsafe);
    }
  });

  it("removes release links from the keyboard tab order during installation", () => {
    const html = renderToStaticMarkup(<ReleaseNotes notes="[Docs](https://example.test/docs)" releaseUrl={releaseUrl} disabled />);
    expect(html).toContain('aria-disabled="true"');
    expect(html).toContain('tabindex="-1"');
  });
});
