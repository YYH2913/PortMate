import { describe, expect, it } from "vitest";
import { buildDesktopEnvironment } from "./desktop-clean-environment.mjs";
import {
  isProjectViteCommand,
  parseWindowsListeningPids,
  signalProcessIfRunning,
  waitForStablePortAvailability,
} from "./desktop-clean-process.mjs";

describe("buildDesktopEnvironment", () => {
  it("removes injected desktop variables without dropping build configuration", () => {
    const env = buildDesktopEnvironment({
      HOME: "/home/alice",
      PATH: "/usr/bin",
      DISPLAY: ":0",
      SSH_AUTH_SOCK: "/run/user/1000/ssh-agent.socket",
      TMPDIR: "/tmp/alice",
      CARGO_HOME: "/opt/rust/cargo",
      RUSTUP_HOME: "/opt/rust/rustup",
      RUSTUP_TOOLCHAIN: "stable-custom",
      CC: "clang-19",
      PKG_CONFIG_PATH: "/opt/gtk/lib/pkgconfig",
      RUSTFLAGS: "-C target-cpu=native",
      PORTMATE_STORE_PATH: "/tmp/portmate.sqlite3",
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
      CARGO_HOME: "/opt/rust/cargo",
      RUSTUP_HOME: "/opt/rust/rustup",
      RUSTUP_TOOLCHAIN: "stable-custom",
      CC: "clang-19",
      PKG_CONFIG_PATH: "/opt/gtk/lib/pkgconfig",
      RUSTFLAGS: "-C target-cpu=native",
      PORTMATE_STORE_PATH: "/tmp/portmate.sqlite3",
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

  it("preserves a custom XDG data path outside a Snap environment", () => {
    const env = buildDesktopEnvironment({
      HOME: "/home/alice",
      XDG_DATA_DIRS: "/opt/company/share:/usr/share",
    });

    expect(env.XDG_DATA_DIRS).toBe("/opt/company/share:/usr/share");
  });

  it("replaces Snap-injected XDG data paths when no original value is available", () => {
    const env = buildDesktopEnvironment({
      HOME: "/home/alice",
      SNAP: "/snap/code/200",
      SNAP_NAME: "code",
      XDG_DATA_DIRS: "/snap/code/200/usr/share:/usr/share",
    });

    expect(env.XDG_DATA_DIRS).toBe(
      "/home/alice/.local/share/flatpak/exports/share:/var/lib/flatpak/exports/share:/usr/local/share:/usr/share:/var/lib/snapd/desktop",
    );
    expect(env).not.toHaveProperty("SNAP");
    expect(env).not.toHaveProperty("SNAP_NAME");
  });

  it("preserves macOS SDK and toolchain configuration", () => {
    const env = buildDesktopEnvironment({
      HOME: "/Users/alice",
      PATH: "/usr/bin:/opt/homebrew/bin",
      DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer",
      SDKROOT: "/Applications/Xcode.app/SDKs/MacOSX.sdk",
      MACOSX_DEPLOYMENT_TARGET: "13.0",
      CARGO_TARGET_DIR: "/tmp/portmate-target",
    }, "darwin");

    expect(env).toMatchObject({
      DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer",
      SDKROOT: "/Applications/Xcode.app/SDKs/MacOSX.sdk",
      MACOSX_DEPLOYMENT_TARGET: "13.0",
      CARGO_TARGET_DIR: "/tmp/portmate-target",
    });
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

  it("treats an already-exited listener as a successful no-op", () => {
    const missing = Object.assign(new Error("process not found"), { code: "ESRCH" });
    expect(signalProcessIfRunning(4210, "SIGTERM", () => { throw missing; })).toBe(false);
  });

  it("does not hide permission failures while signalling a listener", () => {
    const denied = Object.assign(new Error("permission denied"), { code: "EPERM" });
    expect(() => signalProcessIfRunning(4210, "SIGTERM", () => { throw denied; })).toThrow(denied);
  });

  it("reports when a listener received the requested signal", () => {
    const calls = [];
    expect(signalProcessIfRunning(4210, "SIGKILL", (pid, signal) => calls.push([pid, signal]))).toBe(true);
    expect(calls).toEqual([[4210, "SIGKILL"]]);
  });

  it("requires the dev port to remain available before restarting Vite", async () => {
    let clock = 0;
    const probes = [false, true, true, false, true, true, true];
    const result = await waitForStablePortAvailability(
      async () => probes.shift() ?? true,
      {
        timeoutMs: 1_000,
        stableMs: 200,
        intervalMs: 100,
        now: () => clock,
        sleep: async (durationMs) => {
          clock += durationMs;
        },
      },
    );

    expect(result).toBe(true);
    expect(clock).toBe(600);
  });

  it("times out when the dev port keeps flapping", async () => {
    let clock = 0;
    let available = true;
    const result = await waitForStablePortAvailability(
      async () => {
        available = !available;
        return available;
      },
      {
        timeoutMs: 500,
        stableMs: 200,
        intervalMs: 100,
        now: () => clock,
        sleep: async (durationMs) => {
          clock += durationMs;
        },
      },
    );

    expect(result).toBe(false);
    expect(clock).toBe(500);
  });
});
