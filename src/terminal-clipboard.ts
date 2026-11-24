import type { IClipboardProvider } from "@xterm/addon-clipboard";

type ClipboardWriter = Pick<Clipboard, "writeText">;

export function createWriteOnlyClipboardProvider(clipboard: ClipboardWriter | undefined): IClipboardProvider {
  return {
    readText: () => "",
    writeText: (_selection, text) => {
      if (!clipboard) return;
      try {
        return Promise.resolve(clipboard.writeText(text)).catch(() => {});
      } catch {
        // WebViews may reject clipboard access before returning a Promise.
      }
    },
  };
}

// Context menus live outside xterm. Delegate paste to the exact mounted view
// so bracketed paste, private input and connection fencing match middle-click.
type ViewPaste = (isCurrent: () => boolean) => Promise<boolean>;
const pasteTargets = new Map<string, { sessionId: string; paste: ViewPaste }>();

export function registerTerminalPasteTarget(sessionId: string, viewId: string, paste: ViewPaste) {
  const entry = { sessionId, paste };
  if (viewId) pasteTargets.set(viewId, entry);
  return () => { if (pasteTargets.get(viewId) === entry) pasteTargets.delete(viewId); };
}

export function pasteClipboardIntoTerminalView(sessionId: string, viewId: string, isCurrent: () => boolean) {
  const target = pasteTargets.get(viewId);
  return target?.sessionId === sessionId ? target.paste(isCurrent) : Promise.resolve(false);
}
