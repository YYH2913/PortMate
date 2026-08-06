import {
  mkdirSync,
  mkdtempSync,
  readlinkSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { APPIMAGE_ROOT_LINKS, normalizeAppImageRootLinks } from "./appimage-tree.mjs";

const roots = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe("normalizeAppImageRootLinks", () => {
  it("replaces every generated root link with the portable canonical target", () => {
    const root = fixture();
    for (const name of Object.keys(APPIMAGE_ROOT_LINKS)) {
      symlinkSync("usr/share/icons/hicolor/128x128/apps/portmate.png", join(root, name));
    }

    normalizeAppImageRootLinks(root);

    for (const [name, target] of Object.entries(APPIMAGE_ROOT_LINKS)) {
      expect(readlinkSync(join(root, name))).toBe(target);
    }
  });

  it("rejects canonical links whose targets are missing", () => {
    const root = fixture();
    for (const [name, target] of Object.entries(APPIMAGE_ROOT_LINKS)) {
      symlinkSync(target, join(root, name));
    }
    rmSync(join(root, APPIMAGE_ROOT_LINKS["portmate.png"]));

    expect(() => normalizeAppImageRootLinks(root)).toThrow(/portmate\.png.*regular file/);
  });
});

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "portmate-appimage-tree-"));
  roots.push(root);
  for (const target of new Set([
    ...Object.values(APPIMAGE_ROOT_LINKS),
    "usr/share/icons/hicolor/128x128/apps/portmate.png",
  ])) {
    const path = join(root, target);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, target);
  }
  return root;
}
