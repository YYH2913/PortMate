import {
  lstatSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { sha256File, verifyMacAppBundle } from "./native-package-layout.mjs";
import { smokePackagedApplicationLifecycle } from "./native-packaged-smoke.mjs";
import { smokePackagedSidecarParentWatchdog } from "./native-packaged-sidecar-smoke.mjs";

if (process.platform !== "darwin") {
  throw new Error("macOS package verification must run on macOS");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tauriConfig = JSON.parse(
  readFileSync(join(projectRoot, "src-tauri", "tauri.conf.json"), "utf8"),
);
const expectedCategory = macCategory(tauriConfig.bundle?.category);
const expectedMetadata = {
  CFBundleIdentifier: tauriConfig.identifier,
  CFBundleShortVersionString: tauriConfig.version,
  CFBundleVersion: tauriConfig.version,
  CFBundleExecutable: "portmate",
  LSApplicationCategoryType: expectedCategory,
};
const bundleRoot = join(projectRoot, "target", "release", "bundle");
const sourceMain = join(projectRoot, "target", "release", "portmate");
const sourceSidecar = join(projectRoot, "target", "release", "portmate-mcp");
const sourceLicense = join(projectRoot, "LICENSE");
const sourceThirdPartyLicense = join(projectRoot, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt");
const app = findSingleBundle(join(bundleRoot, "macos"));
const dmg = findSingleArtifact(join(bundleRoot, "dmg"), ".dmg", "DMG image");
const auditRoot = mkdtempSync(join(tmpdir(), "portmate-macos-package-check-"));
const mountPoint = join(auditRoot, "mounted-dmg");
let mounted = false;
let verifiedApp;
let verifiedDmg;
let failure;
let directBinaryVerification;
const runtimeSmokes = [];
const sidecarWatchdogSmokes = [];

try {
  const appMain = join(app, "Contents", "MacOS", "portmate");
  const appSidecar = join(app, "Contents", "MacOS", "portmate-mcp");
  const exactReleaseBinaries = sha256File(sourceMain) === sha256File(appMain)
    && sha256File(sourceSidecar) === sha256File(appSidecar);
  if (exactReleaseBinaries) {
    directBinaryVerification = "release-source SHA-256";
  } else {
    run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", app]);
    directBinaryVerification = "strict Apple code-signature verification";
  }
  verifiedApp = verifyApp(app, { compareBinaries: exactReleaseBinaries });
  sidecarWatchdogSmokes.push({
    package: "macOS app",
    result: await smokePackagedSidecarParentWatchdog({
      executable: verifiedApp.sidecar,
      label: "macOS application MCP sidecar",
    }),
  });
  runtimeSmokes.push({
    package: "macOS app",
    result: await smokePackagedApplicationLifecycle({
      executable: verifiedApp.main,
      dataDirectory: join(auditRoot, "runtime-app", "dev.portmate.desktop"),
      label: "macOS application bundle",
    }),
  });
  run("hdiutil", ["verify", dmg]);
  mkdirSync(mountPoint, { recursive: true });
  run("hdiutil", [
    "attach",
    dmg,
    "-readonly",
    "-nobrowse",
    "-mountpoint",
    mountPoint,
  ]);
  mounted = true;
  const dmgApp = findSingleBundle(mountPoint);
  verifiedDmg = verifyApp(dmgApp, {
    sourceMain: verifiedApp.main,
    sourceSidecar: verifiedApp.sidecar,
  });
  sidecarWatchdogSmokes.push({
    package: "DMG",
    result: await smokePackagedSidecarParentWatchdog({
      executable: verifiedDmg.sidecar,
      label: "DMG packaged MCP sidecar",
    }),
  });
  runtimeSmokes.push({
    package: "DMG",
    result: await smokePackagedApplicationLifecycle({
      executable: verifiedDmg.main,
      dataDirectory: join(auditRoot, "runtime-dmg", "dev.portmate.desktop"),
      label: "DMG packaged application",
    }),
  });
} catch (error) {
  failure = error;
} finally {
  if (mounted) {
    try {
      run("hdiutil", ["detach", mountPoint]);
    } catch (error) {
      failure ??= error;
    }
  }
  try {
    rmSync(auditRoot, { recursive: true, force: true });
  } catch (error) {
    failure ??= error;
  }
}
if (failure) throw failure;

console.log(JSON.stringify({
  app,
  dmg,
  verifiedPackages: ["macOS app", "DMG"],
  verified: [
    "fixed Contents/MacOS and Contents/Resources payload layout",
    "unique non-empty main executable, MCP sidecar, application license, JetBrains Mono license, and Info.plist",
    "unsigned release SHA-256 or strict code-signature verification",
    "direct app/DMG binary and both repository license SHA-256 equality",
    "bundle identifier, version, executable, and application category",
    "portable bundle symlinks",
    "DMG verification and read-only mount",
    "packaged main-process IPC, stable restart and legacy-migration Store, fail-closed two-store conflict, credential rotation, clean exit, and endpoint cleanup",
    "packaged MCP sidecar HTTP readiness and abnormal-parent cleanup",
  ],
  payloads: {
    app: verifiedApp,
    dmg: verifiedDmg,
  },
  directBinaryVerification,
  runtimeSmokes,
  sidecarWatchdogSmokes,
}, null, 2));

function verifyApp(path, options = {}) {
  const infoPlist = join(path, "Contents", "Info.plist");
  const metadata = Object.fromEntries(Object.keys(expectedMetadata).map((key) => [
    key,
    readCommandOutput("plutil", ["-extract", key, "raw", "-o", "-", infoPlist]).trim(),
  ]));
  return verifyMacAppBundle({
    app: path,
    sourceMain: options.sourceMain ?? sourceMain,
    sourceSidecar: options.sourceSidecar ?? sourceSidecar,
    sourceLicense,
    sourceThirdPartyLicense,
    metadata,
    expectedMetadata,
    compareBinaries: options.compareBinaries ?? true,
  });
}

function findSingleBundle(root) {
  const matches = [];
  visit(root);
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one PortMate.app below ${root}, found ${matches.length}`);
  }
  return matches[0];

  function visit(path) {
    const metadata = lstatSync(path);
    if (metadata.isSymbolicLink() || !metadata.isDirectory()) return;
    if (basename(path) === "PortMate.app") {
      matches.push(resolve(path));
      return;
    }
    for (const entry of readdirSync(path)) visit(join(path, entry));
  }
}

function findSingleArtifact(directory, extension, label) {
  const matches = readdirSync(directory)
    .map((entry) => join(directory, entry))
    .filter((path) => lstatSync(path).isFile() && extname(path).toLowerCase() === extension);
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one ${label} in ${directory}, found ${matches.length}`);
  }
  return matches[0];
}

function macCategory(category) {
  const categories = {
    DeveloperTool: "public.app-category.developer-tools",
  };
  const value = categories[category];
  if (!value) throw new Error(`Unsupported macOS bundle category for verification: ${category}`);
  return value;
}

function readCommandOutput(command, args) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: process.env,
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${basename(command)} failed with exit code ${result.status ?? 1}: ${result.stderr || result.stdout}`,
    );
  }
  return result.stdout;
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: process.env,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${basename(command)} failed with exit code ${result.status ?? 1}`);
  }
}
