import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { t } from "./i18n";
import { settingsLabel } from "./settings-navigation";
import type { SettingsHit } from "./settings-navigation";

/** Results stay ordinary tab stops; arrow keys are a shortcut, not a requirement. */
export function navigateSettingsResults(event: KeyboardEvent<HTMLElement>) {
  if (event.nativeEvent.isComposing || !["ArrowDown", "ArrowUp"].includes(event.key)) return;
  const buttons = Array.from(event.currentTarget.closest(".wind-dialog")?.querySelectorAll<HTMLButtonElement>(".settings-results button:not(:disabled)") ?? []);
  if (!buttons.length) return;
  const index = buttons.findIndex(button => button === event.target);
  const next = event.key === "ArrowDown" ? Math.min(index + 1, buttons.length - 1) : index < 0 ? buttons.length - 1 : Math.max(0, index - 1);
  event.preventDefault();
  buttons[next].focus();
}

export function SettingsResults({ hits, onSelect, disabled }: { hits: SettingsHit[]; onSelect: (hit: SettingsHit) => void; disabled?: boolean }) {
  return <nav onKeyDown={navigateSettingsResults} className="settings-results" aria-label={t("search-settings")}>
    {hits.length ? hits.map(hit => <button key={`${hit.page}/${hit.field ?? ""}`} type="button" disabled={disabled} onClick={() => onSelect(hit)}>
      {hit.field && <small>{settingsLabel(hit.page)}</small>}
      <span>{settingsLabel(hit.field ?? hit.page)}</span>
    </button>) : <p className="settings-search-empty" role="status">{t("no-matching-settings")}</p>}
  </nav>;
}

/** Wait until React has mounted the chosen page. Never activate a switch/button while navigating. */
export function useSettingsTarget() {
  const panel = useRef<HTMLElement>(null);
  const [target, focusHit] = useState<SettingsHit | null>(null);
  useLayoutEffect(() => {
    if (!target || !panel.current) return;
    const field = target.field && panel.current.querySelector<HTMLElement>(`[data-settings-key="${CSS.escape(target.field)}"]`);
    const control = field && (field.querySelector<HTMLElement>("input:not([disabled]),select:not([disabled]),textarea:not([disabled]),button:not([disabled])") ?? field);
    const element = control || panel.current;
    if (element === field && !element.matches("button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled])")) element.tabIndex = -1;
    element.focus({ preventScroll: true });
    if (field) field.scrollIntoView({ block: "nearest" });
    else panel.current.scrollTop = 0;
  }, [target]);
  return { panel, focusHit };
}

export function DiscardSettingsPrompt({ onKeep, onDiscard }: { onKeep: () => void; onDiscard: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const title = useId();
  useEffect(() => {
    const dialog = ref.current!;
    dialog.showModal();
    return () => dialog.close();
  }, []);
  return <dialog ref={ref} className="settings-discard" aria-labelledby={title}
    onKeyDown={event => event.stopPropagation()} onCancel={event => { event.preventDefault(); onKeep(); }}>
    <h2 id={title}>{t("discard-settings-question")}</h2>
    <div><button type="button" autoFocus onClick={onKeep}>{t("keep-editing-settings")}</button>
      <button type="button" onClick={onDiscard}>{t("discard-changes")}</button></div>
  </dialog>;
}
