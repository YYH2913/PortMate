import { createHash, randomUUID } from "node:crypto";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const script = fileURLToPath(import.meta.url);
const root = resolve(dirname(script), "..");
const maxArchiveBytes = 64 * 1024 * 1024;
export const easyTierManifest = JSON.parse(await readFile(join(root, "src-tauri/easytier-manifest.json"), "utf8"));

export function easyTierRelease(target) {
  const release = easyTierManifest.releases[easyTierManifest.targets[target]];
  if (!release) throw new Error(`Bundled EasyTier does not support ${target}`);
  return release;
}

export function verifyEasyTierArchive(bytes, release) {
  if (!bytes.length || bytes.length > maxArchiveBytes
      || createHash("sha256").update(bytes).digest("hex") !== release.sha256) {
    throw new Error(`EasyTier archive integrity check failed: ${release.archive}`);
  }
}

export async function prepareEasyTier({ target, archive, cache = join(root, "src-tauri/binaries/easytier"), fetchArchive = fetch } = {}) {
  const release = easyTierRelease(target);
  const destination = join(cache, release.archive);
  let bytes;
  if (archive) {
    bytes = await readFile(resolve(archive));
    verifyEasyTierArchive(bytes, release);
  } else {
    try {
      bytes = await readFile(destination);
      verifyEasyTierArchive(bytes, release);
      console.log(`Bundled EasyTier ${easyTierManifest.version} verified for ${target} (cached)`);
      return destination;
    } catch (error) {
      if (error.code && error.code !== "ENOENT") throw error;
      // A corrupt cache is replaced only after a fresh download passes validation.
    }
    const url = `https://github.com/EasyTier/EasyTier/releases/download/v${easyTierManifest.version}/${release.archive}`;
    console.log(`Preparing bundled EasyTier ${easyTierManifest.version} for ${target}...`);
    const response = await fetchArchive(url, { signal: AbortSignal.timeout(180_000) });
    if (!response.ok) throw new Error(`EasyTier download failed: HTTP ${response.status}`);
    if (Number(response.headers.get("content-length")) > maxArchiveBytes) throw new Error("EasyTier archive exceeds 64 MiB");
    const chunks = [];
    let size = 0;
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > maxArchiveBytes) throw new Error("EasyTier archive exceeds 64 MiB");
      chunks.push(chunk);
    }
    bytes = Buffer.concat(chunks);
    verifyEasyTierArchive(bytes, release);
  }
  await mkdir(cache, { recursive: true });
  const staged = `${destination}.${randomUUID()}.tmp`;
  try {
    await writeFile(staged, bytes, { flag: "wx" });
    await rename(staged, destination);
  } finally {
    await rm(staged, { force: true });
  }
  console.log(`Bundled EasyTier ${easyTierManifest.version} ready for ${target}`);
  return destination;
}

function hostTarget() {
  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8", windowsHide: true });
  const host = result.stdout?.match(/^host: (.+)$/m)?.[1]?.trim();
  if (result.status !== 0 || !host) throw new Error("Unable to determine the Rust host target");
  return host;
}

if (process.argv[1] && resolve(process.argv[1]) === script) {
  const { values } = parseArgs({ options: { target: { type: "string" }, archive: { type: "string" } } });
  try {
    await prepareEasyTier({ target: values.target || process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET || hostTarget(), archive: values.archive });
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
