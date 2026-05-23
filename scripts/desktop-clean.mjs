import { spawn } from "node:child_process";

const keep = new Set([
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

const env = {};
for (const [key, value] of Object.entries(process.env)) {
  if (keep.has(key) && value !== undefined) {
    env[key] = value;
  }
}

env.XDG_DATA_DIRS =
  process.env.XDG_DATA_DIRS_VSCODE_SNAP_ORIG ??
  "/home/yyh/.local/share/flatpak/exports/share:/var/lib/flatpak/exports/share:/usr/local/share:/usr/share:/var/lib/snapd/desktop";

const child = spawn("npm", ["run", "desktop"], {
  stdio: "inherit",
  env,
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  }
  process.exit(code ?? 0);
});

