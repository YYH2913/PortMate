import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

export const MINIMUM_NODE_VERSION = Object.freeze({ major: 22, minor: 12, patch: 0 });

export function parseNodeVersion(version) {
  if (typeof version !== "string") return null;
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/.exec(version.trim());
  if (!match) return null;

  const [major, minor, patch] = match.slice(1).map(Number);
  if (![major, minor, patch].every(Number.isSafeInteger)) return null;
  return { major, minor, patch };
}

export function supportsNodeVersion(version, minimum = MINIMUM_NODE_VERSION) {
  const parsed = typeof version === "string" ? parseNodeVersion(version) : version;
  if (!parsed || !Number.isSafeInteger(parsed.major) || !Number.isSafeInteger(parsed.minor)) {
    return false;
  }

  if (parsed.major !== minimum.major) return parsed.major > minimum.major;
  return parsed.minor >= minimum.minor;
}

export function assertSupportedNodeVersion(version = process.versions.node) {
  if (supportsNodeVersion(version)) return;
  const current = typeof version === "string" && version.trim() ? version : "unknown";
  throw new Error(
    `PortMate requires Node >=${MINIMUM_NODE_VERSION.major}.${MINIMUM_NODE_VERSION.minor}.${MINIMUM_NODE_VERSION.patch}; current runtime is ${current}. Run \`nvm use\` before running this command.`,
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assertSupportedNodeVersion();
}
