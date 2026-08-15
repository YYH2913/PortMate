import { describe, expect, it, vi } from "vitest";
import {
  runNpmDependencyAudit,
  validateNpmDependencyAuditReport,
} from "./dependency-audit.mjs";

describe("npm dependency audit policy", () => {
  it("accepts clean reports and low-severity findings", () => {
    expect(validateNpmDependencyAuditReport({ vulnerabilities: {} })).toEqual({
      findings: 0,
      allowedLowOrInfo: 0,
    });
    expect(validateNpmDependencyAuditReport({
      vulnerabilities: {
        optionalPackage: { severity: "low" },
      },
    })).toEqual({ findings: 1, allowedLowOrInfo: 1 });
  });

  it.each(["moderate", "high", "critical"])(
    "rejects %s vulnerabilities even when npm exits successfully",
    (severity) => {
      const spawn = vi.fn(() => ({
        status: 0,
        signal: null,
        stdout: JSON.stringify({
          vulnerabilities: { nanoid: { severity } },
        }),
        stderr: "",
      }));
      expect(() => runNpmDependencyAudit({ spawn, command: "npm" })).toThrow(
        `nanoid (${severity})`,
      );
      expect(spawn).toHaveBeenCalledWith("npm", ["audit", "--json"], expect.objectContaining({
        timeout: 120_000,
        windowsHide: true,
      }));
    },
  );

  it("rejects malformed audit reports", () => {
    expect(() => validateNpmDependencyAuditReport({ vulnerabilities: [] })).toThrow(
      "missing vulnerabilities",
    );
    expect(() => validateNpmDependencyAuditReport({
      vulnerabilities: { packageName: { severity: "unknown" } },
    })).toThrow("invalid severity");
  });

  it("rejects audit process failures", () => {
    const spawn = vi.fn(() => ({
      status: 2,
      signal: null,
      stdout: "",
      stderr: "registry unavailable",
    }));
    expect(() => runNpmDependencyAudit({ spawn, command: "npm" })).toThrow(
      "registry unavailable",
    );
  });
});
