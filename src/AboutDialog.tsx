import { useEffect, useId, useRef } from "react";
import type {
  KeyboardEvent as ReactKeyboardEvent,
  MouseEvent as ReactMouseEvent,
  ReactNode,
} from "react";
import {
  ArrowRightLeft,
  Check,
  CircleAlert,
  Code2,
  Copy,
  ExternalLink,
  FileText,
  GitBranch,
  Network,
  RefreshCw,
  Scale,
  Shield,
  SquareTerminal,
  User,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { version as packageVersion } from "../package.json";
import type enCatalog from "./locales/en.json";
import { t, useLocale } from "./i18n";
import PortMateIcon from "./PortMateIcon";
import { normalizeTerminalWebLink, openIsolatedWebLink } from "./terminal-web-link";
import { useClipboardFeedback } from "./use-clipboard-feedback";

type AboutMessageKey = keyof typeof enCatalog;

export interface AboutDialogProps {
  onClose: () => void;
  onCheckUpdates?: () => void;
}

interface AboutProjectLink {
  readonly labelKey: AboutMessageKey;
  readonly detailKey: AboutMessageKey;
  readonly url: string;
  readonly icon: LucideIcon;
}

interface AboutCapability {
  readonly titleKey: AboutMessageKey;
  readonly detailKey: AboutMessageKey;
  readonly icon: LucideIcon;
}

interface AboutOpenSourceComponent {
  readonly name: string;
  readonly roleKey: AboutMessageKey;
  readonly license: string;
  readonly url: string;
}

interface AboutMetaEntry {
  readonly key: string;
  readonly labelKey: AboutMessageKey;
  readonly content: ReactNode;
}

interface AboutExternalLinkProps {
  url: string;
  className?: string;
  title?: string;
  children: ReactNode;
}

interface AboutSectionProps {
  title: string;
  subtitle: string;
  children: ReactNode;
}

const PROJECT_URL = "https://github.com/YYH2913/PortMate";
const AUTHOR_URL = "https://github.com/YYH2913";
const AUTHOR_LOGIN = "YYH2913";
const PROJECT_LICENSE = "Apache License 2.0";
const FONT_LICENSE_URL = `${PROJECT_URL}/blob/main/THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt`;

const ABOUT_PROJECT_LINKS: readonly AboutProjectLink[] = [
  { labelKey: "project-homepage", detailKey: "source-code-release-notes-and-contribution-guide", url: PROJECT_URL, icon: GitBranch },
  { labelKey: "github-profile", detailKey: "github-com-yyh2913", url: AUTHOR_URL, icon: User },
  { labelKey: "documentation", detailKey: "readme-and-mcp-api-reference", url: `${PROJECT_URL}#readme`, icon: FileText },
  { labelKey: "report-an-issue", detailKey: "report-a-bug-or-request-a-feature", url: `${PROJECT_URL}/issues`, icon: Code2 },
  { labelKey: "releases", detailKey: "downloads-and-changelog", url: `${PROJECT_URL}/releases`, icon: RefreshCw },
  { labelKey: "open-source-licenses", detailKey: "full-apache-license-2-0-text", url: `${PROJECT_URL}/blob/main/LICENSE`, icon: Scale },
];

const ABOUT_CAPABILITIES: readonly AboutCapability[] = [
  { titleKey: "supported-protocols", detailKey: "ssh-shell-serial-telnet-tcp-and-tmux", icon: SquareTerminal },
  { titleKey: "profile-isolated-trust", detailKey: "host-keys-identities-and-jump-hosts-stay-per-profile", icon: Shield },
  { titleKey: "workspace-and-transfers", detailKey: "split-panes-file-transfers-tunnels-logs-and-sysmon", icon: ArrowRightLeft },
  { titleKey: "optional-mcp-bridge", detailKey: "no-built-in-ai-external-hosts-need-an-explicit-grant", icon: Network },
];

const ABOUT_OPEN_SOURCE_COMPONENTS: readonly AboutOpenSourceComponent[] = [
  { name: "Tauri", roleKey: "cross-platform-desktop-runtime", license: "Apache-2.0 / MIT", url: "https://github.com/tauri-apps/tauri" },
  { name: "React", roleKey: "workspace-ui-framework", license: "MIT", url: "https://github.com/facebook/react" },
  { name: "xterm.js", roleKey: "terminal-emulation-and-rendering", license: "MIT", url: "https://github.com/xtermjs/xterm.js" },
  { name: "libssh", roleKey: "linux-gssapi-ssh-transport", license: "LGPL-2.1", url: "https://www.libssh.org/" },
  { name: "russh", roleKey: "rust-ssh-protocol-implementation", license: "Apache-2.0", url: "https://github.com/warp-tech/russh" },
  { name: "russh-sftp", roleKey: "rust-sftp-subsystem", license: "Apache-2.0", url: "https://github.com/AspectUnk/russh-sftp" },
  { name: "JetBrains Mono", roleKey: "bundled-monospace-font", license: "SIL OFL-1.1", url: "https://github.com/JetBrains/JetBrainsMono" },
];

export function resolveAboutVersion(value: unknown): string {
  if (typeof value !== "string") return "unknown";
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : "unknown";
}

export function shouldInterceptExternalLinkClick(
  event: Pick<ReactMouseEvent, "defaultPrevented" | "button" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">,
): boolean {
  return !event.defaultPrevented
    && event.button === 0
    && !event.metaKey
    && !event.ctrlKey
    && !event.shiftKey
    && !event.altKey;
}

export function AboutExternalLink({ url, className, title, children }: AboutExternalLinkProps) {
  const safeUrl = normalizeTerminalWebLink(url);
  if (!safeUrl) {
    return (
      <span className={className} title={title}>
        {children}
      </span>
    );
  }

  return (
    <a
      href={safeUrl}
      target="_blank"
      rel="noopener noreferrer"
      className={className}
      data-url={safeUrl}
      title={title}
      onClick={(event) => {
        if (shouldInterceptExternalLinkClick(event) && openIsolatedWebLink(safeUrl)) {
          event.preventDefault();
        }
      }}
    >
      {children}
    </a>
  );
}

function AboutSection({ title, subtitle, children }: AboutSectionProps) {
  const headingId = useId();
  return (
    <section className="about-section" aria-labelledby={headingId}>
      <header>
        <h2 id={headingId}>{title}</h2>
        <p>{subtitle}</p>
      </header>
      {children}
    </section>
  );
}

function AboutHero({ versionLabel }: { versionLabel: string }) {
  const { feedback, busy: copyBusy, copy } = useClipboardFeedback();
  const copyState = feedback?.status;
  const copyLabel = t("copy-version", [versionLabel]);
  const copyMessage = copyState === "copied"
    ? t("copied")
    : copyState === "error"
      ? t("clipboard-copy-failed")
      : "";
  const CopyIcon = copyState === "copied" ? Check : copyState === "error" ? CircleAlert : Copy;

  return (
    <section className="about-hero" aria-label={t("portmate-product-information")}>
      <PortMateIcon size={56} variant="badge" className="about-hero-icon" />
      <div className="about-hero-copy">
        <div className="about-name-row">
          <h1><bdi>PortMate</bdi></h1>
          <button
            type="button"
            className="about-version"
            aria-label={copyLabel}
            aria-busy={copyBusy || undefined}
            title={copyMessage || copyLabel}
            data-copy-state={copyState}
            onClick={() => void copy("version", versionLabel)}
          >
            <code dir="ltr">{versionLabel}</code>
            <CopyIcon size={13} aria-hidden="true" />
          </button>
        </div>
        <span className="sr-only" role="status" aria-live="polite" aria-atomic="true">
          {copyState === "copied" ? copyMessage : ""}
        </span>
        {copyState === "error" ? (
          <span className="about-copy-message" role="alert" data-copy-state="error">
            {copyMessage}
          </span>
        ) : null}
        <p>{t("a-cross-platform-terminal-workspace-for-ssh-serial-connections")}</p>
        <small>{t("sessions-terminals-file-transfers-diagnostics-and-a-permissioned-mcp")}</small>
      </div>
    </section>
  );
}

function AboutMetadataGrid({ version }: { version: string }) {
  const entries: readonly AboutMetaEntry[] = [
    { key: "version", labelKey: "version", content: <bdi>{version}</bdi> },
    { key: "status", labelKey: "release-status", content: t("beta-release") },
    { key: "license", labelKey: "license", content: <bdi>{PROJECT_LICENSE}</bdi> },
    { key: "product-type", labelKey: "product-type", content: t("tauri-v2-desktop-application") },
    { key: "platforms", labelKey: "platforms", content: t("linux-windows-and-macos") },
    {
      key: "maintainer",
      labelKey: "maintainer",
      content: (
        <AboutExternalLink
          url={AUTHOR_URL}
          className="about-author-link"
          title={t("open-the-github-homepage")}
        >
          <bdi>{AUTHOR_LOGIN}</bdi>
          <ExternalLink size={12} aria-hidden="true" />
        </AboutExternalLink>
      ),
    },
  ];

  return (
    <dl className="about-meta" aria-label={t("portmate-metadata")}>
      {entries.map(({ key, labelKey, content }) => (
        <div key={key}>
          <dt>{t(labelKey)}</dt>
          <dd>{content}</dd>
        </div>
      ))}
    </dl>
  );
}

export default function AboutDialog({ onClose, onCheckUpdates }: AboutDialogProps) {
  useLocale();
  const titleId = useId();
  const closeRef = useRef<HTMLButtonElement>(null);
  const version = resolveAboutVersion(packageVersion);
  const versionLabel = `v${version}`;

  // Global modal isolation (inert background, Tab wrapping, and focus return) is managed by
  // useModalInteractionBoundary() in main.tsx; AboutDialog only sets initial focus and Escape.
  useEffect(() => {
    closeRef.current?.focus({ preventScroll: true });
  }, []);

  function handleDialogKeyDown(event: ReactKeyboardEvent<HTMLElement>) {
    if (event.key === "Escape" && !event.defaultPrevented && !event.nativeEvent.isComposing) {
      event.preventDefault();
      event.stopPropagation();
      onClose();
    }
  }

  return (
    <div
      className="dialog-backdrop about-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <section
        className="wind-dialog about-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onKeyDown={handleDialogKeyDown}
      >
        <header className="about-titlebar">
          <PortMateIcon size={18} variant="mark" />
          <strong id={titleId}>{t("about-portmate")}</strong>
          <button
            ref={closeRef}
            type="button"
            className="about-close"
            title={t("close")}
            aria-label={t("close-about-portmate")}
            onClick={onClose}
          >
            <X size={17} aria-hidden="true" />
          </button>
        </header>

        <div className="about-content">
          <AboutHero versionLabel={versionLabel} />
          <AboutMetadataGrid version={version} />

          <AboutSection
            title={t("what-portmate-covers")}
            subtitle={t("desktop-operator-and-optional-authorized-mcp-access")}
          >
            <dl className="about-capabilities">
              {ABOUT_CAPABILITIES.map(({ titleKey, detailKey, icon: Icon }) => (
                <div key={titleKey}>
                  <dt>
                    <span className="about-item-icon"><Icon size={15} aria-hidden="true" /></span>
                    {t(titleKey)}
                  </dt>
                  <dd>{t(detailKey)}</dd>
                </div>
              ))}
            </dl>
          </AboutSection>

          <AboutSection
            title={t("project-links")}
            subtitle={t("source-code-documentation-issue-tracking-and-releases")}
          >
            <div className="about-link-grid">
              {ABOUT_PROJECT_LINKS.map(({ labelKey, detailKey, url, icon: Icon }) => (
                <AboutExternalLink key={labelKey} url={url} className="about-link">
                  <span className="about-item-icon"><Icon size={15} aria-hidden="true" /></span>
                  <span className="about-link-copy">
                    <strong>{t(labelKey)}</strong>
                    <small><bdi>{t(detailKey)}</bdi></small>
                  </span>
                  <ExternalLink size={13} aria-hidden="true" />
                </AboutExternalLink>
              ))}
            </div>
          </AboutSection>

          <AboutSection
            title={t("open-source-software")}
            subtitle={t("portmate-is-built-on-the-following-open-source-projects")}
          >
            <ul className="about-components" role="list" aria-label={t("open-source-components")}>
              {ABOUT_OPEN_SOURCE_COMPONENTS.map(({ name, roleKey, license, url }) => (
                <li className="about-component" key={name}>
                  <AboutExternalLink url={url} title={t("open-the-project-homepage", [name])}>
                    <bdi>{name}</bdi>
                    <ExternalLink size={12} aria-hidden="true" />
                  </AboutExternalLink>
                  <span>{t(roleKey)}</span>
                  <code dir="ltr">{license}</code>
                </li>
              ))}
            </ul>
            <AboutExternalLink url={FONT_LICENSE_URL} className="about-license-link">
              <Scale size={14} aria-hidden="true" />
              {t("view-the-bundled-font-license")}
            </AboutExternalLink>
          </AboutSection>

          <p className="about-notice">{t("portmate-is-under-active-development-verify-security-compatibility-and")}</p>
        </div>

        <footer className="about-actions">
          <button type="button" className="about-secondary" onClick={onClose}>
            {t("close")}
          </button>
          <button type="button" className="about-primary" onClick={onCheckUpdates} disabled={!onCheckUpdates}>
            <RefreshCw size={14} aria-hidden="true" />
            {t("check-for-updates")}
          </button>
        </footer>
      </section>
    </div>
  );
}
