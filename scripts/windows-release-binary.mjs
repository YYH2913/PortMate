import { lstatSync, readFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve, sep } from "node:path";

const PE_MACHINE_AMD64 = 0x8664;
const PE32_PLUS_MAGIC = 0x20b;
const PE_CHARACTERISTIC_DLL = 0x2000;
const PE_SUBSYSTEM_WINDOWS_GUI = 2;
const PE_SUBSYSTEM_WINDOWS_CONSOLE = 3;

export function verifyWindowsReleaseBinary({ executable, frontendDist }) {
  const pe = verifyWindowsPeFile({
    path: executable,
    label: "Windows main executable",
    expectedSubsystem: PE_SUBSYSTEM_WINDOWS_GUI,
    expectedDll: false,
  });
  const frontendAssets = verifyEmbeddedFrontend(executable, frontendDist);
  return { ...pe, frontendAssets };
}

export function verifyWindowsSidecarBinary(executable) {
  return verifyWindowsPeFile({
    path: executable,
    label: "Windows MCP sidecar",
    expectedSubsystem: PE_SUBSYSTEM_WINDOWS_CONSOLE,
    expectedDll: false,
  });
}

export function verifyWindowsLoaderDll(path) {
  return verifyWindowsPeFile({
    path,
    label: "Windows WebView2 loader",
    expectedSubsystem: PE_SUBSYSTEM_WINDOWS_CONSOLE,
    expectedDll: true,
  });
}

export function verifyWindowsPeFile({ path, label, expectedSubsystem, expectedDll }) {
  const binary = readFileSync(path);
  if (binary.length < 0x40 || binary.readUInt16LE(0) !== 0x5a4d) {
    throw new Error(`${label} is not a DOS/PE executable: ${path}`);
  }

  const peOffset = binary.readUInt32LE(0x3c);
  requireBytes(binary, peOffset, 24, label);
  if (binary.readUInt32LE(peOffset) !== 0x0000_4550) {
    throw new Error(`${label} has an invalid PE signature: ${path}`);
  }

  const machine = binary.readUInt16LE(peOffset + 4);
  if (machine !== PE_MACHINE_AMD64) {
    throw new Error(
      `${label} must target Windows x86-64 (machine 0x${PE_MACHINE_AMD64.toString(16)}), found 0x${machine.toString(16)}`,
    );
  }

  const optionalHeaderBytes = binary.readUInt16LE(peOffset + 20);
  const characteristics = binary.readUInt16LE(peOffset + 22);
  const optionalHeaderOffset = peOffset + 24;
  requireBytes(binary, optionalHeaderOffset, optionalHeaderBytes, label);
  if (optionalHeaderBytes < 70 || binary.readUInt16LE(optionalHeaderOffset) !== PE32_PLUS_MAGIC) {
    throw new Error(`${label} is not a PE32+ x86-64 executable: ${path}`);
  }

  const subsystem = binary.readUInt16LE(optionalHeaderOffset + 68);
  if (subsystem !== expectedSubsystem) {
    throw new Error(
      `${label} has PE subsystem ${subsystem}, expected ${expectedSubsystem}`,
    );
  }
  const dll = (characteristics & PE_CHARACTERISTIC_DLL) !== 0;
  if (dll !== expectedDll) {
    throw new Error(`${label} DLL characteristic is ${dll}, expected ${expectedDll}`);
  }

  return {
    path: resolve(path),
    bytes: binary.length,
    architecture: "x86_64",
    format: "PE32+",
    subsystem: subsystem === PE_SUBSYSTEM_WINDOWS_GUI ? "windows-gui" : "windows-console",
    dll,
  };
}

export function verifyEmbeddedFrontend(executable, frontendDist) {
  const root = resolve(frontendDist);
  const indexPath = join(root, "index.html");
  const index = readFileSync(indexPath, "utf8");
  const entryAssets = [...index.matchAll(/\b(?:src|href)=["']([^"'?#]+)["']/g)]
    .map((match) => match[1].replace(/^\.\//, "").replace(/^\//, ""))
    .filter((path) => path.startsWith("assets/"));
  const uniqueAssets = [...new Set(entryAssets)];
  if (!uniqueAssets.some((path) => path.endsWith(".js"))
      || !uniqueAssets.some((path) => path.endsWith(".css"))) {
    throw new Error(`Production frontend entry assets are missing from ${indexPath}`);
  }

  for (const asset of uniqueAssets) {
    const assetPath = resolve(root, asset);
    const relativePath = relative(root, assetPath);
    if (isAbsolute(relativePath) || relativePath === ".." || relativePath.startsWith(`..${sep}`)) {
      throw new Error(`Frontend entry asset escapes its distribution root: ${asset}`);
    }
    if (!lstatSync(assetPath).isFile()) {
      throw new Error(`Frontend entry asset is not a regular file: ${assetPath}`);
    }
  }

  const binary = readFileSync(executable);
  const missingAssets = uniqueAssets.filter(
    (asset) => !binary.includes(Buffer.from(`/${asset}`, "utf8")),
  );
  if (missingAssets.length > 0) {
    throw new Error(
      `Windows release executable does not embed frontend entry assets (${missingAssets.join(", ")}); build it with Tauri production mode instead of cargo build --release`,
    );
  }
  return uniqueAssets;
}

function requireBytes(binary, offset, bytes, label) {
  if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(bytes)
      || offset < 0 || bytes < 0 || offset > binary.length - bytes) {
    throw new Error(`${label} has a truncated PE header`);
  }
}
