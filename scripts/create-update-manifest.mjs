import { spawnSync } from "node:child_process";
import { lstatSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { verifyUpdateSignature } from "./update-signature.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const releases = "https://github.com/YYH2913/PortMate/releases/download";
const supportedTargets = new Set(["windows-x86_64", "windows-aarch64", "linux-x86_64", "linux-aarch64", "darwin-x86_64", "darwin-aarch64"]);

export function createUpdateManifest({ version, tag = version, target, notes, publishedAt, artifacts, publicKey, sign }) {
  if (!/^\d+\.\d+\.\d+$/.test(version) || ![version, `v${version}`].includes(tag)) throw new Error("Expected a stable release version and matching tag");
  if (!supportedTargets.has(target)) throw new Error(`Unsupported updater target: ${target}`);
  if (!Number.isFinite(Date.parse(publishedAt))) throw new Error("Invalid release publication time");
  const platforms = {};
  for (const file of artifacts) {
    const name = basename(file);
    let installer;
    if (target.startsWith("windows-") && /\.msi$/i.test(name)) installer = "msi";
    if (target.startsWith("windows-") && /-setup\.exe$/i.test(name)) installer = "nsis";
    if (target.startsWith("linux-") && /\.AppImage$/.test(name)) installer = "appimage";
    if (target.startsWith("darwin-") && /\.app\.tar\.gz$/.test(name)) installer = "app";
    if (!installer) continue;
    const key = `${target}-${installer}`;
    if (platforms[key]) throw new Error(`Multiple updater packages for ${key}`);
    const signature = sign(file, version).trim();
    verifyUpdateSignature(readFileSync(file), signature, publicKey, version);
    platforms[key] = { url: `${releases}/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`, signature };
  }
  if (!Object.keys(platforms).length) throw new Error("No compatible updater packages found");
  return { version, notes, pub_date: new Date(publishedAt).toISOString(), platforms };
}

export function mergeUpdateManifests(fragments) {
  if (!fragments.length) throw new Error("No updater manifests supplied");
  const first = fragments[0];
  const result = { ...first, platforms: {} };
  for (const fragment of fragments) {
    if (fragment.version !== first.version || fragment.notes !== first.notes || fragment.pub_date !== first.pub_date) throw new Error("Updater manifest release identities do not match");
    for (const [target, platform] of Object.entries(fragment.platforms)) {
      if (result.platforms[target]) throw new Error(`Duplicate updater target: ${target}`);
      result.platforms[target] = platform;
    }
  }
  return result;
}

function filesIn(path) {
  const metadata = lstatSync(path);
  if (metadata.isSymbolicLink()) return [];
  if (metadata.isFile()) return [path];
  return metadata.isDirectory() ? readdirSync(path).flatMap(name => filesIn(join(path, name))) : [];
}

function main() {
  const { values } = parseArgs({ options: {
    artifacts: { type: "string" }, target: { type: "string" }, notes: { type: "string" },
    "published-at": { type: "string" }, tag: { type: "string" }, output: { type: "string" },
    merge: { type: "string", multiple: true },
  } });
  if (!values.output) throw new Error("Supply --output for the generated manifest");
  let manifest;
  if (values.merge?.length) {
    manifest = mergeUpdateManifests(values.merge.map(path => JSON.parse(readFileSync(path, "utf8"))));
  } else {
    if (!values.artifacts || !values.target || !values.notes || !values["published-at"]) throw new Error("Supply --artifacts, --target, --notes and --published-at");
    if (!process.env.TAURI_SIGNING_PRIVATE_KEY && !process.env.TAURI_SIGNING_PRIVATE_KEY_PATH) throw new Error("A release signing key is required in TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH");
    const config = JSON.parse(readFileSync(join(projectRoot, "src-tauri", "tauri.conf.json"), "utf8"));
    manifest = createUpdateManifest({
      version: config.version, tag: values.tag, target: values.target,
      notes: readFileSync(values.notes, "utf8"), publishedAt: values["published-at"],
      publicKey: config.plugins.updater.pubkey, artifacts: filesIn(resolve(values.artifacts)),
      sign(file, version) {
        const result = spawnSync(process.execPath, [join(projectRoot, "node_modules", "@tauri-apps", "cli", "tauri.js"), "signer", "sign", "--app-version", version, file], {
          cwd: projectRoot, encoding: "utf8", env: process.env, windowsHide: true,
        });
        // Never forward signer output: release secrets must not reach CI logs.
        if (result.error || result.status !== 0) throw new Error(`Could not sign ${basename(file)} (exit ${result.status ?? "unknown"})`);
        return readFileSync(`${file}.sig`, "utf8");
      },
    });
  }
  writeFileSync(values.output, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`Created updater manifest with ${Object.keys(manifest.platforms).length} target(s): ${values.output}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
