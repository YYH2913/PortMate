import {
  lstatSync,
  readlinkSync,
  statSync,
  symlinkSync,
  unlinkSync,
} from "node:fs";
import { join } from "node:path";

export const APPIMAGE_ROOT_LINKS = Object.freeze({
  ".DirIcon": "PortMate.png",
  "PortMate.desktop": "usr/share/applications/PortMate.desktop",
  "portmate.png": "usr/share/icons/hicolor/256x256@2/apps/portmate.png",
});

export function normalizeAppImageRootLinks(appDir) {
  for (const [name, target] of Object.entries(APPIMAGE_ROOT_LINKS)) {
    replaceSymlink(join(appDir, name), target);
  }
}

function replaceSymlink(path, target) {
  const metadata = lstatSync(path);
  if (!metadata.isSymbolicLink()) {
    throw new Error(`Expected ${path} to be a symbolic link`);
  }
  if (readlinkSync(path) !== target) {
    unlinkSync(path);
    symlinkSync(target, path);
  }
  let targetMetadata;
  try {
    targetMetadata = statSync(path);
  } catch {
    throw new Error(`Expected ${path} -> ${target} to resolve to a regular file`);
  }
  if (!targetMetadata.isFile()) {
    throw new Error(`Expected ${path} -> ${target} to resolve to a regular file`);
  }
}
