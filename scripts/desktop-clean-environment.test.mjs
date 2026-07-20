import { describe, expect, it } from "vitest";
import { buildDesktopEnvironment } from "./desktop-clean-environment.mjs";

describe("buildDesktopEnvironment", () => {
  it("keeps only desktop runtime variables", () => {
    const env = buildDesktopEnvironment({
      HOME: "/home/alice",
      PATH: "/usr/bin",
      DISPLAY: ":0",
      LD_PRELOAD: "/snap/injected.so",
      GTK_PATH: "/snap/gtk",
      GIO_MODULE_DIR: "/snap/gio",
    });

    expect(env).toMatchObject({ HOME: "/home/alice", PATH: "/usr/bin", DISPLAY: ":0" });
    expect(env).not.toHaveProperty("LD_PRELOAD");
    expect(env).not.toHaveProperty("GTK_PATH");
    expect(env).not.toHaveProperty("GIO_MODULE_DIR");
  });

  it("restores the pre-snap XDG data path when available", () => {
    const env = buildDesktopEnvironment({
      HOME: "/home/alice",
      XDG_DATA_DIRS_VSCODE_SNAP_ORIG: "/opt/share:/usr/share",
    });

    expect(env.XDG_DATA_DIRS).toBe("/opt/share:/usr/share");
    expect(env).not.toHaveProperty("XDG_DATA_DIRS_VSCODE_SNAP_ORIG");
  });

  it("derives the flatpak user path from HOME without a fixed username", () => {
    const env = buildDesktopEnvironment({ HOME: "/srv/users/bob" });

    expect(env.XDG_DATA_DIRS).toBe(
      "/srv/users/bob/.local/share/flatpak/exports/share:/var/lib/flatpak/exports/share:/usr/local/share:/usr/share:/var/lib/snapd/desktop",
    );
    expect(env.XDG_DATA_DIRS).not.toContain("/home/yyh");
  });

  it("uses only system paths when HOME is unavailable", () => {
    const env = buildDesktopEnvironment({});

    expect(env.XDG_DATA_DIRS).toBe(
      "/var/lib/flatpak/exports/share:/usr/local/share:/usr/share:/var/lib/snapd/desktop",
    );
  });
});
