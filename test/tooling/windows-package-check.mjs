import {
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
} from "node:fs";
import { dirname, extname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  findUniqueRegularFile,
  inspectPortableTree,
  verifyWindowsPackageLayout,
} from "../../scripts/native-package-layout.mjs";
import { verifyWindowsReleaseBinary } from "../../scripts/windows-release-binary.mjs";
import { smokePackagedApplicationLifecycle } from "./native-packaged-smoke.mjs";
import { smokePackagedSidecarParentWatchdog } from "./native-packaged-sidecar-smoke.mjs";
import { createNativePackageCheckWorkspace } from "./native-package-workspace.mjs";
import { runSilentNsis } from "./windows-nsis-silent.mjs";
import { extractWindowsMsi } from "./windows-msi-admin-extract.mjs";

if (process.platform !== "win32") {
  throw new Error("Windows package verification must run on Windows");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const targetRoot = resolveTargetDirectory(projectRoot);
const bundleRoot = join(targetRoot, "release", "bundle");
const sourceMain = join(targetRoot, "release", "portmate.exe");
const sourceSidecar = join(targetRoot, "release", "portmate-mcp.exe");
const sourceLicense = join(projectRoot, "LICENSE");
const sourceThirdPartyLicense = join(projectRoot, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt");
const releaseBinary = verifyWindowsReleaseBinary({
  executable: sourceMain,
  frontendDist: join(projectRoot, "dist"),
});
const msi = findSingleArtifact(join(bundleRoot, "msi"), ".msi", "MSI installer");
const nsis = findSingleArtifact(join(bundleRoot, "nsis"), ".exe", "NSIS installer");
const packageWorkspace = createNativePackageCheckWorkspace({
  projectRoot,
  label: "portmate windows package check",
});
const auditRoot = packageWorkspace.root;
const packageEnvironment = packageWorkspace.environment;
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
  console.log("Windows package check: MSI administrative extraction");
  extractWindowsMsi({
    installer: msi,
    destination: msiRoot,
    logPath: msiLog,
    diagnosticLogPath: join(targetRoot, "native-ci", "windows-msi-extract.log"),
    cwd: projectRoot,
    environment: packageEnvironment,
  });
  console.log("Windows package check: verify extracted MSI payload");
  verifiedMsi = verifyWindowsPackageLayout({
    root: msiRoot,
    bundleType: "msi",
    sourceMain,
    sourceSidecar,
    sourceLicense,
    sourceThirdPartyLicense,
  });
  console.log("Windows package check: MSI sidecar watchdog");
  sidecarWatchdogSmokes.push({
    package: "MSI",
    result: await smokePackagedSidecarParentWatchdog({
      executable: verifiedMsi.sidecar,
      label: "MSI packaged MCP sidecar",
      environment: packageEnvironment,
    }),
  });
  console.log("Windows package check: MSI application lifecycle");
  runtimeSmokes.push({
    package: "MSI",
    result: await smokePackagedApplicationLifecycle({
      executable: verifiedMsi.main,
      dataDirectory: join(auditRoot, "runtime-msi", "dev.portmate.desktop"),
      label: "MSI packaged application",
      environment: packageEnvironment,
    }),
  });

  console.log("Windows package check: NSIS silent installation");
  runSilentNsis({
    executable: nsis,
    installationDirectory: nsisRoot,
    phase: "install",
    cwd: projectRoot,
    environment: packageEnvironment,
  });
  console.log("Windows package check: verify installed NSIS payload");
  verifiedNsis = verifyWindowsPackageLayout({
    root: nsisRoot,
    bundleType: "nsis",
    sourceMain,
    sourceSidecar,
    sourceLicense,
    sourceThirdPartyLicense,
  });
  console.log("Windows package check: NSIS sidecar watchdog");
  sidecarWatchdogSmokes.push({
    package: "NSIS",
    result: await smokePackagedSidecarParentWatchdog({
      executable: verifiedNsis.sidecar,
      label: "NSIS installed MCP sidecar",
      environment: packageEnvironment,
    }),
  });
  console.log("Windows package check: NSIS application lifecycle");
  runtimeSmokes.push({
    package: "NSIS",
    result: await smokePackagedApplicationLifecycle({
      executable: verifiedNsis.main,
      dataDirectory: join(auditRoot, "runtime-nsis", "dev.portmate.desktop"),
      label: "NSIS installed application",
      environment: packageEnvironment,
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
      console.log("Windows package check: NSIS silent uninstallation");
      runSilentNsis({
        executable: expectedUninstaller,
        installationDirectory: nsisRoot,
        phase: "uninstall",
        cwd: projectRoot,
        environment: packageEnvironment,
      });
      if (verifiedNsis) assertNsisPayloadRemoved(verifiedNsis);
    } catch (error) {
      failure ??= error;
    }
  } else if (verifiedNsis) {
    failure ??= new Error(`NSIS installation did not provide its uninstaller: ${expectedUninstaller}`);
  }
  try {
    packageWorkspace.cleanup();
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
    "PE32+ x86-64 GUI main executable with embedded production frontend assets",
    "non-empty regular payload files",
    "package-specific main executable, MCP sidecar, and both license SHA-256 equality",
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
  releaseBinary,
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

function resolveTargetDirectory(root) {
  const configured = process.env.CARGO_TARGET_DIR;
  if (!configured) return join(root, "target");
  return isAbsolute(configured) ? configured : resolve(root, configured);
}
