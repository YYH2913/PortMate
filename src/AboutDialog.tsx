import { t, useLocale } from "./i18n";
import { Code2, ExternalLink, FileText, GitBranch, Scale, User, X } from "lucide-react";
import packageJson from "../package.json";
import { normalizeTerminalWebLink, openIsolatedWebLink } from "./terminal-web-link";

const PROJECT_URL = "https://github.com/YYH2913/PortMate";
const AUTHOR_URL = "https://github.com/YYH2913";
const AUTHOR_LOGIN = "YYH2913";

const projectLinks = [
  { label: "project-homepage", detail: "source-code-release-notes-and-contribution-guide", url: PROJECT_URL, icon: GitBranch },
  { label: "github-profile", detail: "github-com-yyh2913", url: AUTHOR_URL, icon: User },
  { label: "documentation", detail: "readme-and-mcp-api-reference", url: `${PROJECT_URL}#readme`, icon: FileText },
  { label: "report-an-issue", detail: "report-a-bug-or-request-a-feature", url: `${PROJECT_URL}/issues`, icon: Code2 },
  { label: "releases", detail: "downloads-and-changelog", url: `${PROJECT_URL}/releases`, icon: ExternalLink },
  { label: "open-source-licenses", detail: "full-apache-license-2-0-text", url: `${PROJECT_URL}/blob/main/LICENSE`, icon: Scale },
] as const;

const capabilities = [
  { title: "supported-protocols", detail: "ssh-shell-serial-telnet-tcp-and-tmux" },
  { title: "profile-isolated-trust", detail: "host-keys-identities-and-jump-hosts-stay-per-profile" },
  { title: "workspace-and-transfers", detail: "split-panes-file-transfers-tunnels-logs-and-sysmon" },
  { title: "optional-mcp-bridge", detail: "no-built-in-ai-external-hosts-need-an-explicit-grant" },
] as const;

const openSourceComponents = [
  { name: "Tauri", role: "cross-platform-desktop-runtime", license: "Apache-2.0 / MIT", url: "https://github.com/tauri-apps/tauri" },
  { name: "React", role: "workspace-ui-framework", license: "MIT", url: "https://github.com/facebook/react" },
  { name: "xterm.js", role: "terminal-emulation-and-rendering", license: "MIT", url: "https://github.com/xtermjs/xterm.js" },
  { name: "libssh", role: "linux-gssapi-ssh-transport", license: "LGPL-2.1", url: "https://www.libssh.org/" },
  { name: "russh", role: "rust-ssh-protocol-implementation", license: "Apache-2.0", url: "https://github.com/warp-tech/russh" },
  { name: "russh-sftp", role: "rust-sftp-subsystem", license: "Apache-2.0", url: "https://github.com/AspectUnk/russh-sftp" },
  { name: "JetBrains Mono", role: "bundled-monospace-font", license: "SIL OFL-1.1", url: "https://github.com/JetBrains/JetBrainsMono" },
] as const;

export default function AboutDialog({ onClose }: { onClose: () => void }) {
  useLocale();
  const version = typeof packageJson.version === "string" ? packageJson.version : "unknown";

  function openLink(value: string) {
    const safeLink = normalizeTerminalWebLink(value);
    if (safeLink) openIsolatedWebLink(safeLink);
  }

  return (
    <div className="dialog-backdrop about-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) onClose();
    }}>
      <section className="wind-dialog about-dialog" role="dialog" aria-modal="true" aria-labelledby="about-title">
        <header className="dialog-title">
          <span className="app-icon" aria-hidden="true" />
          <strong id="about-title">{t("about-portmate")}</strong>
          <button type="button" title={t("close")} aria-label={t("close-about-portmate")} onClick={onClose}><X size={20} /></button>
        </header>

        <div className="about-content">
          <section className="about-hero" aria-label={t("portmate-product-information")}>
            <div className="about-mark" aria-hidden="true"><span>PM</span></div>
            <div className="about-hero-copy">
              <div className="about-name-row"><h1>PortMate</h1><code>v{version}</code></div>
              <p>{t("a-cross-platform-terminal-workspace-for-ssh-serial-connections")}</p>
              <small>{t("sessions-terminals-file-transfers-diagnostics-and-a-permissioned-mcp")}</small>
            </div>
          </section>

          <dl className="about-meta" aria-label={t("portmate-metadata")}>
            <div><dt>{t("version")}</dt><dd>{version}</dd></div>
            <div><dt>{t("release-status")}</dt><dd>{t("alpha-preview")}</dd></div>
            <div><dt>{t("license")}</dt><dd>Apache License 2.0</dd></div>
            <div><dt>{t("product-type")}</dt><dd>{t("tauri-v2-desktop-application")}</dd></div>
            <div><dt>{t("platforms")}</dt><dd>{t("linux-windows-and-macos")}</dd></div>
            <div>
              <dt>{t("maintainer")}</dt>
              <dd>
                <button
                  type="button"
                  className="about-author-link"
                  data-url={AUTHOR_URL}
                  title={t("open-the-github-homepage")}
                  onClick={() => openLink(AUTHOR_URL)}
                >
                  {AUTHOR_LOGIN}
                </button>
              </dd>
            </div>
          </dl>

          <section className="about-section" aria-labelledby="about-capabilities-title">
            <header>
              <div>
                <h2 id="about-capabilities-title">{t("what-portmate-covers")}</h2>
                <p>{t("desktop-operator-and-optional-authorized-mcp-access")}</p>
              </div>
            </header>
            <dl className="about-capabilities">
              {capabilities.map(({ title, detail }) => (
                <div key={title}>
                  <dt>{t(title)}</dt>
                  <dd>{t(detail)}</dd>
                </div>
              ))}
            </dl>
          </section>

          <section className="about-section" aria-labelledby="about-links-title">
            <header><div><h2 id="about-links-title">{t("project-links")}</h2><p>{t("source-code-documentation-issue-tracking-and-releases")}</p></div><ExternalLink size={15} aria-hidden="true" /></header>
            <div className="about-link-grid">
              {projectLinks.map(({ label, detail, url, icon: Icon }) => (
                <button type="button" key={label} className="about-link" data-url={url} onClick={() => openLink(url)}>
                  <Icon size={16} aria-hidden="true" />
                  <span><strong>{t(label)}</strong><small>{t(detail)}</small></span>
                  <ExternalLink size={13} aria-hidden="true" />
                </button>
              ))}
            </div>
          </section>

          <section className="about-section" aria-labelledby="about-open-source-title">
            <header><div><h2 id="about-open-source-title">{t("open-source-software")}</h2><p>{t("portmate-is-built-on-the-following-open-source-projects")}</p></div><Code2 size={15} aria-hidden="true" /></header>
            <div className="about-components" role="list" aria-label={t("open-source-components")}>
              {openSourceComponents.map(({ name, role, license, url }) => (
                <div className="about-component" role="listitem" key={name}>
                  <button type="button" data-url={url} onClick={() => openLink(url)} title={t("open-the-project-homepage", [name])}><strong>{name}</strong><ExternalLink size={12} aria-hidden="true" /></button>
                  <span>{t(role)}</span>
                  <code>{license}</code>
                </div>
              ))}
            </div>
            <button type="button" className="about-license-link" data-url={`${PROJECT_URL}/blob/main/THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt`} onClick={() => openLink(`${PROJECT_URL}/blob/main/THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt`)}><Scale size={14} aria-hidden="true" />{t("view-the-bundled-font-license")}</button>
          </section>

          <p className="about-notice">{t("portmate-is-under-active-development-verify-security-compatibility-and")}</p>
        </div>

        <footer className="about-actions"><button type="button" onClick={onClose}>{t("close")}</button></footer>
      </section>
    </div>
  );
}
