import { Languages } from "lucide-react";
import { languages, setLanguagePreference, t, useLocale } from "./i18n";
import type { LanguagePreference } from "./i18n";

export default function LanguageSelector({ compact = false, value, onChange = setLanguagePreference }: { compact?: boolean; value?: LanguagePreference; onChange?: (value: LanguagePreference) => void }) {
  const { preference } = useLocale();
  return (
    <label data-settings-key="interface-language" className={`language-selector${compact ? " compact" : ""}`}>
      <Languages size={15} aria-hidden="true" />
      {!compact ? <span>{t("interface-language")}</span> : null}
      <select aria-label={t("interface-language")} value={value ?? preference} onChange={event => onChange(event.target.value as LanguagePreference)}>
        <option value="system">{t("system-language")}</option>
        {languages.map(language => <option key={language.id} value={language.id} lang={language.id}>{language.name}</option>)}
      </select>
    </label>
  );
}
