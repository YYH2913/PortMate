import { t } from "./i18n";
export function exactNonBlankPathInput(value: string | null): string | null {
  if (value === null || !value.trim()) return null;
  return value;
}

/** Parse the complete octal value; parseInt alone accepts 644junk and 648. */
export function parseFilePermissionMode(value: string): number {
  const text = value.trim();
  if (!/^(?:0o)?[0-7]{3,4}$/i.test(text)) {
    throw new Error(t("permissions-must-be-3-4-octal-digits-e-g"));
  }
  return Number.parseInt(text.replace(/^0o/i, ""), 8);
}
