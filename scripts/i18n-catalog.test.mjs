import { readFileSync, readdirSync } from "node:fs";
import { parse } from "@babel/parser";
import { describe, expect, it } from "vitest";

const locales = ["en", "zh", "ar", "fr", "ru", "es"];
const catalogs = Object.fromEntries(locales.map(locale => [locale, JSON.parse(readFileSync(new URL(`../src/locales/${locale}.json`, import.meta.url), "utf8"))]));
const placeholders = text => [...text.matchAll(/\{\d+\}/g)].map(match => match[0]).sort();

describe("translation catalog contract", () => {
  it("uses English message identifiers and preserves interpolation placeholders", () => {
    expect(Object.keys(catalogs.zh).sort()).toEqual(Object.keys(catalogs.en).sort());
    for (const [locale, catalog] of Object.entries(catalogs)) {
      expect(Object.keys(catalog).sort(), `${locale} must cover every application-owned message`).toEqual(Object.keys(catalogs.en).sort());
      for (const [key, value] of Object.entries(catalog)) {
        expect(key, `${locale}:${key}`).toMatch(/^[a-z0-9]+(?:-[a-z0-9]+)*$/);
        expect(Object.hasOwn(catalogs.en, key), `${locale}:${key} lacks an English fallback`).toBe(true);
        expect(typeof value, `${locale}:${key}`).toBe("string");
        expect(value.trim(), `${locale}:${key}`).not.toBe("");
        expect(placeholders(value), `${locale}:${key}`).toEqual(placeholders(catalogs.en[key]));
        for (const mnemonic of catalogs.en[key].match(/\([A-Z]\)/g) ?? []) {
          expect(value, `${locale}:${key} lost a keyboard mnemonic`).toContain(mnemonic);
        }
        expect(value, `${locale}:${key} contains a translation draft marker`).not.toMatch(/ZXQ\d*|QXZ|▁/);
      }
    }
  });

  it("resolves every literal translation call and keeps translated labels out of identifiers", () => {
    const issues = [];
    const isTranslation = node => node?.type === "CallExpression" && ["t", "tr", "translate"].includes(node.callee?.name);
    const containsTranslation = node => isTranslation(node)
      || node?.type === "TemplateLiteral" && node.expressions.some(containsTranslation)
      || node?.type === "BinaryExpression" && (containsTranslation(node.left) || containsTranslation(node.right));
    for (const file of readdirSync(new URL("../src/", import.meta.url)).filter(file => /\.tsx?$/.test(file) && !file.includes(".test."))) {
      const source = readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
      const ast = parse(source, { sourceType: "module", plugins: ["typescript", ...(file.endsWith(".tsx") ? ["jsx"] : [])] });
      function walk(node) {
        if (!node?.type) return;
        const report = message => issues.push(`${file}:${node.loc.start.line} ${message}`);
        if (node.type === "JSXElement" && node.openingElement.name.name === "option"
          && !node.openingElement.attributes.some(attribute => attribute.name?.name === "value")) {
          report("Options must declare a language-independent value instead of using their label");
        }
        if (isTranslation(node) && node.arguments[0]?.type === "StringLiteral" && !Object.hasOwn(catalogs.en, node.arguments[0].value)) {
          report(`Missing English message: ${node.arguments[0].value}`);
        }
        if (node.type === "JSXAttribute" && ["value", "id", "name", "key", "aria-controls", "aria-labelledby", "aria-describedby"].includes(node.name?.name)
          && containsTranslation(node.value?.expression)) report("Translated presentation used as a control identifier/value");
        if (node.type === "ObjectProperty" && ["value", "id", "source", "kind", "mode", "type"].includes(node.key?.name ?? node.key?.value)
          && isTranslation(node.value)) report("Translated presentation used as a data identifier/value");
        if (node.type === "BinaryExpression" && ["===", "!==", "==", "!="].includes(node.operator)
          && (isTranslation(node.left) || isTranslation(node.right))) report("Control flow depends on translated text");
        for (const [key, value] of Object.entries(node)) {
          if (key === "loc") continue;
          if (Array.isArray(value)) value.forEach(walk); else if (value?.type) walk(value);
        }
      }
      walk(ast);
    }
    expect(issues).toEqual([]);
  });
});
