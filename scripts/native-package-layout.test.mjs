import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  verifyMacAppBundle,
  verifyMacBundleMetadata,
  verifyWindowsPackageLayout,
} from "./native-package-layout.mjs";

const temporaryRoots = [];
const macMetadata = {
  CFBundleIdentifier: "dev.portmate.desktop",
  CFBundleShortVersionString: "0.1.0",
  CFBundleVersion: "0.1.0",
  CFBundleExecutable: "portmate",
  LSApplicationCategoryType: "public.app-category.developer-tools",
};

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("native package layouts", () => {
  it("accepts a nested Windows installer payload", () => {
    const fixture = windowsFixture();

    const verified = verifyWindowsPackageLayout(fixture);

    expect(verified.applicationDirectory).toBe(fixture.applicationDirectory);
    expect(verified.sha256.main).toMatch(/^[a-f0-9]{64}$/);
  });

  it("rejects a missing Windows sidecar", () => {
    const fixture = windowsFixture();
    rmSync(join(fixture.applicationDirectory, "portmate-mcp.exe"));

    expect(() => verifyWindowsPackageLayout(fixture)).toThrow(
      /exactly one portmate-mcp\.exe regular file, found 0/,
    );
  });

  it("rejects a duplicate Windows sidecar", () => {
    const fixture = windowsFixture();
    write(join(fixture.root, "duplicate", "PORTMATE-MCP.EXE"), "sidecar");

    expect(() => verifyWindowsPackageLayout(fixture)).toThrow(
      /exactly one portmate-mcp\.exe regular file, found 2/,
    );
  });

  it("rejects a Windows license that differs from the repository license", () => {
    const fixture = windowsFixture();
    write(join(fixture.applicationDirectory, "LICENSE"), "different-license");

    expect(() => verifyWindowsPackageLayout(fixture)).toThrow(
      /Windows license SHA-256 does not match its reference/,
    );
  });

  it("rejects a missing Windows JetBrains Mono license", () => {
    const fixture = windowsFixture();
    rmSync(join(fixture.applicationDirectory, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"));

    expect(() => verifyWindowsPackageLayout(fixture)).toThrow(
      /exactly one JetBrainsMono-OFL\.txt regular file, found 0/,
    );
  });

  it("rejects a Windows JetBrains Mono license outside its resource directory", () => {
    const fixture = windowsFixture();
    rmSync(join(fixture.applicationDirectory, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"));
    write(join(fixture.applicationDirectory, "JetBrainsMono-OFL.txt"), "font-license");

    expect(() => verifyWindowsPackageLayout(fixture)).toThrow(
      /Expected Windows JetBrains Mono license at .*THIRD_PARTY_LICENSES.*JetBrainsMono-OFL\.txt/,
    );
  });

  it("accepts the expected macOS application bundle layout and metadata", () => {
    const fixture = macFixture();

    const verified = verifyMacAppBundle(fixture);

    expect(verified.app).toBe(fixture.app);
    expect(verified.metadata).toEqual(macMetadata);
  });

  it("rejects an unexpected macOS bundle identifier", () => {
    expect(() => verifyMacBundleMetadata(
      { ...macMetadata, CFBundleIdentifier: "dev.example.invalid" },
      macMetadata,
    )).toThrow(/Expected macOS CFBundleIdentifier=dev\.portmate\.desktop/);
  });

  it("requires macOS binaries to match their references unless signing was verified externally", () => {
    const fixture = macFixture();
    write(join(fixture.app, "Contents", "MacOS", "portmate"), "signed-main");

    expect(() => verifyMacAppBundle(fixture)).toThrow(
      /macOS main executable SHA-256 does not match its reference/,
    );
    expect(() => verifyMacAppBundle({ ...fixture, compareBinaries: false })).not.toThrow();
  });

  it("rejects a macOS license outside Contents/Resources", () => {
    const fixture = macFixture();
    rmSync(join(fixture.app, "Contents", "Resources", "LICENSE"));
    write(join(fixture.app, "Contents", "LICENSE"), "license");

    expect(() => verifyMacAppBundle(fixture)).toThrow(
      /Expected macOS license at .*Contents.*Resources.*LICENSE/,
    );
  });

  it("rejects a modified macOS JetBrains Mono license", () => {
    const fixture = macFixture();
    write(
      join(fixture.app, "Contents", "Resources", "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"),
      "modified-font-license",
    );

    expect(() => verifyMacAppBundle(fixture)).toThrow(
      /macOS JetBrains Mono license SHA-256 does not match its reference/,
    );
  });

  it.skipIf(process.platform === "win32")("rejects a package symlink that escapes its root", () => {
    const fixture = windowsFixture();
    symlinkSync("../../../sources/portmate.exe", join(fixture.applicationDirectory, "escaped-source"));

    expect(() => verifyWindowsPackageLayout(fixture)).toThrow(/symlink escapes its root/);
  });
});

function windowsFixture() {
  const root = temporaryRoot("portmate-windows-layout-");
  const sources = join(root, "sources");
  const packageRoot = join(root, "extracted");
  const applicationDirectory = join(packageRoot, "Program Files", "PortMate");
  const sourceMain = write(join(sources, "portmate.exe"), "main");
  const sourceSidecar = write(join(sources, "portmate-mcp.exe"), "sidecar");
  const sourceLicense = write(join(sources, "LICENSE"), "license");
  const sourceThirdPartyLicense = write(join(sources, "JetBrainsMono-OFL.txt"), "font-license");
  write(join(applicationDirectory, "portmate.exe"), "main");
  write(join(applicationDirectory, "portmate-mcp.exe"), "sidecar");
  write(join(applicationDirectory, "LICENSE"), "license");
  write(join(applicationDirectory, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"), "font-license");
  return {
    root: packageRoot,
    applicationDirectory,
    sourceMain,
    sourceSidecar,
    sourceLicense,
    sourceThirdPartyLicense,
  };
}

function macFixture() {
  const root = temporaryRoot("portmate-macos-layout-");
  const sources = join(root, "sources");
  const app = join(root, "PortMate.app");
  const sourceMain = write(join(sources, "portmate"), "main");
  const sourceSidecar = write(join(sources, "portmate-mcp"), "sidecar");
  const sourceLicense = write(join(sources, "LICENSE"), "license");
  const sourceThirdPartyLicense = write(join(sources, "JetBrainsMono-OFL.txt"), "font-license");
  write(join(app, "Contents", "MacOS", "portmate"), "main");
  write(join(app, "Contents", "MacOS", "portmate-mcp"), "sidecar");
  write(join(app, "Contents", "Resources", "LICENSE"), "license");
  write(
    join(app, "Contents", "Resources", "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"),
    "font-license",
  );
  write(join(app, "Contents", "Info.plist"), "plist");
  return {
    app,
    sourceMain,
    sourceSidecar,
    sourceLicense,
    sourceThirdPartyLicense,
    metadata: { ...macMetadata },
    expectedMetadata: macMetadata,
  };
}

function temporaryRoot(prefix) {
  const root = mkdtempSync(join(tmpdir(), prefix));
  temporaryRoots.push(root);
  return root;
}

function write(path, contents) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, contents);
  return path;
}
