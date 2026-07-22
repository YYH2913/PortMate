import { resolve } from "node:path";

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
  const env = Object.fromEntries(
    Object.entries(source).filter(([key, value]) => (
      value !== undefined && !injectedDesktopVariables.has(key)
    )),
  );
  if (platform === "win32") return env;

  const restoredDataDirectories = source.XDG_DATA_DIRS_VSCODE_SNAP_ORIG?.trim();
  if (restoredDataDirectories) {
    env.XDG_DATA_DIRS = restoredDataDirectories;
    return env;
  }

  const configuredDataDirectories = source.XDG_DATA_DIRS?.trim();
  const snapInjected = Boolean(source.SNAP?.trim() || source.SNAP_NAME?.trim());
  if (configuredDataDirectories && !snapInjected) {
    env.XDG_DATA_DIRS = configuredDataDirectories;
    return env;
  }

  const dataDirectories = [...systemDataDirectories];
  if (source.HOME?.trim()) {
    dataDirectories.unshift(resolve(source.HOME, ".local/share/flatpak/exports/share"));
  }
  env.XDG_DATA_DIRS = dataDirectories.join(":");
  return env;
}
