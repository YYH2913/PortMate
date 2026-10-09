import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";

export function createNativePackageCheckWorkspace({
  projectRoot,
  label,
  environment = process.env,
}) {
  if (typeof projectRoot !== "string" || !isAbsolute(projectRoot)) {
    throw new Error("Native package check project root must be absolute");
  }
  if (typeof label !== "string" || !label.trim() || /[\0/\\]/.test(label)) {
    throw new Error("Native package check label must be a non-empty path segment");
  }

  const workspaceParent = join(projectRoot, "target", "native-package-check");
  mkdirSync(workspaceParent, { recursive: true, mode: 0o700 });
  const root = mkdtempSync(join(workspaceParent, `${label.trim()} `));
  const temporaryDirectory = join(root, "temporary files");
  mkdirSync(temporaryDirectory, { recursive: false, mode: 0o700 });
  const childEnvironment = nativePackageCheckEnvironment(environment, temporaryDirectory);
  let cleaned = false;

  return {
    root,
    temporaryDirectory,
    environment: childEnvironment,
    cleanup() {
      if (cleaned) return;
      rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
      cleaned = true;
    },
  };
}

export function nativePackageCheckEnvironment(environment, temporaryDirectory) {
  if (!environment || typeof environment !== "object") {
    throw new Error("Native package check environment must be an object");
  }
  if (typeof temporaryDirectory !== "string" || !isAbsolute(temporaryDirectory)) {
    throw new Error("Native package check temporary directory must be absolute");
  }
  const resolvedTemporaryDirectory = resolve(temporaryDirectory);
  const metadata = statSync(resolvedTemporaryDirectory);
  if (!metadata.isDirectory()) {
    throw new Error("Native package check temporary path must be a directory");
  }
  const javaTemporaryDirectory = resolvedTemporaryDirectory
    .replaceAll("\\", "/")
    .replaceAll('"', '\\"');
  const javaTemporaryOption = `-Djava.io.tmpdir="${javaTemporaryDirectory}"`;
  const existingJavaOptions = typeof environment.JAVA_TOOL_OPTIONS === "string"
    ? environment.JAVA_TOOL_OPTIONS.trim()
    : "";

  return {
    ...environment,
    TMPDIR: resolvedTemporaryDirectory,
    TMP: resolvedTemporaryDirectory,
    TEMP: resolvedTemporaryDirectory,
    JAVA_TOOL_OPTIONS: existingJavaOptions
      ? `${existingJavaOptions} ${javaTemporaryOption}`
      : javaTemporaryOption,
  };
}

export function temporaryDirectoryFromEnvironment(
  environment = process.env,
  platform = process.platform,
) {
  const candidates = platform === "win32"
    ? [environment?.TEMP, environment?.TMP]
    : [environment?.TMPDIR, environment?.TMP, environment?.TEMP];
  const configured = candidates.find((value) => typeof value === "string" && value.trim());
  return resolve(configured?.trim() || tmpdir());
}
