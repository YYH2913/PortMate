import { createHash, randomUUID } from "node:crypto";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const script = fileURLToPath(import.meta.url);
const root = resolve(dirname(script), "..");
export const elevationManifest = JSON.parse(await readFile(join(root, "src-tauri/shell-elevation-manifest.json"), "utf8"));
const maxBytes = 16 * 1024 * 1024;

export function verifyElevationArchive(bytes) {
  if (!bytes.length || bytes.length > maxBytes || createHash("sha256").update(bytes).digest("hex") !== elevationManifest.sha256) {
    throw new Error("Bundled Windows elevation helper integrity check failed");
  }
}

export async function prepareElevationHelper({ target, archive, cache = join(root, "src-tauri/binaries/shell-elevation"), fetchArchive = fetch } = {}) {
  if (!target?.includes("windows")) return null;
  if (!Object.hasOwn(elevationManifest.targets, target)) throw new Error(`Unsupported Windows elevation target: ${target}`);
  const destination = join(cache, "gsudo.portable.zip");
  let bytes;
  if (archive) {
    bytes = await readFile(resolve(archive));
  } else {
    try {
      bytes = await readFile(destination);
      verifyElevationArchive(bytes);
      return destination;
    } catch (error) {
      if (error.code && error.code !== "ENOENT") throw error;
    }
    const response = await fetchArchive(elevationManifest.url, { signal: AbortSignal.timeout(180_000) });
    if (!response.ok) throw new Error(`Windows elevation helper download failed: HTTP ${response.status}`);
    if (Number(response.headers.get("content-length")) > maxBytes) throw new Error("Elevation archive exceeds 16 MiB");
    const chunks = [];
    let size = 0;
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > maxBytes) throw new Error("Elevation archive exceeds 16 MiB");
      chunks.push(chunk);
    }
    bytes = Buffer.concat(chunks);
  }
  verifyElevationArchive(bytes);
  await mkdir(cache, { recursive: true });
  const staged = `${destination}.${randomUUID()}.tmp`;
  try {
    await writeFile(staged, bytes, { flag: "wx" });
    await rename(staged, destination);
  } finally {
    await rm(staged, { force: true });
  }
  return destination;
}

if (process.argv[1] && resolve(process.argv[1]) === script) {
  const { values } = parseArgs({ options: { target: { type: "string" }, archive: { type: "string" } } });
  try {
    const probe = spawnSync("rustc", ["-vV"], { encoding: "utf8", windowsHide: true });
    const target = values.target || process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET || probe.stdout?.match(/^host: (.+)$/m)?.[1];
    if (!target) throw new Error("Cannot determine elevation helper target");
    const path = await prepareElevationHelper({ target, archive: values.archive });
    console.log(path ? `Verified bundled gsudo ${elevationManifest.version} for ${target}` : "Windows elevation helper is not required for this target");
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
