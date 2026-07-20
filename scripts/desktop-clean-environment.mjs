import { resolve } from "node:path";

const inheritedDesktopVariables = new Set([
  "HOME",
  "USER",
  "LOGNAME",
  "SHELL",
  "PATH",
  "TMPDIR",
  "DISPLAY",
  "WAYLAND_DISPLAY",
  "XDG_RUNTIME_DIR",
  "DBUS_SESSION_BUS_ADDRESS",
  "XDG_CURRENT_DESKTOP",
  "XDG_SESSION_TYPE",
  "TERM",
  "COLORTERM",
  "SSH_AUTH_SOCK",
  "LANG",
  "LC_ALL",
]);

const injectedDesktopVariables = new Set([
  "GDK_PIXBUF_MODULEDIR",
  "GDK_PIXBUF_MODULE_FILE",
  "GIO_EXTRA_MODULES",
  "GIO_MODULE_DIR",
  "GTK_DATA_PREFIX",
  "GTK_EXE_PREFIX",
  "GTK_PATH",
  "LD_LIBRARY_PATH",
  "LD_PRELOAD",
  "SNAP",
  "SNAP_ARCH",
  "SNAP_INSTANCE_NAME",
  "SNAP_NAME",
  "SNAP_REVISION",
  "SNAP_USER_COMMON",
  "SNAP_USER_DATA",
  "XDG_DATA_DIRS_VSCODE_SNAP_ORIG",
]);

const systemDataDirectories = [
  "/var/lib/flatpak/exports/share",
  "/usr/local/share",
  "/usr/share",
  "/var/lib/snapd/desktop",
];

export function buildDesktopEnvironment(source, platform = process.platform) {
  if (platform === "win32") {
    return Object.fromEntries(
      Object.entries(source).filter(([key, value]) => (
        value !== undefined && !injectedDesktopVariables.has(key)
      )),
    );
  }

  const env = {};
  for (const [key, value] of Object.entries(source)) {
    if (inheritedDesktopVariables.has(key) && value !== undefined) env[key] = value;
  }

  const restoredDataDirectories = source.XDG_DATA_DIRS_VSCODE_SNAP_ORIG?.trim();
  if (restoredDataDirectories) {
    env.XDG_DATA_DIRS = restoredDataDirectories;
    return env;
  }

  const dataDirectories = [...systemDataDirectories];
  if (source.HOME?.trim()) {
    dataDirectories.unshift(resolve(source.HOME, ".local/share/flatpak/exports/share"));
  }
  env.XDG_DATA_DIRS = dataDirectories.join(":");
  return env;
}
