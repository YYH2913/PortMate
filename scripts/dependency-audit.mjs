import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const blockedSeverities = new Set(["moderate", "high", "critical"]);
const knownSeverities = new Set(["info", "low", ...blockedSeverities]);

export function validateNpmDependencyAuditReport(report) {
  if (!report?.vulnerabilities || typeof report.vulnerabilities !== "object"
    || Array.isArray(report.vulnerabilities)) {
    throw new Error("npm audit JSON is missing vulnerabilities");
  }

  const findings = Object.entries(report.vulnerabilities).map(([name, finding]) => {
    if (!finding || typeof finding !== "object" || Array.isArray(finding)) {
      throw new Error(`npm audit returned an invalid finding for ${name}`);
    }
    const severity = finding.severity;
    if (typeof severity !== "string" || !knownSeverities.has(severity)) {
      throw new Error(`npm audit returned an invalid severity for ${name}`);
    }
    return { name, severity };
  });

  const blocked = findings.filter((finding) => blockedSeverities.has(finding.severity));
  if (blocked.length > 0) {
    const details = blocked
      .sort((left, right) => left.name.localeCompare(right.name))
      .map((finding) => `${finding.name} (${finding.severity})`)
      .join(", ");
    throw new Error(`npm dependency audit found blocked vulnerabilities: ${details}`);
  }

  return {
    findings: findings.length,
    allowedLowOrInfo: findings.length,
  };
}

export function runNpmDependencyAudit(options = {}) {
  const spawn = options.spawn ?? spawnSync;
  const command = options.command ?? (process.platform === "win32" ? "npm.cmd" : "npm");
  const result = spawn(command, ["audit", "--json"], {
    cwd: options.projectRoot ?? projectRoot,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    env: options.environment ?? process.env,
  });

  if (result.error) {
    throw new Error(`failed to execute npm audit: ${result.error.message}`);
  }
  if (result.signal) {
    throw new Error(`npm audit was terminated by ${result.signal}`);
  }
  if (result.status !== 0 && result.status !== 1) {
    throw new Error(
      `npm audit failed with exit code ${String(result.status)}: ${boundedDiagnostic(result.stderr)}`,
    );
  }

  let report;
  try {
    report = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(
      `npm audit did not return valid JSON: ${error.message}; ${boundedDiagnostic(result.stderr)}`,
    );
  }

  const summary = validateNpmDependencyAuditReport(report);
  console.log(
    `npm dependency audit passed (${summary.findings} allowed low/info findings)`,
  );
  return summary;
}

function boundedDiagnostic(value) {
  const diagnostic = typeof value === "string" ? value.trim() : "";
  if (!diagnostic) return "no diagnostic output";
  return diagnostic.length <= 2_000 ? diagnostic : `${diagnostic.slice(-2_000)} (truncated)`;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    runNpmDependencyAudit();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
