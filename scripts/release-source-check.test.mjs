import { describe, expect, it } from "vitest";
import { findReleaseSourceViolations } from "./release-source-check.mjs";

function releaseNotes(version = "0.1.1") {
  return `# Changelog

## [${version}] - 2026-08-14

### Added
Added behavior.
### Changed
Changed behavior.
### Fixed
Fixed behavior.
### Security
Security boundary.
### Migration
Migration boundary.
### Known Limitations
Alpha limitation.
`;
}

function cleanSource() {
  const packages = [
    {
      id: "portmate 0.1.1",
      name: "portmate",
      version: "0.1.1",
      license: "Apache-2.0",
      authors: ["PortMate Contributors"],
    },
    {
      id: "portmate-core 0.1.1",
      name: "portmate-core",
      version: "0.1.1",
      license: "Apache-2.0",
      authors: ["PortMate Contributors"],
    },
    {
      id: "libssh-rs 0.3.8",
      name: "libssh-rs",
      version: "0.3.8",
      license: "MIT",
      authors: [],
    },
  ];
  return {
    packageJson: { name: "portmate", private: true, version: "0.1.1" },
    packageLock: {
      name: "portmate",
      version: "0.1.1",
      packages: { "": { name: "portmate", version: "0.1.1" } },
    },
    tauri: {
      productName: "PortMate",
      version: "0.1.1",
      identifier: "dev.portmate.desktop",
      bundle: {
        publisher: "PortMate Contributors",
        licenseFile: "../LICENSE",
        resources: {
          "../LICENSE": "LICENSE",
          "../THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt": "THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt",
        },
      },
    },
    cargoMetadata: {
      workspace_members: packages.map((entry) => entry.id),
      packages,
    },
    trackedFiles: new Set([
      "Cargo.lock",
      "CHANGELOG.md",
      "LICENSE",
      "package-lock.json",
      "package.json",
      "src-tauri/tauri.conf.json",
    ]),
    licenseText: "Apache License\nVersion 2.0, January 2004",
    fontLicenseText: "Copyright 2020 The JetBrains Mono Project Authors\nSIL OPEN FONT LICENSE Version 1.1",
    changelogText: releaseNotes(),
  };
}

describe("release source boundary", () => {
  it("accepts aligned application metadata while preserving upstream fork versions", () => {
    expect(findReleaseSourceViolations(cleanSource())).toEqual([]);
  });

  it("rejects JavaScript, Tauri, and PortMate Cargo version drift", () => {
    const source = cleanSource();
    source.packageLock.packages[""].version = "0.1.0";
    source.tauri.version = "0.2.0";
    source.cargoMetadata.packages[1].version = "0.1.0";
    expect(findReleaseSourceViolations(source)).toEqual([
      "Cargo package portmate-core version 0.1.0 does not match 0.1.1",
      "package-lock.json root package version 0.1.0 does not match 0.1.1",
      "src-tauri/tauri.conf.json version 0.2.0 does not match 0.1.1",
    ]);
  });

  it("rejects incomplete release notes, metadata, licenses, and untracked source", () => {
    const source = cleanSource();
    source.changelogText = releaseNotes().replace("### Security\nSecurity boundary.\n", "");
    source.tauri.identifier = "dev.portmate.app";
    source.cargoMetadata.packages[0].license = "MIT";
    source.fontLicenseText = "missing";
    source.trackedFiles.delete("Cargo.lock");
    expect(findReleaseSourceViolations(source)).toEqual([
      "CHANGELOG.md [0.1.1] must contain a non-empty Security section",
      "Cargo package portmate must use Apache-2.0",
      "JetBrains Mono must retain its SIL OFL 1.1 license",
      "Tauri identifier must be dev.portmate.desktop",
      "required release source file is not tracked: Cargo.lock",
    ]);
  });
});
