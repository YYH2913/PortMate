import { copyFileSync, lstatSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
export const easyTierNotices = ["EasyTier-NOTICE.txt", "EasyTier-LGPL-3.0.txt", "GPL-3.0.txt", "Wintun-LICENSE.txt", "WinDivert-LICENSE.txt", "portable-pty-MIT.txt"];

export function copyEasyTierNotices(directory) {
  mkdirSync(directory, { recursive: true });
  for (const name of easyTierNotices) copyFileSync(join(root, "licenses", name), join(directory, name));
  return verifyEasyTierNotices(directory);
}

/** Shared bundled-component resources also retain PortMate’s native PTY license. */
export function verifyEasyTierNotices(directory) {
  for (const name of easyTierNotices) {
    const path = join(directory, name);
    const metadata = lstatSync(path);
    if (!metadata.isFile() || metadata.isSymbolicLink()
      || !readFileSync(path).equals(readFileSync(join(root, "licenses", name)))) {
      throw new Error(`Missing, linked or modified upstream notice: ${name}`);
    }
  }
  return { directory, files: [...easyTierNotices] };
}
