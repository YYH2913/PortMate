export type TerminalWebLinkWindow = { opener: unknown };

export type TerminalWebLinkOpener = (
  url: string,
  target: string,
  features: string,
) => TerminalWebLinkWindow | null;

export function normalizeTerminalWebLink(value: string): string | null {
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
  } catch {
    return null;
  }
}

export function openTerminalWebLink(
  event: Pick<MouseEvent, "preventDefault" | "stopImmediatePropagation">,
  value: string,
  openWindow: TerminalWebLinkOpener = (url, target, features) => window.open(url, target, features),
): boolean {
  const url = normalizeTerminalWebLink(value);
  if (!url) return false;
  event.preventDefault();
  event.stopImmediatePropagation();
  try {
    const popup = openWindow(url, "_blank", "noopener,noreferrer");
    if (popup) popup.opener = null;
    return true;
  } catch {
    return false;
  }
}
