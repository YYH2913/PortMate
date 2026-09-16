import { t, useLocale } from "./i18n";
import { RotateCcw, X } from "lucide-react";

export default function WorkspaceViewRenameDialog({
  state,
  onChange,
  onUseSessionName,
  onSave,
  onClose,
}: {
  state: { value: string; sessionName: string };
  onChange: (value: string) => void;
  onUseSessionName: () => void;
  onSave: () => void;
  onClose: () => void;
}) {
  useLocale();
  return (
    <div className="dialog-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="wind-dialog workspace-view-rename-dialog" onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}>
        <header className="dialog-title">
          <span>{t("rename-view")}</span>
          <button type="button" title={t("close")} aria-label={t("close")} onClick={onClose}><X size={18} /></button>
        </header>
        <div className="workspace-view-rename-content">
          <label>
            <span>{t("view-name")}</span>
            <input
              autoFocus
              maxLength={128}
              value={state.value}
              placeholder={state.sessionName}
              onFocus={(event) => event.currentTarget.select()}
              onChange={(event) => onChange(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.preventDefault();
                  onClose();
                }
              }}
            />
          </label>
        </div>
        <footer className="workspace-view-rename-actions">
          <button type="button" className="reset" onClick={onUseSessionName}><RotateCcw size={13} />{t("use-session-name")}</button>
          <span />
          <button type="button" onClick={onClose}>{t("cancel")}</button>
          <button type="submit" className="primary" disabled={!state.value.trim()}>{t("save")}</button>
        </footer>
      </form>
    </div>
  );
}
