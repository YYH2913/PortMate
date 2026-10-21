const markerPrefix = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_", "ascii");
const windowsTypes = new Map([["msi", "MSI"], ["nsis", "NSS"], ["portable", "UNK"]]);
const referenceTypes = new Set(["UNK", "MSI", "NSS"]);

export function windowsBundleReference(reference, bundleType) {
  const expectedType = windowsTypes.get(bundleType);
  if (!expectedType) throw new Error(`Unsupported Windows bundle type: ${bundleType}`);
  const markerOffset = reference.indexOf(markerPrefix);
  if (markerOffset < 0 || reference.indexOf(markerPrefix, markerOffset + 1) !== -1) {
    throw new Error("Windows main reference must contain exactly one Tauri bundle-type marker");
  }
  const typeOffset = markerOffset + markerPrefix.length;
  const sourceType = reference.subarray(typeOffset, typeOffset + 3).toString("latin1");
  if (!referenceTypes.has(sourceType) || typeOffset + 3 > reference.length) {
    throw new Error(`Unsupported Windows reference bundle-type marker: ${sourceType}`);
  }
  // Tauri 2.11.4 patches this three-byte field for each package in turn;
  // the final release EXE therefore carries the last bundle's type.
  // Construct the exact package-specific reference and hash the whole file.
  const bytes = Buffer.from(reference);
  bytes.write(expectedType, typeOffset, 3, "ascii");
  return { bytes, markerOffset, sourceType, expectedType };
}
