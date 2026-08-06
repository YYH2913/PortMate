import {
  existsSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  readdirSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  findUniqueRegularFile,
  inspectPortableTree,
  verifyWindowsPackageLayout,
} from "./native-package-layout.mjs";
import { smokePackagedApplicationLifecycle } from "./native-packaged-smoke.mjs";
import { smokePackagedSidecarParentWatchdog } from "./native-packaged-sidecar-smoke.mjs";

if (process.platform !== "win32") {
  throw new Error("Windows package verification must run on Windows");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const bundleRoot = join(projectRoot, "target", "release", "bundle");
const sourceMain = join(projectRoot, "target", "release", "portmate.exe");
const sourceSidecar = join(projectRoot, "target", "release", "portmate-mcp.exe");
const sourceLicense = join(projectRoot, "LICENSE");
const sourceThirdPartyLicense = join(projectRoot, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt");
const msi = findSingleArtifact(join(bundleRoot, "msi"), ".msi", "MSI installer");
const nsis = findSingleArtifact(join(bundleRoot, "nsis"), ".exe", "NSIS installer");
const auditRoot = mkdtempSync(join(tmpdir(), "portmate windows package check "));
const msiRoot = join(auditRoot, "msi");
const nsisRoot = join(auditRoot, "nsis");
const expectedUninstaller = join(nsisRoot, "uninstall.exe");
let verifiedMsi;
let verifiedNsis;
const runtimeSmokes = [];
const sidecarWatchdogSmokes = [];
let failure;

try {
  mkdirSync(msiRoot, { recursive: true });
  const msiLog = join(auditRoot, "msi-extract.log");
  run("msiexec.exe", [
    "/a",
    msi,
    "/qn",
    "/norestart",
    "/l*v",
    msiLog,
    `TARGETDIR=${msiRoot}`,
  ]);
  verifiedMsi = verifyWindowsPackageLayout({
    root: msiRoot,
    sourceMain,
    sourceSidecar,
    sourceLicense,
    sourceThirdPartyLicense,
  });
  sidecarWatchdogSmokes.push({
    package: "MSI",
    result: await smokePackagedSidecarParentWatchdog({
      executable: verifiedMsi.sidecar,
      label: "MSI packaged MCP sidecar",
    }),
  });
  runtimeSmokes.push({
    package: "MSI",
    result: await smokePackagedApplicationLifecycle({
      executable: verifiedMsi.main,
      dataDirectory: join(auditRoot, "runtime-msi", "dev.portmate.desktop"),
      label: "MSI packaged application",
    }),
  });

  run(nsis, ["/S", `/D=${nsisRoot}`]);
  verifiedNsis = verifyWindowsPackageLayout({
    root: nsisRoot,
    sourceMain,
    sourceSidecar,
    sourceLicense,
    sourceThirdPartyLicense,
  });
  sidecarWatchdogSmokes.push({
    package: "NSIS",
    result: await smokePackagedSidecarParentWatchdog({
      executable: verifiedNsis.sidecar,
      label: "NSIS installed MCP sidecar",
    }),
  });
  runtimeSmokes.push({
    package: "NSIS",
    result: await smokePackagedApplicationLifecycle({
      executable: verifiedNsis.main,
      dataDirectory: join(auditRoot, "runtime-nsis", "dev.portmate.desktop"),
      label: "NSIS installed application",
    }),
  });
  const uninstaller = findUniqueRegularFile(
    inspectPortableTree(nsisRoot),
    "uninstall.exe",
    { caseInsensitive: true },
  );
  if (dirname(uninstaller).toLocaleLowerCase("en-US")
      !== verifiedNsis.applicationDirectory.toLocaleLowerCase("en-US")) {
    throw new Error(`NSIS uninstaller is outside the application directory: ${uninstaller}`);
  }
} catch (error) {
  failure = error;
} finally {
  if (existsSync(expectedUninstaller)) {
    try {
      run(expectedUninstaller, ["/S", `_?=${nsisRoot}`]);
      if (verifiedNsis) assertNsisPayloadRemoved(verifiedNsis);
    } catch (error) {
      failure ??= error;
    }
  } else if (verifiedNsis) {
    failure ??= new Error(`NSIS installation did not provide its uninstaller: ${expectedUninstaller}`);
  }
  try {
    rmSync(auditRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  } catch (error) {
    failure ??= error;
  }
}
if (failure) throw failure;

console.log(JSON.stringify({
  msi,
  nsis,
  verifiedPackages: ["MSI", "NSIS"],
  verified: [
    "unique main executable, MCP sidecar, application license, and JetBrains Mono license",
    "non-empty regular payload files",
    "release-binary and both license SHA-256 equality",
    "portable package symlinks",
    "MSI administrative extraction",
    "NSIS silent install and uninstall",
    "installation, execution, runtime data, and sidecar paths containing spaces",
    "installed main-process IPC, stable restart and legacy-migration Store, fail-closed two-store conflict, credential rotation, clean exit, and endpoint cleanup",
    "installed MCP sidecar HTTP readiness and abnormal-parent cleanup",
  ],
  payloads: {
    msi: verifiedMsi,
    nsis: verifiedNsis,
  },
  runtimeSmokes,
  sidecarWatchdogSmokes,
}, null, 2));

function findSingleArtifact(directory, extension, label) {
  const matches = readdirSync(directory)
    .map((entry) => join(directory, entry))
    .filter((path) => lstatSync(path).isFile() && extname(path).toLocaleLowerCase("en-US") === extension);
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one ${label} in ${directory}, found ${matches.length}`);
  }
  return matches[0];
}

function assertNsisPayloadRemoved(payload) {
  for (const path of [payload.main, payload.sidecar, payload.license, payload.thirdPartyLicense]) {
    if (existsSync(path)) throw new Error(`NSIS uninstall left an application payload file: ${path}`);
  }
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: process.env,
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${basename(command)} failed with exit code ${result.status ?? 1}`);
  }
}
