import { describe, expect, it } from "vitest";
import { buildDesktopEnvironment } from "./desktop-clean-environment.mjs";
import { isProjectViteCommand, parseWindowsListeningPids } from "./desktop-clean-process.mjs";

describe("buildDesktopEnvironment", () => {
  it("keeps only desktop runtime variables", () => {
    const env = buildDesktopEnvironment({
      HOME: "/home/alice",
      PATH: "/usr/bin",
      DISPLAY: ":0",
      SSH_AUTH_SOCK: "/run/user/1000/ssh-agent.socket",
      TMPDIR: "/tmp/alice",
      LD_PRELOAD: "/snap/injected.so",
      GTK_PATH: "/snap/gtk",
      GIO_MODULE_DIR: "/snap/gio",
    });

    expect(env).toMatchObject({
      HOME: "/home/alice",
      PATH: "/usr/bin",
      DISPLAY: ":0",
      SSH_AUTH_SOCK: "/run/user/1000/ssh-agent.socket",
      TMPDIR: "/tmp/alice",
    });
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

  it("preserves the Windows process environment without injecting Unix data paths", () => {
    const env = buildDesktopEnvironment({
      Path: "C:\\Windows\\System32",
      SystemRoot: "C:\\Windows",
      USERPROFILE: "C:\\Users\\alice",
      APPDATA: "C:\\Users\\alice\\AppData\\Roaming",
      LOCALAPPDATA: "C:\\Users\\alice\\AppData\\Local",
      TEMP: "C:\\Users\\alice\\AppData\\Local\\Temp",
      CARGO_HOME: "C:\\Users\\alice\\.cargo",
      PORTMATE_SIGNING_IDENTITY: "test-signing-identity",
      LD_PRELOAD: "/snap/injected.so",
      GTK_PATH: "/snap/gtk",
    }, "win32");

    expect(env).toMatchObject({
      Path: "C:\\Windows\\System32",
      SystemRoot: "C:\\Windows",
      USERPROFILE: "C:\\Users\\alice",
      APPDATA: "C:\\Users\\alice\\AppData\\Roaming",
      LOCALAPPDATA: "C:\\Users\\alice\\AppData\\Local",
      TEMP: "C:\\Users\\alice\\AppData\\Local\\Temp",
      CARGO_HOME: "C:\\Users\\alice\\.cargo",
      PORTMATE_SIGNING_IDENTITY: "test-signing-identity",
    });
    expect(env).not.toHaveProperty("XDG_DATA_DIRS");
    expect(env).not.toHaveProperty("LD_PRELOAD");
    expect(env).not.toHaveProperty("GTK_PATH");
  });
});

describe("desktop clean process ownership", () => {
  it("extracts exact Windows TCP listeners and deduplicates their PIDs", () => {
    const output = `
      TCP    127.0.0.1:1420       0.0.0.0:0       LISTENING       4210
      TCP    [::1]:1420           [::]:0          LISTENING       4211
      TCP    127.0.0.1:1420       127.0.0.1:52000 ESTABLISHED     9999
      TCP    127.0.0.1:11420      0.0.0.0:0       LISTENING       4212
      TCP    0.0.0.0:1420         0.0.0.0:0       LISTENING       4210
    `;

    expect(parseWindowsListeningPids(output, 1420)).toEqual([4210, 4211]);
  });

  it("matches only Vite commands rooted in the current Windows checkout", () => {
    const projectRoot = "C:\\Users\\Alice\\PortMate";

    expect(isProjectViteCommand(
      '"C:\\Program Files\\nodejs\\node.exe" "C:\\Users\\Alice\\PortMate\\node_modules\\vite\\bin\\vite.js" --port 1420',
      projectRoot,
      "win32",
    )).toBe(true);
    expect(isProjectViteCommand(
      'node.exe C:\\Users\\Alice\\Other\\node_modules\\vite\\bin\\vite.js --port 1420',
      projectRoot,
      "win32",
    )).toBe(false);
    expect(isProjectViteCommand("vite --port 1420", projectRoot, "win32")).toBe(false);
  });
});
