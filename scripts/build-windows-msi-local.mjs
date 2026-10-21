import { createHash, randomUUID } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { verifyWindowsPackageLayout } from "./native-package-layout.mjs";
import { verifyWindowsLoaderDll, verifyWindowsReleaseBinary, verifyWindowsSidecarBinary } from "./windows-release-binary.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { values } = parseArgs({ options: {
  template: { type: "string" }, "template-sha256": { type: "string" },
  "tools-root": { type: "string" }, output: { type: "string" },
  "skip-build": { type: "boolean", default: false },
} });
if (process.platform !== "linux") throw new Error("This local MSI rebuild path requires Linux msitools and gcab");
if (!values.template || !/^[a-f0-9]{64}$/i.test(values["template-sha256"] ?? "")) {
  throw new Error("Supply a verified native MSI with --template and its --template-sha256");
}
const template = resolve(values.template);
if (sha256(template) !== values["template-sha256"].toLowerCase()) throw new Error("Native MSI template checksum mismatch");
const tools = values["tools-root"] ? resolve(values["tools-root"]) : null;
const env = tools ? {
  ...process.env,
  LD_LIBRARY_PATH: [join(tools, "usr/lib/x86_64-linux-gnu"), process.env.LD_LIBRARY_PATH].filter(Boolean).join(":"),
} : process.env;
const tool = (name) => tools ? join(tools, "usr/bin", name) : name;
const config = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
const targetRoot = resolve(root, process.env.CARGO_TARGET_DIR || "target");
const releaseRoot = join(targetRoot, "x86_64-pc-windows-gnu/release");
const artifactRoot = join(targetRoot, "release-artifacts");
const commit = run("git", ["rev-parse", "HEAD"], { text: true }).trim();
const output = resolve(values.output || join(artifactRoot, `PortMate_${config.version}_local-${commit.slice(0, 8)}_x64_en-US.msi`));
if (output === template) throw new Error("MSI output must not overwrite its verified template");

const originalTables = run(tool("msiinfo"), ["tables", template], { text: true }).trim().split(/\r?\n/);
if (originalTables.some((name) => /Digital(Signature|Certificate)/.test(name))) {
  throw new Error("Signed MSI templates must be rebuilt and re-signed on the native release toolchain");
}
const properties = table(template, "Property");
const property = (name) => properties.rows.find((row) => row.Property === name)?.Value;
if (property("ProductName") !== config.productName || property("Manufacturer") !== config.bundle.publisher
    || property("ProductVersion") !== config.version) {
  throw new Error("The native MSI template must match the application identity and exact version");
}
const summary = run(tool("msiinfo"), ["suminfo", template], { text: true });
if (!/^Template: x64;/m.test(summary)) throw new Error("The native MSI template must target Windows x64");
const upgrade = table(template, "Upgrade");
if (!upgrade.rows.some((row) => row.UpgradeCode === property("UpgradeCode") && row.ActionProperty === "WIX_UPGRADE_DETECTED")) {
  throw new Error("Native MSI upgrade detection is missing");
}

if (!values["skip-build"]) {
  run(process.execPath, [join(root, "scripts/build-windows-portable.mjs")], { inherit: true });
}
const sourceMain = join(releaseRoot, "portmate.exe");
const sourceSidecar = join(releaseRoot, "portmate-mcp.exe");
const sourceLoader = join(releaseRoot, "WebView2Loader.dll");
const frontend = verifyWindowsReleaseBinary({ executable: sourceMain, frontendDist: join(root, "dist") });
verifyWindowsSidecarBinary(sourceSidecar);
verifyWindowsLoaderDll(sourceLoader);
const updateAsset = readdirSync(join(root, "dist/assets")).find((name) => /^UpdateDialog-.+\.js$/.test(name));
if (!updateAsset || !readFileSync(sourceMain).includes(Buffer.from(`/assets/${updateAsset}`))) {
  throw new Error("The current update-dialog frontend chunk is not embedded in the Windows executable");
}

mkdirSync(artifactRoot, { recursive: true });
const work = mkdtempSync(join(artifactRoot, ".local-msi-"));
const cabinetRoot = join(work, "cabinet");
const extractedRoot = join(work, "extracted");
mkdirSync(cabinetRoot);
mkdirSync(extractedRoot);
const preparedMain = join(work, "portmate.exe");
// GNU builds retain the other bundle-type comparison strings as well as the
// mutable marker. Only the unique UNK initializer identifies this fresh build.
const unknownMarker = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_UNK");
const patched = { bytes: Buffer.from(readFileSync(sourceMain)), markerOffset: 0 };
patched.markerOffset = patched.bytes.indexOf(unknownMarker);
if (patched.markerOffset < 0 || patched.bytes.indexOf(unknownMarker, patched.markerOffset + 1) >= 0) {
  throw new Error("The fresh Windows GNU build must contain exactly one unknown bundle marker");
}
patched.bytes.write("MSI", patched.markerOffset + unknownMarker.length - 3, "ascii");
const checksumOffset = patched.bytes.readUInt32LE(0x3c) + 24 + 64;
const sourceChecksum = readFileSync(sourceMain).readUInt32LE(checksumOffset);
if (sourceChecksum !== 0 && peChecksum(readFileSync(sourceMain), checksumOffset) !== sourceChecksum) {
  throw new Error("The source Windows executable has an invalid PE checksum");
}
patched.bytes.writeUInt32LE(peChecksum(patched.bytes, checksumOffset), checksumOffset);
writeFileSync(preparedMain, patched.bytes);
const sources = new Map([
  ["portmate.exe", preparedMain], ["portmate-mcp.exe", sourceSidecar],
  ["webview2loader.dll", sourceLoader], ["license", join(root, "LICENSE")],
  ["jetbrainsmono-ofl.txt", join(root, "THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt")],
]);
const files = table(template, "File");
const components = table(template, "Component");
const features = table(template, "FeatureComponents");
const hashes = table(template, "MsiFileHash");
const media = table(template, "Media");
const names = files.rows.map((row) => longName(row.FileName));
if (files.rows.some((row) => !/^[A-Za-z_][A-Za-z0-9_.]{0,71}$/.test(row.File))
    || new Set(names).size !== names.length || names.some((name) => !sources.has(name))
    || !["portmate.exe", "portmate-mcp.exe", "license", "jetbrainsmono-ofl.txt"].every((name) => names.includes(name))) {
  throw new Error("The native MSI template has an unexpected file layout");
}
if (media.rows.length !== 1 || media.rows[0].Cabinet !== "#app.cab") throw new Error("Expected one embedded MSI cabinet");
for (const row of files.rows.filter((row) => ["portmate.exe", "portmate-mcp.exe", "license"].includes(longName(row.FileName)))) {
  if (components.rows.find((component) => component.Component === row.Component_)?.Directory_ !== "INSTALLDIR") {
    throw new Error("Native MSI application files must share INSTALLDIR");
  }
}
if (!names.includes("webview2loader.dll")) {
  const id = "PortMateWebView2Loader";
  if (components.rows.some((row) => row.Component === id)) throw new Error("MSI loader component ID collision");
  components.rows.push({ Component: id, ComponentId: stableGuid(property("UpgradeCode"), "INSTALLDIR/WebView2Loader.dll"), Directory_: "INSTALLDIR", Attributes: "256", Condition: "", KeyPath: id });
  features.rows.push({ Feature_: "MainProgram", Component_: id });
  files.rows.push({ File: id, Component_: id, FileName: "wv2load.dll|WebView2Loader.dll", FileSize: "0", Version: "", Language: "", Attributes: "512", Sequence: String(Math.max(...files.rows.map((row) => Number(row.Sequence))) + 1) });
}
for (const row of files.rows) {
  const source = sources.get(longName(row.FileName));
  const bytes = readFileSync(source);
  copyFileSync(source, join(cabinetRoot, row.File));
  row.FileSize = String(bytes.length);
  if (longName(row.FileName) === "portmate.exe") row.Version = `${config.version}.0`;
  hashes.rows = hashes.rows.filter((hash) => hash.File_ !== row.File);
  if (!row.Version) {
    const hash = createHash("md5").update(bytes).digest();
    hashes.rows.push({ File_: row.File, Options: "0", ...Object.fromEntries([0, 1, 2, 3].map((index) => [`HashPart${index + 1}`, String(hash.readInt32LE(index * 4))])) });
  }
}
media.rows[0].LastSequence = String(Math.max(...files.rows.map((row) => Number(row.Sequence))));
const productCode = guid();
const packageCode = guid();
properties.rows.find((row) => row.Property === "ProductCode").Value = productCode;
const cabinet = join(work, "app.cab");
run(tool("gcab"), ["-c", "-z", cabinet, ...files.rows.toSorted((a, b) => Number(a.Sequence) - Number(b.Sequence)).map((row) => row.File)], { cwd: cabinetRoot });
const rebuilt = join(work, "PortMate.msi");
copyFileSync(template, rebuilt);
for (const [name, data] of Object.entries({ Property: properties, File: files, Component: components, FeatureComponents: features, MsiFileHash: hashes, Media: media })) {
  const path = join(work, `${name}.idt`);
  writeFileSync(path, [...data.header, ...data.rows.map((row) => data.columns.map((key) => row[key] ?? "").join("\t"))].join("\r\n") + "\r\n");
  run(tool("msibuild"), [rebuilt, "-i", path]);
}
run(tool("msibuild"), [rebuilt, "-a", "app.cab", cabinet, "-s", "PortMate", config.bundle.publisher, "x64;1033", packageCode]);
const changed = new Set(["_SummaryInformation", "Property", "File", "Component", "FeatureComponents", "MsiFileHash", "Media"]);
for (const name of originalTables.filter((name) => !changed.has(name))) {
  const original = run(tool("msiinfo"), ["export", template, name], { text: true, cwd: work });
  const current = run(tool("msiinfo"), ["export", rebuilt, name], { text: true, cwd: work });
  if (original !== current) throw new Error(`Unexpected MSI table change: ${name}`);
}
const streams = run(tool("msiinfo"), ["streams", template], { text: true }).trim().split(/\r?\n/);
for (const stream of streams.filter((name) => name !== "app.cab" && !name.endsWith("SummaryInformation"))) {
  const original = run(tool("msiinfo"), ["extract", template, stream]);
  const current = run(tool("msiinfo"), ["extract", rebuilt, stream]);
  if (!original.length || !original.equals(current)) throw new Error(`Unexpected MSI UI/resource stream change: ${stream}`);
}
run(tool("msiextract"), ["-C", extractedRoot, rebuilt]);
const payload = verifyWindowsPackageLayout({ root: extractedRoot, sourceMain: preparedMain, sourceSidecar, sourceLicense: sources.get("license"), sourceThirdPartyLicense: sources.get("jetbrainsmono-ofl.txt") });
const loader = join(dirname(payload.main), "WebView2Loader.dll");
if (!existsSync(loader) || sha256(loader) !== sha256(sourceLoader)) throw new Error("Extracted MSI loader does not match the Windows x64 source");
verifyWindowsReleaseBinary({ executable: payload.main, frontendDist: join(root, "dist") });
verifyWindowsLoaderDll(loader);
mkdirSync(dirname(output), { recursive: true });
copyFileSync(rebuilt, output);
const record = {
  version: config.version, sourceCommit: commit, target: "x86_64-pc-windows-gnu",
  archive: output, archiveBytes: readFileSync(output).length, archiveSha256: sha256(output),
  template, templateSha256: sha256(template), productCode, packageCode, upgradeCode: property("UpgradeCode"),
  preservedNativeInstallerUi: true, preservedUpgradeRules: true, frontend, updateAsset,
  bundleType: "msi", bundleMarkerOffset: patched.markerOffset,
  payload, loaderSha256: sha256(loader), sourceMainSha256: sha256(sourceMain),
  windowsNativeRuntimeTested: false,
};
writeFileSync(`${output}.sha256`, `${record.archiveSha256}  ${basename(output)}\n`);
writeFileSync(`${output}.build.json`, JSON.stringify(record, null, 2) + "\n");
console.log(JSON.stringify(record, null, 2));

function longName(value) { return value.split("|").at(-1).toLowerCase(); }
function sha256(path) { return createHash("sha256").update(readFileSync(path)).digest("hex"); }
function guid() { return `{${randomUUID().toUpperCase()}}`; }
function stableGuid(namespace, name) {
  const hash = createHash("sha1").update(Buffer.from(namespace.replace(/[{}-]/g, ""), "hex")).update(name).digest().subarray(0, 16);
  hash[6] = (hash[6] & 15) | 0x50; hash[8] = (hash[8] & 63) | 0x80;
  const hex = hash.toString("hex").toUpperCase();
  return `{${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}}`;
}
function peChecksum(bytes, offset) {
  let sum = 0;
  for (let position = 0; position < bytes.length; position += 2) {
    if (position >= offset && position < offset + 4) continue;
    sum += bytes[position] | ((bytes[position + 1] || 0) << 8);
    sum = (sum & 0xffff) + (sum >>> 16);
  }
  sum = (sum & 0xffff) + (sum >>> 16);
  return (sum + bytes.length) >>> 0;
}
function table(msi, name) {
  const lines = run(tool("msiinfo"), ["export", msi, name], { text: true }).split(/\r?\n/).filter((line) => line.length);
  const columns = lines[0].split("\t");
  return { columns, header: lines.slice(0, 3), rows: lines.slice(3).map((line) => Object.fromEntries(columns.map((key, index) => [key, line.split("\t")[index] ?? ""]))) };
}
function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: options.cwd || root, env, encoding: options.text ? "utf8" : undefined, maxBuffer: 256 * 1024 * 1024, stdio: options.inherit ? "inherit" : "pipe" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${basename(command)} failed (${result.status}): ${String(result.stderr || "").slice(-4_000)}`);
  return result.stdout;
}
