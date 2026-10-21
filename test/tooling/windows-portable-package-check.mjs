import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createNativePackageCheckWorkspace } from "./native-package-workspace.mjs";
import { smokePackagedApplicationLifecycle } from "./native-packaged-smoke.mjs";
import { smokePackagedSidecarParentWatchdog } from "./native-packaged-sidecar-smoke.mjs";

if (process.platform !== "win32") throw new Error("Portable Windows package checks require Windows");
const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const target = resolve(root, process.env.CARGO_TARGET_DIR || "target");
const archive = JSON.parse(readFileSync(join(target, "native-ci/release-archives.json"), "utf8"));
const workspace = createNativePackageCheckWorkspace({ projectRoot: root, label: "portable windows package check" });
try {
  const lifecycle = await smokePackagedApplicationLifecycle({ executable: archive.windows.main, dataDirectory: join(workspace.root, "runtime", "dev.portmate.desktop"), label: "Windows portable application", environment: workspace.environment });
  const sidecar = await smokePackagedSidecarParentWatchdog({ executable: archive.windows.sidecar, label: "Windows portable MCP sidecar", environment: workspace.environment });
  const report = { lifecycle, sidecar, archives: archive.artifacts };
  writeFileSync(join(target, "native-ci/windows-portable-package.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify(report, null, 2));
} finally {
  workspace.cleanup();
}
