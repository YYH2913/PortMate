import { X } from "lucide-react";

export default function NoticeDialog({
  title,
  message,
  onClose,
}: {
  title: string;
  message: string;
  onClose: () => void;
}) {
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
          <button onClick={onClose}>确定</button>
        </footer>
      </section>
    </div>
  );
}
