export function parseSwiftVersion(output) {
  const match = /^(?:Apple )?Swift version (\d+)\.(\d+)(?:\.(\d+))?\b/.exec(output?.trim() ?? "");
  if (!match) return null;
  return match.slice(1, 4).map((part) => Number.parseInt(part ?? "0", 10));
}

export function swiftVersionIsAtLeast(output, minimum) {
  const current = parseSwiftVersion(output);
  const required = parseSemanticVersion(minimum);
  if (!current || !required) return false;
  for (let index = 0; index < 3; index += 1) {
    if (current[index] !== required[index]) return current[index] > required[index];
  }
  return true;
}

function parseSemanticVersion(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version ?? "");
  return match ? match.slice(1).map((part) => Number.parseInt(part, 10)) : null;
}
