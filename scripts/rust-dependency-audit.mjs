import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export const ALLOWED_RUSTSEC_VULNERABILITIES = Object.freeze([
  "RUSTSEC-2023-0071:rsa@0.10.0-rc.18",
]);

export const REVIEWED_RUSTSEC_WARNINGS = Object.freeze([
  "unmaintained:RUSTSEC-2024-0413:atk@0.18.2",
  "unmaintained:RUSTSEC-2024-0416:atk-sys@0.18.2",
  "unmaintained:RUSTSEC-2025-0141:bincode@1.3.3",
  "unmaintained:RUSTSEC-2025-0057:fxhash@0.2.1",
  "unmaintained:RUSTSEC-2024-0412:gdk@0.18.2",
  "unmaintained:RUSTSEC-2024-0418:gdk-sys@0.18.2",
  "unmaintained:RUSTSEC-2024-0411:gdkwayland-sys@0.18.2",
  "unmaintained:RUSTSEC-2024-0417:gdkx11@0.18.2",
  "unmaintained:RUSTSEC-2024-0414:gdkx11-sys@0.18.2",
  "unmaintained:RUSTSEC-2024-0415:gtk@0.18.2",
  "unmaintained:RUSTSEC-2024-0420:gtk-sys@0.18.2",
  "unmaintained:RUSTSEC-2024-0419:gtk3-macros@0.18.2",
  "unmaintained:RUSTSEC-2024-0436:paste@1.0.15",
  "unmaintained:RUSTSEC-2024-0370:proc-macro-error@1.0.4",
  "unmaintained:RUSTSEC-2025-0081:unic-char-property@0.9.0",
  "unmaintained:RUSTSEC-2025-0075:unic-char-range@0.9.0",
  "unmaintained:RUSTSEC-2025-0080:unic-common@0.9.0",
  "unmaintained:RUSTSEC-2025-0100:unic-ucd-ident@0.9.0",
  "unmaintained:RUSTSEC-2025-0098:unic-ucd-version@0.9.0",
  "unsound:RUSTSEC-2024-0429:glib@0.18.5",
  "unsound:RUSTSEC-2026-0097:rand@0.7.3",
  "yanked:yanked:aes@0.9.0",
]);

export function validateRustDependencyAuditReport(report) {
  const vulnerabilityList = report?.vulnerabilities?.list;
  if (!Array.isArray(vulnerabilityList)) {
    throw new Error("cargo audit JSON is missing vulnerabilities.list");
  }
  const vulnerabilities = vulnerabilityList.map(vulnerabilityFingerprint);
  assertExactFindings(
    "RustSec vulnerability exceptions",
    vulnerabilities,
    ALLOWED_RUSTSEC_VULNERABILITIES,
  );

  if (!report?.warnings || typeof report.warnings !== "object" || Array.isArray(report.warnings)) {
    throw new Error("cargo audit JSON is missing warnings");
  }
  const warnings = Object.entries(report.warnings).flatMap(([kind, entries]) => {
    if (!Array.isArray(entries)) {
      throw new Error(`cargo audit warning category ${kind} is not an array`);
    }
    return entries.map((entry) => warningFingerprint(kind, entry));
  });
  assertExactFindings("reviewed RustSec warnings", warnings, REVIEWED_RUSTSEC_WARNINGS);

  return {
    vulnerabilityExceptions: vulnerabilities.length,
    reviewedWarnings: warnings.length,
  };
}

export function runRustDependencyAudit(options = {}) {
  const spawn = options.spawn ?? spawnSync;
  const result = spawn("cargo", ["audit", "--json"], {
    cwd: options.projectRoot ?? projectRoot,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    env: options.environment ?? process.env,
  });

  if (result.error) {
    throw new Error(`failed to execute cargo audit: ${result.error.message}`);
  }
  if (result.signal) {
    throw new Error(`cargo audit was terminated by ${result.signal}`);
  }
  if (result.status !== 0 && result.status !== 1) {
    throw new Error(
      `cargo audit failed with exit code ${String(result.status)}: ${boundedDiagnostic(result.stderr)}`,
    );
  }

  let report;
  try {
    report = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(
      `cargo audit did not return valid JSON: ${error.message}; ${boundedDiagnostic(result.stderr)}`,
    );
  }

  const summary = validateRustDependencyAuditReport(report);
  console.log(
    `Rust dependency audit passed (${summary.vulnerabilityExceptions} mitigated vulnerability exception, `
      + `${summary.reviewedWarnings} reviewed warnings)`,
  );
  return summary;
}

function vulnerabilityFingerprint(entry) {
  return `${requiredString(entry?.advisory?.id, "vulnerability advisory id")}:`
    + `${requiredString(entry?.package?.name, "vulnerability package")}`
    + `@${requiredString(entry?.package?.version, "vulnerability package version")}`;
}

function warningFingerprint(kind, entry) {
  const advisoryId = entry?.advisory?.id ?? "yanked";
  return `${requiredString(kind, "warning kind")}:`
    + `${requiredString(advisoryId, "warning advisory id")}:`
    + `${requiredString(entry?.package?.name, "warning package")}`
    + `@${requiredString(entry?.package?.version, "warning package version")}`;
}

function requiredString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`cargo audit JSON has an invalid ${label}`);
  }
  return value;
}

function assertExactFindings(label, actualEntries, expectedEntries) {
  const actual = new Set(actualEntries);
  const expected = new Set(expectedEntries);
  if (actual.size !== actualEntries.length) {
    throw new Error(`${label} contain duplicate findings`);
  }
  const unexpected = [...actual].filter((entry) => !expected.has(entry)).sort();
  const missing = [...expected].filter((entry) => !actual.has(entry)).sort();
  if (unexpected.length === 0 && missing.length === 0) return;

  const details = [];
  if (unexpected.length > 0) details.push(`unexpected: ${unexpected.join(", ")}`);
  if (missing.length > 0) details.push(`missing: ${missing.join(", ")}`);
  throw new Error(`${label} changed; ${details.join("; ")}`);
}

function boundedDiagnostic(value) {
  const diagnostic = typeof value === "string" ? value.trim() : "";
  if (!diagnostic) return "no diagnostic output";
  return diagnostic.length <= 2_000 ? diagnostic : `${diagnostic.slice(-2_000)} (truncated)`;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    runRustDependencyAudit();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
