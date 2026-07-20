import { resolve } from "node:path";

const inheritedDesktopVariables = new Set([
  "HOME",
  "USER",
  "LOGNAME",
  "SHELL",
  "PATH",
  "DISPLAY",
  "WAYLAND_DISPLAY",
  "XDG_RUNTIME_DIR",
  "DBUS_SESSION_BUS_ADDRESS",
  "XDG_CURRENT_DESKTOP",
  "XDG_SESSION_TYPE",
  "TERM",
  "LANG",
  "LC_ALL",
]);

const systemDataDirectories = [
  "/var/lib/flatpak/exports/share",
  "/usr/local/share",
  "/usr/share",
  "/var/lib/snapd/desktop",
];

export function buildDesktopEnvironment(source) {
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
