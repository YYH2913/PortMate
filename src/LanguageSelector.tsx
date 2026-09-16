import { Languages } from "lucide-react";
import { languages, setLanguagePreference, t, useLocale } from "./i18n";
import type { LanguagePreference } from "./i18n";

export default function LanguageSelector({ compact = false }: { compact?: boolean }) {
  const { preference } = useLocale();
  return (
    <label className={`language-selector${compact ? " compact" : ""}`}>
      <Languages size={15} aria-hidden="true" />
      {!compact ? <span>{t("interface-language")}</span> : null}
      <select aria-label={t("interface-language")} value={preference} onChange={event => setLanguagePreference(event.target.value as LanguagePreference)}>
        <option value="system">{t("system-language")}</option>
        {languages.map(language => <option key={language.id} value={language.id} lang={language.id}>{language.name}</option>)}
      </select>
    </label>
  );
}
