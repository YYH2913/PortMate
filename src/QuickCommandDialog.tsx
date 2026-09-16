import { t, useLocale, localizeDiagnostic } from "./i18n";
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
  useLocale();
  const [items, setItems] = useState<QuickCommand[]>(() => commands.map((command) => ({ ...command })));
  const [selectedId, setSelectedId] = useState(commands[0]?.id ?? "");
  const [error, setError] = useState("");
  const selectedIndex = items.findIndex((item) => item.id === selectedId);
  const selected = selectedIndex >= 0 ? items[selectedIndex] : undefined;
  const dirty = quickCommandLibraryHasUnsavedChanges(items, commands);

  function addCommand() {
    if (items.length >= MAX_QUICK_COMMANDS) {
      setError(t("at-most-quick-commands-can-be-saved", [MAX_QUICK_COMMANDS]));
      return;
    }
    const command: QuickCommand = {
      id: createQuickCommandId(),
      label: t("new-command", [items.length + 1]),
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
      setError(t("each-quick-command-requires-a-name-and-command-content"));
      return;
    }
    const normalized = normalizeQuickCommandLibrary({ version: 1, items });
    if (normalized.items.length !== items.length) {
      setError(t("quick-commands-contain-invalid-content-check-and-retry"));
      return;
    }
    onSave(normalized.items);
  }

  function closeDialog() {
    if (dirty && !window.confirm(t("quick-commands-have-unsaved-changes-closing-will-discard-them"))) return;
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
          <strong id="quick-command-dialog-title">{t("quick-commands")}</strong>
          <button type="button" title={t("close")} aria-label={t("close-quick-commands")} onClick={closeDialog}><X size={20} /></button>
        </header>
        <section className="quick-command-content">
          <aside className="quick-command-list">
            <header>
              <strong>{t("command")}</strong>
              <span>{items.length}/{MAX_QUICK_COMMANDS}</span>
              <button type="button" title={t("add-quick-command")} aria-label={t("add-quick-command")} onClick={addCommand} disabled={items.length >= MAX_QUICK_COMMANDS}><Plus size={14} /></button>
            </header>
            <div role="listbox" aria-label={t("quick-command-list")}>
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
                  <span>{item.label || t("unnamed-command")}</span>
                </button>
              ))}
              {!items.length ? <div className="quick-command-list-empty">{t("no-quick-commands")}</div> : null}
            </div>
          </aside>
          <section className="quick-command-editor">
            {selected ? (
              <>
                <label>
                  <span>{t("name")}</span>
                  <input aria-label={t("quick-command-name")} value={selected.label} onChange={(event) => updateSelected({ label: limitQuickCommandLabelInput(event.target.value) })} />
                </label>
                <label className="quick-command-text-field">
                  <span>{t("command")}</span>
                  <textarea aria-label={t("quick-command-content")} value={selected.command} spellCheck={false} onChange={(event) => updateSelected({ command: normalizeQuickCommandText(event.target.value) })} />
                </label>
                <label className="quick-command-enter-toggle">
                  <input type="checkbox" checked={selected.appendEnter} onChange={(event) => updateSelected({ appendEnter: event.target.checked })} />
                  <span>{t("append-enter-and-execute")}</span>
                </label>
                <div className="quick-command-editor-actions">
                  <button type="button" title={t("move-up")} aria-label={t("move-quick-command-up")} disabled={selectedIndex <= 0} onClick={() => moveSelected(-1)}><ArrowUp size={14} /></button>
                  <button type="button" title={t("move-down")} aria-label={t("move-quick-command-down")} disabled={selectedIndex < 0 || selectedIndex >= items.length - 1} onClick={() => moveSelected(1)}><ArrowDown size={14} /></button>
                  <span />
                  <button type="button" className="danger" title={t("delete")} aria-label={t("delete-quick-command")} onClick={removeSelected}><Trash2 size={14} /></button>
                </div>
              </>
            ) : (
              <div className="quick-command-editor-empty">
                <button type="button" onClick={addCommand}><Plus size={14} /><span>{t("add-command")}</span></button>
              </div>
            )}
          </section>
        </section>
        <footer className="utility-actions quick-command-dialog-actions">
          {error ? <span role="alert">{localizeDiagnostic(error)}</span> : <span />}
          <button type="button" onClick={closeDialog}>{t("cancel")}</button>
          <button type="submit">{t("save")}</button>
        </footer>
      </form>
    </div>
  );
}
