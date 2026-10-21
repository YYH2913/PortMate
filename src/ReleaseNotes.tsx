import { memo, type ReactNode } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { normalizeTerminalWebLink, openIsolatedWebLink } from "./terminal-web-link";

const plugins = [remarkGfm];

function releaseLink(value: string, releaseUrl: string): string {
  if (!value) return "";
  try {
    const base = normalizeTerminalWebLink(releaseUrl) ?? "https://github.com/YYH2913/PortMate/releases";
    return normalizeTerminalWebLink(new URL(value, base).href) ?? "";
  } catch {
    return "";
  }
}

export default memo(function ReleaseNotes({ notes, releaseUrl, disabled = false }: {
  notes: string;
  releaseUrl: string;
  disabled?: boolean;
}) {
  function link(href: string | undefined, children: ReactNode, title?: string) {
    if (!href) return <span>{children}</span>;
    return <a href={href} title={title} target="_blank" rel="noopener noreferrer"
      aria-disabled={disabled || undefined} tabIndex={disabled ? -1 : undefined}
      onClick={(event) => {
        event.preventDefault();
        if (!disabled) openIsolatedWebLink(href);
      }}>{children}</a>;
  }

  return (
    <div className="update-release-notes">
      <Markdown remarkPlugins={plugins} skipHtml urlTransform={(value) => releaseLink(value, releaseUrl)} components={{
        a: ({ href, children, title }) => link(href, children, title),
        // Keep release images accessible without fetching remote content when
        // opening the dialog. The app's CSP does not allow remote images.
        img: ({ src, alt, title }) => link(typeof src === "string" ? src : undefined, alt || title || (typeof src === "string" ? src : ""), title),
        table: ({ children }) => <div className="update-notes-table"><table>{children}</table></div>,
      }}>{notes}</Markdown>
    </div>
  );
});
