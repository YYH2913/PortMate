export function cargoLockPinsPackage(lockText, expectedName, expectedVersion) {
  if (typeof lockText !== "string" || !expectedName || !expectedVersion) return false;
  const normalized = lockText.replace(/\r\n?/g, "\n");
  const packages = normalized.split(/^\s*\[\[package\]\]\s*$/m).slice(1);
  return packages.some((entry) => {
    const fields = new Map();
    for (const match of entry.matchAll(/^\s*([A-Za-z0-9_-]+)\s*=\s*"([^"]*)"\s*$/gm)) {
      fields.set(match[1], match[2]);
    }
    return fields.get("name") === expectedName && fields.get("version") === expectedVersion;
  });
}
