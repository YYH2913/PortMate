import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

export const MINIMUM_NODE_VERSION = Object.freeze({ major: 24, minor: 20, patch: 0 });

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
  if (!parsed || ![parsed.major, parsed.minor, parsed.patch].every(value => Number.isSafeInteger(value) && value >= 0)) {
    return false;
  }

  if (parsed.major !== minimum.major) return parsed.major > minimum.major;
  if (parsed.minor !== minimum.minor) return parsed.minor > minimum.minor;
  return parsed.patch >= minimum.patch;
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
