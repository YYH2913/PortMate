import { readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

function rustSources(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? rustSources(path) : entry.name.endsWith(".rs") ? [path] : [];
  });
}

function run(command, args) {
  const result = spawnSync(command, args, { cwd: projectRoot, stdio: "inherit", windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run("cargo", ["fmt", "--all", "--", "--check"]);
// rustfmt does not traverse include! support files. Check those and all external
// suites explicitly, without invoking the separate SDK compatibility workspaces.
for (const entry of readdirSync(join(projectRoot, "test", "rust"))) {
  const sources = rustSources(join(projectRoot, "test", "rust", entry));
  const edition = entry.startsWith("libssh-rs") ? "2018" : "2021";
  run("rustfmt", ["--edition", edition, "--check", ...sources]);
}
