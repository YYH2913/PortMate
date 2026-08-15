import { useState } from "react";
import type { FormEvent } from "react";
import { ArrowDown, ArrowUp, Pencil, Play, Plus, Trash2, X } from "lucide-react";
import {
  createQuickCommandId,
  limitQuickCommandLabelInput,
  MAX_QUICK_COMMANDS,
  moveQuickCommand,
  normalizeQuickCommandLibrary,
  normalizeQuickCommandText,
  quickCommandLibraryHasUnsavedChanges,
} from "./quick-command-state";
import type { QuickCommand } from "./quick-command-state";

export default function QuickCommandDialog({
  commands,
  onSave,
  onClose,
}: {
  commands: QuickCommand[];
  onSave: (commands: QuickCommand[]) => void;
  onClose: () => void;
}) {
  const [items, setItems] = useState<QuickCommand[]>(() => commands.map((command) => ({ ...command })));
  const [selectedId, setSelectedId] = useState(commands[0]?.id ?? "");
  const [error, setError] = useState("");
  const selectedIndex = items.findIndex((item) => item.id === selectedId);
  const selected = selectedIndex >= 0 ? items[selectedIndex] : undefined;
  const dirty = quickCommandLibraryHasUnsavedChanges(items, commands);

  function addCommand() {
    if (items.length >= MAX_QUICK_COMMANDS) {
      setError(`快速命令最多保存 ${MAX_QUICK_COMMANDS} 条。`);
      return;
    }
    const command: QuickCommand = {
      id: createQuickCommandId(),
      label: `新命令 ${items.length + 1}`,
      command: "",
      appendEnter: true,
    };
    setItems((current) => [...current, command]);
    setSelectedId(command.id);
    setError("");
  }

  function updateSelected(patch: Partial<QuickCommand>) {
    setItems((current) => current.map((item) => item.id === selectedId ? { ...item, ...patch } : item));
    setError("");
  }

  function removeSelected() {
    if (!selected) return;
    const next = items.filter((item) => item.id !== selected.id);
    setItems(next);
    setSelectedId(next[Math.min(selectedIndex, next.length - 1)]?.id ?? "");
    setError("");
  }

  function moveSelected(offset: -1 | 1) {
    if (!selected) return;
    setItems((current) => moveQuickCommand(current, selected.id, offset));
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    if (items.some((item) => !item.label.trim() || !item.command)) {
      setError("每条快速命令都需要名称和命令内容。");
      return;
    }
    const normalized = normalizeQuickCommandLibrary({ version: 1, items });
    if (normalized.items.length !== items.length) {
      setError("快速命令包含无效内容，请检查后重试。");
      return;
    }
    onSave(normalized.items);
  }

  function closeDialog() {
    if (dirty && !window.confirm("快速命令有未保存的更改，关闭窗口将放弃这些内容。是否继续？")) return;
    onClose();
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && closeDialog()}>
      <form
        className="wind-dialog utility-dialog quick-command-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="quick-command-dialog-title"
        onSubmit={submit}
      >
        <header className="dialog-title">
          <span className="app-icon" />
          <strong id="quick-command-dialog-title">快速命令</strong>
          <button type="button" title="关闭" aria-label="关闭快速命令" onClick={closeDialog}><X size={20} /></button>
        </header>
        <section className="quick-command-content">
          <aside className="quick-command-list">
            <header>
              <strong>命令</strong>
              <span>{items.length}/{MAX_QUICK_COMMANDS}</span>
              <button type="button" title="添加快速命令" aria-label="添加快速命令" onClick={addCommand} disabled={items.length >= MAX_QUICK_COMMANDS}><Plus size={14} /></button>
            </header>
            <div role="listbox" aria-label="快速命令列表">
              {items.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  role="option"
                  aria-selected={item.id === selectedId}
                  className={item.id === selectedId ? "active" : ""}
                  onClick={() => setSelectedId(item.id)}
                >
                  {item.appendEnter ? <Play size={12} /> : <Pencil size={12} />}
                  <span>{item.label || "未命名命令"}</span>
                </button>
              ))}
              {!items.length ? <div className="quick-command-list-empty">没有快速命令</div> : null}
            </div>
          </aside>
          <section className="quick-command-editor">
            {selected ? (
              <>
                <label>
                  <span>名称</span>
                  <input aria-label="快速命令名称" value={selected.label} onChange={(event) => updateSelected({ label: limitQuickCommandLabelInput(event.target.value) })} />
                </label>
                <label className="quick-command-text-field">
                  <span>命令</span>
                  <textarea aria-label="快速命令内容" value={selected.command} spellCheck={false} onChange={(event) => updateSelected({ command: normalizeQuickCommandText(event.target.value) })} />
                </label>
                <label className="quick-command-enter-toggle">
                  <input type="checkbox" checked={selected.appendEnter} onChange={(event) => updateSelected({ appendEnter: event.target.checked })} />
                  <span>追加回车并执行</span>
                </label>
                <div className="quick-command-editor-actions">
                  <button type="button" title="上移" aria-label="上移快速命令" disabled={selectedIndex <= 0} onClick={() => moveSelected(-1)}><ArrowUp size={14} /></button>
                  <button type="button" title="下移" aria-label="下移快速命令" disabled={selectedIndex < 0 || selectedIndex >= items.length - 1} onClick={() => moveSelected(1)}><ArrowDown size={14} /></button>
                  <span />
                  <button type="button" className="danger" title="删除" aria-label="删除快速命令" onClick={removeSelected}><Trash2 size={14} /></button>
                </div>
              </>
            ) : (
              <div className="quick-command-editor-empty">
                <button type="button" onClick={addCommand}><Plus size={14} /><span>添加命令</span></button>
              </div>
            )}
          </section>
        </section>
        <footer className="utility-actions quick-command-dialog-actions">
          {error ? <span role="alert">{error}</span> : <span />}
          <button type="button" onClick={closeDialog}>取消</button>
          <button type="submit">保存</button>
        </footer>
      </form>
    </div>
  );
}
