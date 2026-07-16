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
  return (
    <div className="dialog-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="wind-dialog workspace-view-rename-dialog" onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}>
        <header className="dialog-title">
          <span>重命名视图</span>
          <button type="button" title="关闭" aria-label="关闭" onClick={onClose}><X size={18} /></button>
        </header>
        <div className="workspace-view-rename-content">
          <label>
            <span>视图名称</span>
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
          <button type="button" className="reset" onClick={onUseSessionName}><RotateCcw size={13} />使用会话名称</button>
          <span />
          <button type="button" onClick={onClose}>取消</button>
          <button type="submit" className="primary" disabled={!state.value.trim()}>保存</button>
        </footer>
      </form>
    </div>
  );
}
