import {
  existsSync,
  mkdirSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const minimumLibsshVersion = "0.9.7";
const packageNames = ["libssh-dev", "libssh-4"];

export function prepareGssapiBuildEnvironment({
  projectRoot,
  env = process.env,
  sysroot = env.PORTMATE_GSSAPI_SYSROOT,
  defaultSysroot = resolve(projectRoot, "target/gssapi-compat/sysroot"),
  platform = process.platform,
  runCommand = spawnCommand,
} = {}) {
  if (platform !== "linux") {
    throw new Error("The SSH GSSAPI build environment requires Linux");
  }

  if (sysroot) {
    return configureSysroot(resolve(sysroot), env, runCommand, "PORTMATE_GSSAPI_SYSROOT");
  }
  if (pkgConfigWorks(env, runCommand)) {
    return { env: { ...env }, source: "system", sysroot: null };
  }

  const requestedSysroot = defaultSysroot;

  ensureCommand(runCommand, "apt-get", ["--version"]);
  ensureCommand(runCommand, "dpkg-deb", ["--version"]);
  const packageDir = join(requestedSysroot, "packages");
  mkdirSync(packageDir, { recursive: true });
  const cachedPackages = findDebianPackages(packageDir);
  if (!hasAllDebianPackages(cachedPackages)) {
    const download = runCommand("apt-get", ["download", ...packageNames], {
      cwd: packageDir,
      env,
    });
    if (download.status !== 0) {
      throw commandFailure("apt-get download", download);
    }
  }

  const packages = findDebianPackages(packageDir);
  for (const packageName of packageNames) {
    if (!packages.some((file) => file.startsWith(`${packageName}_`))) {
      throw new Error(`apt-get did not provide ${packageName} in ${packageDir}`);
    }
  }

  const marker = join(requestedSysroot, ".portmate-libssh-extracted");
  if (!existsSync(marker)) {
    mkdirSync(requestedSysroot, { recursive: true });
    for (const packageName of packageNames) {
      const packageFile = packages
        .filter((file) => file.startsWith(`${packageName}_`))
        .sort()
        .at(-1);
      const extracted = runCommand("dpkg-deb", ["-x", join(packageDir, packageFile), requestedSysroot], {
        env,
      });
      if (extracted.status !== 0) {
        throw commandFailure(`dpkg-deb -x ${packageFile}`, extracted);
      }
    }
    writeFileSync(marker, `${packages.join("\n")}\n`, "utf8");
  }

  return configureSysroot(requestedSysroot, env, runCommand, "auto-downloaded");
}

export function buildSysrootEnvironment(sysroot, env = process.env) {
  const root = resolve(sysroot);
  const pkgConfigDirectories = findFiles(root, "libssh.pc").map((file) => file.slice(0, file.lastIndexOf("/")));
  if (pkgConfigDirectories.length === 0) {
    throw new Error(`libssh.pc is missing from GSSAPI sysroot: ${root}`);
  }
  const libraryDirectories = findLibraryDirectories(root);
  if (libraryDirectories.length === 0) {
    throw new Error(`libssh shared library is missing from GSSAPI sysroot: ${root}`);
  }
  return {
    ...env,
    PKG_CONFIG_PATH: prependPaths(pkgConfigDirectories, env.PKG_CONFIG_PATH),
    PKG_CONFIG_SYSROOT_DIR: root,
    LD_LIBRARY_PATH: prependPaths(libraryDirectories, env.LD_LIBRARY_PATH),
  };
}

function configureSysroot(sysroot, env, runCommand, source) {
  const configured = buildSysrootEnvironment(sysroot, env);
  if (!pkgConfigWorks(configured, runCommand)) {
    throw new Error(`libssh >= ${minimumLibsshVersion} is unavailable in GSSAPI sysroot: ${sysroot}`);
  }
  return { env: configured, source, sysroot };
}

function pkgConfigWorks(env, runCommand) {
  const result = runCommand("pkg-config", ["--atleast-version", minimumLibsshVersion, "libssh"], { env });
  return result.status === 0;
}

function ensureCommand(runCommand, command, args) {
  const result = runCommand(command, args, { capture: true });
  if (result.error?.code === "ENOENT") {
    throw new Error(`required command is unavailable: ${command}\nDebian/Ubuntu test dependencies: apt-get, dpkg-deb, pkg-config, jq, binutils`);
  }
  if (result.status !== 0 && result.error) throw result.error;
}

function findDebianPackages(directory) {
  return readdirSync(directory).filter((file) => file.endsWith(".deb") && packageNames.some((name) => file.startsWith(`${name}_`)));
}

function hasAllDebianPackages(files) {
  return packageNames.every((packageName) => files.some((file) => file.startsWith(`${packageName}_`)));
}

function findFiles(root, basename) {
  const found = [];
  if (!existsSync(root)) return found;
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) found.push(...findFiles(path, basename));
    else if (entry.isFile() && entry.name === basename) found.push(path);
  }
  return found;
}

function findLibraryDirectories(root) {
  const directories = new Set();
  for (const file of findFiles(root, "libssh.so")) directories.add(file.slice(0, file.lastIndexOf("/")));
  if (directories.size === 0) {
    for (const file of findFiles(root, "libssh.so.4")) directories.add(file.slice(0, file.lastIndexOf("/")));
  }
  return [...directories];
}

function prependPaths(paths, existing) {
  return [...new Set([...paths, ...(existing ?? "").split(":").filter(Boolean)])].join(":");
}

function commandFailure(label, result) {
  const details = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
  return new Error(`${label} failed${details ? `\n${details}` : ""}`);
}

function spawnCommand(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  return {
    status: result.status ?? 1,
    stdout: result.stdout ?? "",
    stderr: result.stderr ?? "",
    error: result.error,
  };
}
