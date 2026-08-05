import { describe, expect, it, vi } from "vitest";
import {
  ALLOWED_RUSTSEC_VULNERABILITIES,
  REVIEWED_RUSTSEC_WARNINGS,
  runRustDependencyAudit,
  validateRustDependencyAuditReport,
} from "./rust-dependency-audit.mjs";

describe("Rust dependency audit policy", () => {
  it("permits only the RSA advisory covered by the runtime blinding mitigation", () => {
    expect(ALLOWED_RUSTSEC_VULNERABILITIES).toEqual([
      "RUSTSEC-2023-0071:rsa@0.10.0-rc.18",
    ]);
    expect(validateRustDependencyAuditReport(reviewedReport())).toEqual({
      vulnerabilityExceptions: 1,
      reviewedWarnings: 22,
    });
  });

  it("rejects new vulnerabilities and changed warning versions", () => {
    const newVulnerability = reviewedReport();
    newVulnerability.vulnerabilities.list.push(vulnerability(
      "RUSTSEC-2099-0001",
      "unsafe-example",
      "1.0.0",
    ));
    expect(() => validateRustDependencyAuditReport(newVulnerability)).toThrow(
      "unexpected: RUSTSEC-2099-0001:unsafe-example@1.0.0",
    );

    const changedWarning = reviewedReport();
    changedWarning.warnings.unsound[0].package.version = "0.18.6";
    expect(() => validateRustDependencyAuditReport(changedWarning)).toThrow(
      "reviewed RustSec warnings changed",
    );
  });

  it("rejects stale exceptions and reviewed warnings that disappear", () => {
    const missingException = reviewedReport();
    missingException.vulnerabilities.list = [];
    expect(() => validateRustDependencyAuditReport(missingException)).toThrow(
      "missing: RUSTSEC-2023-0071:rsa@0.10.0-rc.18",
    );

    const missingWarning = reviewedReport();
    missingWarning.warnings.unmaintained.shift();
    expect(() => validateRustDependencyAuditReport(missingWarning)).toThrow(
      "reviewed RustSec warnings changed",
    );
  });

  it("accepts cargo-audit's expected vulnerability exit status after validating JSON", () => {
    const spawn = vi.fn(() => ({
      status: 1,
      signal: null,
      stdout: JSON.stringify(reviewedReport()),
      stderr: "",
    }));
    expect(runRustDependencyAudit({ spawn, projectRoot: "/repo", environment: {} })).toEqual({
      vulnerabilityExceptions: 1,
      reviewedWarnings: 22,
    });
    expect(spawn).toHaveBeenCalledWith("cargo", ["audit", "--json"], expect.objectContaining({
      cwd: "/repo",
      encoding: "utf8",
    }));
  });
});

function reviewedReport() {
  const warnings = {};
  for (const fingerprint of REVIEWED_RUSTSEC_WARNINGS) {
    const [kind, advisoryId, packageVersion] = fingerprint.split(":");
    const separator = packageVersion.lastIndexOf("@");
    const packageName = packageVersion.slice(0, separator);
    const version = packageVersion.slice(separator + 1);
    (warnings[kind] ??= []).push({
      package: { name: packageName, version },
      advisory: advisoryId === "yanked" ? null : { id: advisoryId },
    });
  }
  return {
    vulnerabilities: {
      found: true,
      count: 1,
      list: [vulnerability("RUSTSEC-2023-0071", "rsa", "0.10.0-rc.18")],
    },
    warnings,
  };
}

function vulnerability(id, packageName, version) {
  return {
    advisory: { id },
    package: { name: packageName, version },
  };
}
