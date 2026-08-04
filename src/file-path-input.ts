export function exactNonBlankPathInput(value: string | null): string | null {
  if (value === null || !value.trim()) return null;
  return value;
}
