export function exactNonBlankPathInput(value: string | null): string | null {
  if (value === null || !value.trim()) return null;
  return value;
}

/** Parse the complete octal value; parseInt alone accepts 644junk and 648. */
export function parseFilePermissionMode(value: string): number {
  const text = value.trim();
  if (!/^(?:0o)?[0-7]{3,4}$/i.test(text)) {
    throw new Error("权限必须是 3–4 位八进制数，例如 644、0755 或 0o755。");
  }
  return Number.parseInt(text.replace(/^0o/i, ""), 8);
}
