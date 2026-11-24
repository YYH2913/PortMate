import { useLayoutEffect, useId, useRef, useState, useSyncExternalStore } from "react";
import type { FormEvent } from "react";
import { X } from "lucide-react";
import {
  type AppDialogRequest,
  getActiveAppDialog,
  registerAppDialogHost,
  resolveActiveAppDialog,
  subscribeAppDialogs,
} from "./app-dialog";
import { t, useLocale } from "./i18n";

export default function AppDialogHost() {
  const active = useSyncExternalStore(subscribeAppDialogs, getActiveAppDialog, () => null);
  useLayoutEffect(() => registerAppDialogHost(), []);
  return active ? <AppDialog key={active.id} active={active} /> : null;
}

function AppDialog({ active }: { active: AppDialogRequest }) {
  useLocale();
  const [value, setValue] = useState(active.kind === "prompt" ? active.options.defaultValue ?? "" : "");
  const inputRef = useRef<HTMLInputElement>(null);
  const cancelButtonRef = useRef<HTMLButtonElement>(null);
  const confirmButtonRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const messageId = useId();

  useLayoutEffect(() => {
    if (active.kind === "prompt") {
      inputRef.current?.focus({ preventScroll: true });
      inputRef.current?.select();
    } else {
      (active.options.danger ? cancelButtonRef : confirmButtonRef).current?.focus({ preventScroll: true });
    }
  }, [active]);

  const title = active.options.title || (active.kind === "confirm" ? t("confirm") : active.options.label || t("confirm"));
  const cancelLabel = active.options.cancelLabel || t("cancel");
  const confirmLabel = active.options.confirmLabel || t("ok");

  function cancel() {
    resolveActiveAppDialog(active.id, active.kind === "confirm" ? false : null);
  }

  function submit(event?: FormEvent) {
    event?.preventDefault();
    resolveActiveAppDialog(active.id, active.kind === "confirm" ? true : value);
  }

  return (
    <div
      className="dialog-backdrop utility-backdrop app-dialog-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) cancel();
      }}
    >
      <section
        className="wind-dialog app-dialog"
        data-app-dialog={active.kind}
        role={active.kind === "confirm" ? "alertdialog" : "dialog"}
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={active.options.message ? messageId : undefined}
        onKeyDown={(event) => {
          if (event.key === "Escape" && !event.defaultPrevented && !event.nativeEvent.isComposing) {
            event.preventDefault();
            event.stopPropagation();
            cancel();
          }
        }}
      >
        <header className="dialog-title">
          <span className="app-icon" />
          <strong id={titleId}>{title}</strong>
          <button type="button" title={t("close")} aria-label={t("close")} onClick={cancel}>
            <X size={20} />
          </button>
        </header>
        <form className="app-dialog-body" onSubmit={submit}>
          {active.options.message ? (
            <p id={messageId} className="app-dialog-message">{active.options.message}</p>
          ) : null}
          {active.kind === "prompt" ? (
            <label className="app-dialog-field">
              {active.options.label ? <span>{active.options.label}</span> : null}
              <input
                ref={inputRef}
                type="text"
                aria-label={active.options.label || title}
                placeholder={active.options.placeholder}
                value={value}
                onChange={(event) => setValue(event.target.value)}
              />
            </label>
          ) : null}
          <footer className="utility-actions">
            <button ref={cancelButtonRef} type="button" onClick={cancel}>{cancelLabel}</button>
            <button
              ref={confirmButtonRef}
              type="submit"
              className={active.kind === "confirm" && active.options.danger ? "danger-btn" : "primary-btn"}
            >
              {confirmLabel}
            </button>
          </footer>
        </form>
      </section>
    </div>
  );
}
