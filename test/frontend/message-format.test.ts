import { describe, expect, it, vi } from "vitest";
import { formatTranslatedMessage } from "../../src/message-format";
import { translate } from "../../src/i18n";
import type { Locale } from "../../src/i18n";

const reference = (template: string, values: readonly unknown[]) => template.replace(/\{(\d+)\}/g, (placeholder, index: string) => (
  Number(index) < values.length ? String(values[Number(index)] ?? "") : placeholder
));

describe("compiled localized message formatting", () => {
  it("keeps null, missing, repeated/reordered arguments and literal braces unchanged", () => {
    for (const template of ["plain", "{0}{1}{0}", "{01}-{0}-{2}-{9}", "{{0}} {bad} {-1} {1.1}", "{9999999999999999999999999999999999999999999999}", "{0} {1} {2} {3}"]) {
      for (const values of [[], [null], [undefined, false, 0, ""], ["{1}", "界🙂", "<script>"], ["$&", "$1", "\\r"]]) {
        expect(formatTranslatedMessage(template, values)).toBe(reference(template, values));
      }
    }
  });
  it("never caches interpolation values and safely evicts old templates", () => {
    expect(formatTranslatedMessage("{0} secret", ["first"])).toBe("first secret");
    for (let n = 0; n < 2000; n++) expect(formatTranslatedMessage(`template-${n} {0}`, [n])).toBe(`template-${n} ${n}`);
    expect(formatTranslatedMessage("{0} secret", ["second"])).toBe("second secret");
  });
  it("preserves byte-cell labels for all six languages without interpolating values twice", () => {
    const markerValues = ["RX", 1234, "0A", "\\n {1}"];
    for (const locale of ["zh", "en", "fr", "es", "ru", "ar"] as Locale[]) {
      for (const key of ["offset-hex", "offset-ascii-hex", "decimal-offset"]) {
        const template = translate(key, [], locale);
        expect(translate(key, markerValues, locale)).toBe(reference(template, markerValues));
      }
    }
  });
  it("does not retain an arbitrary fallback message as a compiled template", () => {
    const scan = vi.spyOn(String.prototype, "matchAll");
    try {
      for (const value of ["one", "two"]) expect(translate("uncatalogued-server-message-{0}", [value], "en")).toBe(`uncatalogued-server-message-${value}`);
      expect(scan.mock.calls.filter(([pattern]) => String(pattern) === "/\\{(\\d+)\\}/g")).toHaveLength(2);
    } finally { scan.mockRestore(); }
  });
});
