import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  buildSysrootEnvironment,
  prepareGssapiBuildEnvironment,
} from "./gssapi-build-environment.mjs";

const temporaryRoots = [];
afterEach(() => {
  for (const root of temporaryRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function fakeSysroot() {
  const root = mkdtempSync(join(tmpdir(), "portmate-gssapi-env-test-"));
  temporaryRoots.push(root);
  mkdirSync(join(root, "usr/lib/x86_64-linux-gnu/pkgconfig"), { recursive: true });
  writeFileSync(join(root, "usr/lib/x86_64-linux-gnu/pkgconfig/libssh.pc"), "Name: libssh\nVersion: 0.10.6\n");
  writeFileSync(join(root, "usr/lib/x86_64-linux-gnu/libssh.so.4"), "fake");
  return root;
}

describe("GSSAPI build environment", () => {
  it("keeps the system environment when libssh is available", () => {
    const calls = [];
    const result = prepareGssapiBuildEnvironment({
      projectRoot: "/project",
      env: { PATH: "/bin" },
      platform: "linux",
      runCommand: (command, args) => {
        calls.push([command, args]);
        return { status: command === "pkg-config" ? 0 : 1 };
      },
    });
    expect(result.source).toBe("system");
    expect(result.env).toEqual({ PATH: "/bin" });
    expect(calls).toHaveLength(1);
  });

  it("configures an explicit sysroot and preserves existing paths", () => {
    const root = fakeSysroot();
    const result = prepareGssapiBuildEnvironment({
      projectRoot: "/project",
      env: { PKG_CONFIG_PATH: "/opt/pkg", LD_LIBRARY_PATH: "/opt/lib" },
      sysroot: root,
      platform: "linux",
      runCommand: (command, args) => ({
        status: command === "pkg-config" && args[0] === "--atleast-version" ? 0 : 1,
      }),
    });
    expect(result.source).toBe("PORTMATE_GSSAPI_SYSROOT");
    expect(result.env.PKG_CONFIG_SYSROOT_DIR).toBe(root);
    expect(result.env.PKG_CONFIG_PATH).toContain(join(root, "usr/lib/x86_64-linux-gnu/pkgconfig"));
    expect(result.env.PKG_CONFIG_PATH).toContain("/opt/pkg");
    expect(result.env.LD_LIBRARY_PATH).toContain(join(root, "usr/lib/x86_64-linux-gnu"));
    expect(result.env.LD_LIBRARY_PATH).toContain("/opt/lib");
  });

  it("accepts Debian shared-library symlinks", () => {
    const root = mkdtempSync(join(tmpdir(), "portmate-gssapi-env-test-"));
    temporaryRoots.push(root);
    const libraryDirectory = join(root, "usr/lib/x86_64-linux-gnu");
    const pkgConfigDirectory = join(libraryDirectory, "pkgconfig");
    mkdirSync(pkgConfigDirectory, { recursive: true });
    writeFileSync(join(pkgConfigDirectory, "libssh.pc"), "Name: libssh\nVersion: 0.10.6\n");
    writeFileSync(join(libraryDirectory, "libssh.so.4.9.6"), "fake");
    symlinkSync("libssh.so.4.9.6", join(libraryDirectory, "libssh.so.4"));
    symlinkSync("libssh.so.4", join(libraryDirectory, "libssh.so"));

    const result = buildSysrootEnvironment(root, {});

    expect(result.LD_LIBRARY_PATH).toContain(libraryDirectory);
  });

  it("downloads and extracts both Debian packages once", () => {
    const root = mkdtempSync(join(tmpdir(), "portmate-gssapi-env-test-"));
    temporaryRoots.push(root);
    const commands = [];
    let configured = false;
    const result = prepareGssapiBuildEnvironment({
      projectRoot: "/project",
      env: {},
      defaultSysroot: join(root, "sysroot"),
      platform: "linux",
      runCommand: (command, args, options = {}) => {
        commands.push({ command, args, options });
        if (command === "pkg-config") return {
          status: options.env?.PKG_CONFIG_SYSROOT_DIR ? 0 : (configured ? 0 : 1),
        };
        if (command === "apt-get" && args[0] === "download") {
          writeFileSync(join(options.cwd, "libssh-dev_0.10.6_amd64.deb"), "deb");
          writeFileSync(join(options.cwd, "libssh-4_0.10.6_amd64.deb"), "deb");
          return { status: 0 };
        }
        if (command === "dpkg-deb" && args[0] === "-x") {
          configured = true;
          const sysroot = args.at(-1);
          mkdirSync(join(sysroot, "usr/lib/x86_64-linux-gnu/pkgconfig"), { recursive: true });
          writeFileSync(join(sysroot, "usr/lib/x86_64-linux-gnu/pkgconfig/libssh.pc"), "Version: 0.10.6\n");
          writeFileSync(join(sysroot, "usr/lib/x86_64-linux-gnu/libssh.so.4"), "fake");
          return { status: 0 };
        }
        return { status: 0 };
      },
    });
    expect(result.source).toBe("auto-downloaded");
    expect(commands.filter(({ command, args }) => command === "apt-get" && args[0] === "download")).toHaveLength(1);
    expect(commands.filter(({ command, args }) => command === "dpkg-deb" && args[0] === "-x")).toHaveLength(2);

    const second = prepareGssapiBuildEnvironment({
      projectRoot: "/project",
      env: {},
      defaultSysroot: join(root, "sysroot"),
      platform: "linux",
      runCommand: (command, args, options = {}) => ({
        status: command === "pkg-config" && options.env?.PKG_CONFIG_SYSROOT_DIR ? 0 : 1,
      }),
    });
    expect(second.source).toBe("auto-downloaded");
  });

  it("downloads again when a same-package duplicate cannot satisfy the cache", () => {
    const root = mkdtempSync(join(tmpdir(), "portmate-gssapi-env-test-"));
    temporaryRoots.push(root);
    const sysroot = join(root, "sysroot");
    const packageDir = join(sysroot, "packages");
    mkdirSync(packageDir, { recursive: true });
    writeFileSync(join(packageDir, "libssh-dev_0.10.5_amd64.deb"), "old");
    writeFileSync(join(packageDir, "libssh-dev_0.10.6_amd64.deb"), "new");
    let downloads = 0;

    prepareGssapiBuildEnvironment({
      projectRoot: "/project",
      env: {},
      defaultSysroot: sysroot,
      platform: "linux",
      runCommand: (command, args, options = {}) => {
        if (command === "pkg-config") {
          return { status: options.env?.PKG_CONFIG_SYSROOT_DIR ? 0 : 1 };
        }
        if (command === "apt-get" && args[0] === "download") {
          downloads += 1;
          writeFileSync(join(options.cwd, "libssh-4_0.10.6_amd64.deb"), "runtime");
          return { status: 0 };
        }
        if (command === "dpkg-deb" && args[0] === "-x") {
          const destination = args.at(-1);
          mkdirSync(join(destination, "usr/lib/x86_64-linux-gnu/pkgconfig"), { recursive: true });
          writeFileSync(join(destination, "usr/lib/x86_64-linux-gnu/pkgconfig/libssh.pc"), "Version: 0.10.6\n");
          writeFileSync(join(destination, "usr/lib/x86_64-linux-gnu/libssh.so.4"), "fake");
        }
        return { status: 0 };
      },
    });

    expect(downloads).toBe(1);
  });

  it("fails closed for a missing sysroot", () => {
    expect(() => buildSysrootEnvironment("/does/not/exist", {})).toThrow(/libssh\.pc is missing/);
  });
});
