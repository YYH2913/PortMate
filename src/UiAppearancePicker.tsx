import type { KeyboardEvent } from "react";
import { Check, Columns2, LayoutGrid, SlidersHorizontal } from "lucide-react";
import { t, useLocale } from "./i18n";
import {
  uiAccentColors,
  uiLayoutStyles,
  uiThemePresets,
  type UiAccentColor,
  type UiAppearanceState,
  type UiLayoutStyle,
  type UiThemePreset,
} from "./ui-theme";

const THEME_META: Record<UiThemePreset, {
  labelKey: string;
  descKey: string;
  swatches: readonly [string, string, string];
}> = {
  "studio-light": {
    labelKey: "ui-theme-studio-light",
    descKey: "ui-theme-studio-light-desc",
    swatches: ["#eef1f5", "#ffffff", "#2563eb"],
  },
  "carbon-islands": {
    labelKey: "ui-theme-carbon-islands",
    descKey: "ui-theme-carbon-islands-desc",
    swatches: ["#0a0d13", "#181f2e", "#38bdf8"],
  },
  "obsidian-linear": {
    labelKey: "ui-theme-obsidian-linear",
    descKey: "ui-theme-obsidian-linear-desc",
    swatches: ["#09090b", "#18181c", "#818cf8"],
  },
};

const LAYOUT_META: Record<UiLayoutStyle, { labelKey: string }> = {
  islands: { labelKey: "ui-layout-islands" },
  docked: { labelKey: "ui-layout-docked" },
};

const ACCENT_META: Record<UiAccentColor, { labelKey: string; swatch: string }> = {
  default: { labelKey: "ui-accent-default", swatch: "conic-gradient(from 45deg, #2563eb 0 120deg, #38bdf8 120deg 240deg, #818cf8 240deg 360deg)" },
  indigo: { labelKey: "ui-accent-indigo", swatch: "#6366f1" },
  emerald: { labelKey: "ui-accent-emerald", swatch: "#14b8a6" },
  amber: { labelKey: "ui-accent-amber", swatch: "#f59e0b" },
  rose: { labelKey: "ui-accent-rose", swatch: "#f43f5e" },
};

function handleChoiceKeys(event: KeyboardEvent<HTMLDivElement>) {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const rtl = getComputedStyle(event.currentTarget).direction === "rtl";
  const previous = event.key === "ArrowUp" || event.key === (rtl ? "ArrowRight" : "ArrowLeft");
  const next = event.key === "ArrowDown" || event.key === (rtl ? "ArrowLeft" : "ArrowRight");
  if (!previous && !next) return;
  const choices = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button[role="radio"]'));
  const current = choices.findIndex(choice => choice === event.target);
  if (current < 0) return;
  event.preventDefault();
  const choice = choices[(current + (next ? 1 : -1) + choices.length) % choices.length];
  choice.focus();
  choice.click();
}

export default function UiAppearancePicker({
  value,
  onChange,
  compact = false,
  onOpenSettings,
}: {
  value: UiAppearanceState;
  onChange: (patch: Partial<UiAppearanceState>) => void;
  compact?: boolean;
  onOpenSettings?: () => void;
}) {
  useLocale();

  return (
    <div className={compact ? "ui-appearance-picker compact" : "ui-appearance-picker"}>
      <div className="ui-appearance-group" data-settings-key="interface-theme">
        {compact ? <span className="ui-appearance-label">{t("interface-theme")}</span> : null}
        <div className="ui-theme-grid" role="radiogroup" aria-label={t("interface-theme")} onKeyDown={handleChoiceKeys}>
          {uiThemePresets.map((preset) => {
            const meta = THEME_META[preset];
            const selected = value.theme === preset;
            return (
              <button
                key={preset}
                type="button"
                role="radio"
                aria-checked={selected}
                tabIndex={selected ? 0 : -1}
                className={selected ? "ui-theme-card active" : "ui-theme-card"}
                data-theme-choice={preset}
                onClick={() => onChange({ theme: preset })}
              >
                <span className="ui-theme-card-head">
                  <strong>{t(meta.labelKey)}</strong>
                  <span className="ui-theme-swatches" aria-hidden="true">
                    {meta.swatches.map((color, index) => (
                      <i key={index} style={{ background: color }} />
                    ))}
                  </span>
                </span>
                {!compact ? <small>{t(meta.descKey)}</small> : null}
              </button>
            );
          })}
        </div>
      </div>

      <div className="ui-appearance-group" data-settings-key="interface-layout">
        <span className="ui-appearance-label">{t("interface-layout")}</span>
        <div className="ui-layout-grid" role="radiogroup" aria-label={t("interface-layout")} onKeyDown={handleChoiceKeys}>
          {uiLayoutStyles.map((layout) => {
            const selected = value.layout === layout;
            const Icon = layout === "islands" ? LayoutGrid : Columns2;
            return (
              <button
                key={layout}
                type="button"
                role="radio"
                aria-checked={selected}
                tabIndex={selected ? 0 : -1}
                className={selected ? "ui-layout-pill active" : "ui-layout-pill"}
                data-layout-choice={layout}
                onClick={() => onChange({ layout })}
              >
                <Icon size={13} aria-hidden="true" />
                <span>{t(LAYOUT_META[layout].labelKey)}</span>
              </button>
            );
          })}
        </div>
      </div>

      <div className="ui-appearance-group" data-settings-key="accent-color">
        <span className="ui-appearance-label">{t("accent-color")}</span>
        <div className="ui-accent-row" role="radiogroup" aria-label={t("accent-color")} onKeyDown={handleChoiceKeys}>
          {uiAccentColors.map((accent) => {
            const meta = ACCENT_META[accent];
            const selected = value.accent === accent;
            return (
              <button
                key={accent}
                type="button"
                role="radio"
                aria-checked={selected}
                tabIndex={selected ? 0 : -1}
                className={selected ? "ui-accent-pill active" : "ui-accent-pill"}
                data-accent-choice={accent}
                title={t(meta.labelKey)}
                onClick={() => onChange({ accent })}
              >
                <span className="ui-accent-swatch" style={{ background: meta.swatch }} aria-hidden="true">
                  {selected ? <Check size={10} strokeWidth={3} /> : null}
                </span>
                <span>{t(meta.labelKey)}</span>
              </button>
            );
          })}
        </div>
      </div>

      {compact && onOpenSettings ? (
        <div className="ui-appearance-footer">
          <button type="button" className="ui-appearance-more" onClick={onOpenSettings}>
            <SlidersHorizontal size={12} aria-hidden="true" />
            <span>{t("more-appearance-settings")}</span>
          </button>
        </div>
      ) : null}
    </div>
  );
}
