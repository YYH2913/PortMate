import { describe, expect, it } from "vitest";
import { windowsBundleReference } from "../../scripts/windows-bundle-reference.mjs";

const prefix = "__TAURI_BUNDLE_TYPE_VAR_";
const image = type => Buffer.from(`MZ...code...${prefix}${type}\xc0\0...assets...`, "latin1");

describe("Tauri Windows bundle-specific binary references", () => {
  it.each(["UNK", "MSI", "NSS"])("constructs installer and portable variants from %s without modifying the source", sourceType => {
    const source = image(sourceType);
    const snapshot = Buffer.from(source);
    for (const [bundleType, expectedType] of [["msi", "MSI"], ["nsis", "NSS"], ["portable", "UNK"]]) {
      const result = windowsBundleReference(source, bundleType);
      expect(result.bytes).toEqual(image(expectedType));
      expect(result.sourceType).toBe(sourceType);
      expect(result.expectedType).toBe(expectedType);
      expect(result.markerOffset).toBe(source.indexOf(prefix));
      expect(source).toEqual(snapshot);
    }
  });

  it.each([Buffer.from("missing marker"), Buffer.from(`${prefix}NSS${prefix}MSI`)])("rejects an absent or ambiguous marker", source => {
    expect(() => windowsBundleReference(source, "msi")).toThrow("exactly one");
  });

  it.each(["", "M", "DEB", "BAD", "\xcdSI"])("rejects unsupported or truncated marker bytes: %j", type => {
    expect(() => windowsBundleReference(Buffer.from(`${prefix}${type}`, "latin1"), "msi")).toThrow("marker");
  });

  it.each([undefined, "MSI", "zip", "__proto__"])("requires an explicit supported installer type: %j", type => {
    expect(() => windowsBundleReference(image("NSS"), type)).toThrow("bundle type");
  });
});
