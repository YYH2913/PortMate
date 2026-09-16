import { t, useLocale, localizeDiagnostic } from "./i18n";
import { ExternalLink, X } from "lucide-react";
import { normalizeTerminalWebLink, openIsolatedWebLink } from "./terminal-web-link";

export default function NoticeDialog({
  title,
  message,
  diagnostic = false,
  link,
  onClose,
}: {
  title: string;
  message: string;
  diagnostic?: boolean;
  link?: string;
  onClose: () => void;
}) {
  useLocale();
  const safeLink = link ? normalizeTerminalWebLink(link) : null;
  return (
    <div className="dialog-backdrop notice-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) onClose();
    }}>
      <section className="wind-dialog notice-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{title}</strong>
          <button onClick={onClose}><X size={20} /></button>
        </header>
        <div className="notice-content">{diagnostic ? localizeDiagnostic(message) : message}</div>
        <footer className="notice-actions">
          {safeLink ? (
            <button onClick={() => {
              if (openIsolatedWebLink(safeLink)) onClose();
            }}><ExternalLink size={15} />{t("open-link")}</button>
          ) : null}
          <button onClick={onClose}>{safeLink ? t("close") : t("ok")}</button>
        </footer>
      </section>
    </div>
  );
}
