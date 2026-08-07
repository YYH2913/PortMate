import { ExternalLink, X } from "lucide-react";
import { normalizeTerminalWebLink, openIsolatedWebLink } from "./terminal-web-link";

export default function NoticeDialog({
  title,
  message,
  link,
  onClose,
}: {
  title: string;
  message: string;
  link?: string;
  onClose: () => void;
}) {
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
        <div className="notice-content">{message}</div>
        <footer className="notice-actions">
          {safeLink ? (
            <button onClick={() => {
              if (openIsolatedWebLink(safeLink)) onClose();
            }}><ExternalLink size={15} />打开链接</button>
          ) : null}
          <button onClick={onClose}>{safeLink ? "关闭" : "确定"}</button>
        </footer>
      </section>
    </div>
  );
}
