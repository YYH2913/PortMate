import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const exactVersionPattern = /^\d+\.\d+\.\d+$/;
const requiredTrackedFiles = [
  "Cargo.lock",
  "CHANGELOG.md",
  "LICENSE",
  "package-lock.json",
  "package.json",
  "src-tauri/tauri.conf.json",
];
const requiredReleaseHeadings = [
  "Added",
  "Changed",
  "Fixed",
  "Security",
  "Migration",
  "Known Limitations",
];

export function findReleaseSourceViolations(source) {
  const violations = [];
  const packageJson = source.packageJson ?? {};
  const packageLock = source.packageLock ?? {};
  const tauri = source.tauri ?? {};
  const version = packageJson.version;

  if (packageJson.name !== "portmate") violations.push("package.json name must be portmate");
  if (packageJson.private !== true) violations.push("package.json must remain private");
  if (typeof version !== "string" || !exactVersionPattern.test(version)) {
    violations.push("package.json version must be an exact stable semantic version");
  }

  compareVersion(violations, "package-lock.json", packageLock.version, version);
  compareVersion(violations, "package-lock.json root package", packageLock.packages?.[""]?.version, version);
  compareVersion(violations, "src-tauri/tauri.conf.json", tauri.version, version);
  if (packageLock.name !== "portmate" || packageLock.packages?.[""]?.name !== "portmate") {
    violations.push("package-lock.json root package must be portmate");
  }

  if (tauri.productName !== "PortMate") violations.push("Tauri productName must be PortMate");
  if (tauri.identifier !== "dev.portmate.desktop") {
    violations.push("Tauri identifier must be dev.portmate.desktop");
  }
  if (tauri.bundle?.publisher !== "PortMate Contributors") {
    violations.push("Tauri publisher must be PortMate Contributors");
  }
  if (tauri.bundle?.licenseFile !== "../LICENSE") {
    violations.push("Tauri licenseFile must reference ../LICENSE");
  }
  if (tauri.bundle?.resources?.["../LICENSE"] !== "LICENSE") {
    violations.push("Tauri bundle must include the Apache-2.0 LICENSE resource");
  }
  if (tauri.bundle?.resources?.["../THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt"]
      !== "THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt") {
    violations.push("Tauri bundle must include the JetBrains Mono OFL resource");
  }

  const packages = Array.isArray(source.cargoMetadata?.packages)
    ? source.cargoMetadata.packages
    : [];
  const workspaceMembers = new Set(source.cargoMetadata?.workspace_members ?? []);
  const ownedPackages = packages.filter((entry) => workspaceMembers.has(entry.id)
    && (entry.name === "portmate" || entry.name?.startsWith("portmate-")));
  if (ownedPackages.length === 0) violations.push("Cargo metadata contains no PortMate-owned packages");
  for (const entry of ownedPackages) {
    compareVersion(violations, `Cargo package ${entry.name}`, entry.version, version);
    if (entry.license !== "Apache-2.0") {
      violations.push(`Cargo package ${entry.name} must use Apache-2.0`);
    }
    if (!Array.isArray(entry.authors) || !entry.authors.includes("PortMate Contributors")) {
      violations.push(`Cargo package ${entry.name} must list PortMate Contributors`);
    }
  }

  if (!source.licenseText?.includes("Apache License")
      || !source.licenseText?.includes("Version 2.0, January 2004")) {
    violations.push("LICENSE must contain the Apache License 2.0 text");
  }
  if (!source.fontLicenseText?.includes("SIL OPEN FONT LICENSE Version 1.1")
      || !source.fontLicenseText?.includes("JetBrains Mono Project Authors")) {
    violations.push("JetBrains Mono must retain its SIL OFL 1.1 license");
  }

  const releaseSection = currentReleaseSection(source.changelogText ?? "", version);
  if (!releaseSection) {
    violations.push(`CHANGELOG.md is missing a dated [${version}] release section`);
  } else {
    for (const heading of requiredReleaseHeadings) {
      if (!sectionBody(releaseSection, heading)) {
        violations.push(`CHANGELOG.md [${version}] must contain a non-empty ${heading} section`);
      }
    }
  }

  const trackedFiles = source.trackedFiles instanceof Set
    ? source.trackedFiles
    : new Set(source.trackedFiles ?? []);
  for (const path of requiredTrackedFiles) {
    if (!trackedFiles.has(path)) violations.push(`required release source file is not tracked: ${path}`);
  }
  return violations.sort();
}

function compareVersion(violations, label, actual, expected) {
  if (actual !== expected) violations.push(`${label} version ${actual ?? "<missing>"} does not match ${expected ?? "<invalid>"}`);
}

function currentReleaseSection(changelog, version) {
  if (typeof version !== "string") return null;
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`^## \\[${escaped}\\] - \\d{4}-\\d{2}-\\d{2}$`, "m").exec(changelog);
  if (!match) return null;
  const start = match.index + match[0].length;
  const next = changelog.slice(start).search(/^## /m);
  return changelog.slice(start, next < 0 ? undefined : start + next);
}

function sectionBody(releaseSection, heading) {
  const escaped = heading.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`^### ${escaped}\\s*$`, "m").exec(releaseSection);
  if (!match) return "";
  const start = match.index + match[0].length;
  const next = releaseSection.slice(start).search(/^### /m);
  return releaseSection.slice(start, next < 0 ? undefined : start + next).trim();
}

function readJson(path) {
  return JSON.parse(readFileSync(join(projectRoot, path), "utf8"));
}

function main() {
  const cargoMetadata = JSON.parse(execFileSync(
    "cargo",
    ["metadata", "--locked", "--no-deps", "--format-version", "1"],
    { cwd: projectRoot, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
  ));
  const trackedFiles = new Set(execFileSync("git", ["ls-files", "-z"], {
    cwd: projectRoot,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  }).split("\0").filter(Boolean));
  const source = {
    packageJson: readJson("package.json"),
    packageLock: readJson("package-lock.json"),
    tauri: readJson("src-tauri/tauri.conf.json"),
    cargoMetadata,
    trackedFiles,
    licenseText: readFileSync(join(projectRoot, "LICENSE"), "utf8"),
    fontLicenseText: readFileSync(
      join(projectRoot, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"),
      "utf8",
    ),
    changelogText: readFileSync(join(projectRoot, "CHANGELOG.md"), "utf8"),
  };
  const violations = findReleaseSourceViolations(source);
  if (violations.length > 0) {
    throw new Error(`release source boundary failed:\n- ${violations.join("\n- ")}`);
  }
  const ownedPackages = cargoMetadata.packages
    .filter((entry) => cargoMetadata.workspace_members.includes(entry.id)
      && (entry.name === "portmate" || entry.name.startsWith("portmate-")))
    .map((entry) => entry.name)
    .sort();
  console.log(`PortMate ${source.packageJson.version} release source boundary passed (${ownedPackages.join(", ")})`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
