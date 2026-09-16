import { t } from "./i18n";
export function exactNonBlankPathInput(value: string | null): string | null {
  if (value === null || !value.trim()) return null;
  return value;
}

/** Navigate without turning a Windows drive root into a drive-relative path. */
export function fileParentPath(path: string, remote: boolean): string {
  if (!remote && /^~[\\/]*$/.test(path)) return "~";
  const syntax = filePathSyntax(path, remote);
  const trimmed = trimFilePathSeparators(path, syntax.rootLength, syntax.windows);
  if (trimmed.length <= syntax.rootLength) return trimmed || ".";
  const lastComponent = trimmed.split(syntax.windows ? /[\\/]/ : /\//).at(-1);
  // Listings may keep relative addresses. Let the filesystem resolve link/..
  // rather than cancelling it lexically and navigating back into the link.
  if (lastComponent === "." || lastComponent === "..") {
    return trimmed === "." ? ".." : `${trimmed}${syntax.separator}..`;
  }
  for (let index = trimmed.length - 1; index >= syntax.rootLength; index -= 1) {
    if (isFilePathSeparator(trimmed[index], syntax.windows)) return trimmed.slice(0, index);
  }
  return syntax.rootLength ? trimmed.slice(0, syntax.rootLength) : ".";
}

/** Join display paths only; native validation still owns filesystem safety. */
export function fileJoinPath(base: string, name: string, remote: boolean): string {
  if (!base) return name;
  const syntax = filePathSyntax(base, remote);
  const trimmed = trimFilePathSeparators(base, syntax.rootLength, syntax.windows);
  // A drive-relative input must remain drive-relative, not silently gain a root.
  if (!remote && /^(?:\\\\\?\\)?[a-z]:$/i.test(trimmed)) return `${trimmed}${name}`;
  return isFilePathSeparator(trimmed.at(-1), syntax.windows)
    ? `${trimmed}${name}` : `${trimmed}${syntax.separator}${name}`;
}

function filePathSyntax(path: string, remote: boolean) {
  if (remote) return { windows: false, separator: "/", rootLength: path.startsWith("/") ? 1 : 0 };
  const drive = /^(?:\\\\\?\\)?[a-z]:([\\/])/i.exec(path);
  const unc = /^(?:\\\\\?\\UNC\\|\\\\|\/\/)[^\\/]+[\\/][^\\/]+(?:[\\/]|$)/i.exec(path);
  const windows = Boolean(drive || unc || /^[a-z]:/i.test(path)
    || path.startsWith("\\") || (!path.includes("/") && path.includes("\\")));
  const separator = windows && (drive?.[1] === "\\" || (drive === null && path.includes("\\"))) ? "\\" : "/";
  const rootLength = drive?.[0].length ?? unc?.[0].length
    ?? (isFilePathSeparator(path[0], windows) ? 1 : 0);
  return { windows, separator, rootLength };
}

function isFilePathSeparator(character: string | undefined, windows: boolean): boolean {
  return character === "/" || (windows && character === "\\");
}

function trimFilePathSeparators(path: string, rootLength: number, windows: boolean): string {
  let end = path.length;
  while (end > rootLength && isFilePathSeparator(path[end - 1], windows)) end -= 1;
  return path.slice(0, end);
}

/** Parse the complete octal value; parseInt alone accepts 644junk and 648. */
export function parseFilePermissionMode(value: string): number {
  const text = value.trim();
  if (!/^(?:0o)?[0-7]{3,4}$/i.test(text)) {
    throw new Error(t("permissions-must-be-3-4-octal-digits-e-g"));
  }
  return Number.parseInt(text.replace(/^0o/i, ""), 8);
}
