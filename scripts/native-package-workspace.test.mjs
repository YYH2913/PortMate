import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  createNativePackageCheckWorkspace,
  nativePackageCheckEnvironment,
  temporaryDirectoryFromEnvironment,
} from "./native-package-workspace.mjs";

const temporaryRoots = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("native package check workspace", () => {
  it("keeps package extraction and child temporary files below target", () => {
    const projectRoot = temporaryRoot("portmate package workspace project ");
    const workspace = createNativePackageCheckWorkspace({
      projectRoot,
      label: "linux package check",
      environment: { JAVA_TOOL_OPTIONS: "-Xmx512m" },
    });

    expect(workspace.root.startsWith(join(projectRoot, "target", "native-package-check"))).toBe(true);
    expect(workspace.root).toContain("linux package check ");
    expect(workspace.temporaryDirectory).toContain("temporary files");
    expect(statSync(workspace.root).isDirectory()).toBe(true);
    if (process.platform !== "win32") {
      expect(statSync(workspace.root).mode & 0o077).toBe(0);
    }
    expect(workspace.environment.TMPDIR).toBe(workspace.temporaryDirectory);
    expect(workspace.environment.TMP).toBe(workspace.temporaryDirectory);
    expect(workspace.environment.TEMP).toBe(workspace.temporaryDirectory);
    expect(workspace.environment.JAVA_TOOL_OPTIONS).toBe(
      `-Xmx512m -Djava.io.tmpdir="${workspace.temporaryDirectory.replaceAll("\\", "/")}"`,
    );

    workspace.cleanup();
    workspace.cleanup();
    expect(existsSync(workspace.root)).toBe(false);
  });

  it("rejects invalid roots, labels, and temporary paths", () => {
    const projectRoot = temporaryRoot("portmate-package-workspace-invalid-");
    const file = join(projectRoot, "not-a-directory");
    const directory = join(projectRoot, "temporary");
    writeFileSync(file, "file");
    mkdirSync(directory);

    expect(() => createNativePackageCheckWorkspace({ projectRoot: "relative", label: "linux" }))
      .toThrow(/project root must be absolute/);
    expect(() => createNativePackageCheckWorkspace({ projectRoot, label: "../linux" }))
      .toThrow(/label must be a non-empty path segment/);
    expect(() => nativePackageCheckEnvironment({}, file)).toThrow(/must be a directory/);
    expect(() => nativePackageCheckEnvironment(null, directory)).toThrow(/environment must be an object/);
  });

  it("uses platform-specific temporary environment precedence", () => {
    expect(temporaryDirectoryFromEnvironment({
      TMPDIR: "/unix/tmpdir",
      TMP: "/unix/tmp",
      TEMP: "/unix/temp",
    }, "linux")).toBe(resolve("/unix/tmpdir"));
    expect(temporaryDirectoryFromEnvironment({
      TMP: "C:/windows/tmp",
      TEMP: "C:/windows/temp",
    }, "win32")).toBe(resolve("C:/windows/temp"));
  });
});

function temporaryRoot(prefix) {
  const root = mkdtempSync(join(tmpdir(), prefix));
  temporaryRoots.push(root);
  return root;
}
