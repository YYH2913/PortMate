import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  verifyWindowsReleaseBinary,
  verifyWindowsSidecarBinary,
} from "./windows-release-binary.mjs";

const temporaryRoots = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("Windows release binary verification", () => {
  it("accepts an x86-64 GUI executable with embedded frontend entry assets", () => {
    const fixture = releaseFixture();
    writeFileSync(fixture.executable, fakePe({
      subsystem: 2,
      strings: fixture.assets.map((asset) => `/${asset}`),
    }));

    const result = verifyWindowsReleaseBinary(fixture);

    expect(result.architecture).toBe("x86_64");
    expect(result.subsystem).toBe("windows-gui");
    expect(result.frontendAssets).toEqual(fixture.assets);
  });

  it("rejects a development executable that does not embed the frontend", () => {
    const fixture = releaseFixture();
    writeFileSync(fixture.executable, fakePe({ subsystem: 2 }));

    expect(() => verifyWindowsReleaseBinary(fixture)).toThrow(
      /build it with Tauri production mode instead of cargo build --release/,
    );
  });

  it("rejects a console subsystem for the main desktop executable", () => {
    const fixture = releaseFixture();
    writeFileSync(fixture.executable, fakePe({
      subsystem: 3,
      strings: fixture.assets.map((asset) => `/${asset}`),
    }));

    expect(() => verifyWindowsReleaseBinary(fixture)).toThrow(/subsystem 3, expected 2/);
  });

  it("accepts an x86-64 console MCP sidecar", () => {
    const root = temporaryRoot();
    const executable = join(root, "portmate-mcp.exe");
    writeFileSync(executable, fakePe({ subsystem: 3 }));

    expect(verifyWindowsSidecarBinary(executable).subsystem).toBe("windows-console");
  });
});

function releaseFixture() {
  const root = temporaryRoot();
  const frontendDist = join(root, "dist");
  const assetsRoot = join(frontendDist, "assets");
  const assets = ["assets/index-test.js", "assets/index-test.css"];
  mkdirSync(assetsRoot, { recursive: true });
  writeFileSync(
    join(frontendDist, "index.html"),
    '<script type="module" src="/assets/index-test.js"></script>\n'
      + '<link rel="stylesheet" href="/assets/index-test.css">\n',
  );
  writeFileSync(join(assetsRoot, "index-test.js"), "export {};\n");
  writeFileSync(join(assetsRoot, "index-test.css"), "body {}\n");
  return {
    executable: join(root, "portmate.exe"),
    frontendDist,
    assets,
  };
}

function temporaryRoot() {
  const root = mkdtempSync(join(tmpdir(), "portmate-windows-release-test-"));
  temporaryRoots.push(root);
  return root;
}

function fakePe({ subsystem, strings = [], dll = false, machine = 0x8664 }) {
  const peOffset = 0x80;
  const optionalHeaderBytes = 0xf0;
  const header = Buffer.alloc(peOffset + 24 + optionalHeaderBytes);
  header.writeUInt16LE(0x5a4d, 0);
  header.writeUInt32LE(peOffset, 0x3c);
  header.writeUInt32LE(0x0000_4550, peOffset);
  header.writeUInt16LE(machine, peOffset + 4);
  header.writeUInt16LE(optionalHeaderBytes, peOffset + 20);
  header.writeUInt16LE(0x0022 | (dll ? 0x2000 : 0), peOffset + 22);
  header.writeUInt16LE(0x20b, peOffset + 24);
  header.writeUInt16LE(subsystem, peOffset + 24 + 68);
  return Buffer.concat([header, Buffer.from(`\0${strings.join("\0")}\0`, "utf8")]);
}
