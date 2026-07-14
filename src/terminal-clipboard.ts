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
