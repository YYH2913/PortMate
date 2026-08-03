import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const forbiddenPackages = new Set([
  "db-keystore",
  "keyring",
  "libmimalloc-sys",
  "mimalloc",
]);
const providerTargets = new Map([
  [
    "dbus-secret-service-keyring-store",
    'cfg(any(target_os = "linux", target_os = "freebsd"))',
  ],
  ["apple-native-keyring-store", 'cfg(target_os = "macos")'],
  ["windows-native-keyring-store", "cfg(windows)"],
]);
const sharedDependencies = new Set(["keyring-core", ...providerTargets.keys()]);

export function findNativeKeyringDependencyViolations(metadata) {
  const violations = [];
  const packages = Array.isArray(metadata?.packages) ? metadata.packages : [];
  const workspaceMembers = new Set(metadata?.workspace_members ?? []);
  const workspacePackages = packages.filter((entry) => workspaceMembers.has(entry.id));

  for (const entry of packages) {
    if (forbiddenPackages.has(entry.name) || entry.name.startsWith("turso")) {
      violations.push(`forbidden package is present in the resolved graph: ${entry.name}`);
    }
  }

  for (const name of ["portmate", "portmate-mcp"]) {
    const entry = workspacePackages.find((candidate) => candidate.name === name);
    if (!entry) {
      violations.push(`workspace package is missing: ${name}`);
      continue;
    }
    const dependencies = new Set(entry.dependencies.map((dependency) => dependency.name));
    if (!dependencies.has("portmate-keyring")) {
      violations.push(`${name} must depend on portmate-keyring`);
    }
    for (const provider of providerTargets.keys()) {
      if (dependencies.has(provider)) {
        violations.push(`${name} must not depend directly on ${provider}`);
      }
    }
  }

  const shared = workspacePackages.find((entry) => entry.name === "portmate-keyring");
  if (!shared) {
    violations.push("workspace package is missing: portmate-keyring");
    return violations.sort();
  }
  const dependencyByName = new Map(
    shared.dependencies.map((dependency) => [dependency.name, dependency]),
  );
  for (const dependency of shared.dependencies) {
    if (!sharedDependencies.has(dependency.name)) {
      violations.push(`portmate-keyring has an unexpected dependency: ${dependency.name}`);
    }
  }
  if (!dependencyByName.has("keyring-core")) {
    violations.push("portmate-keyring must depend on keyring-core");
  }
  for (const [provider, expectedTarget] of providerTargets) {
    const dependency = dependencyByName.get(provider);
    if (!dependency) {
      violations.push(`portmate-keyring is missing provider: ${provider}`);
    } else if (dependency.target !== expectedTarget) {
      violations.push(
        `${provider} target changed: expected ${expectedTarget}, got ${dependency.target ?? "all targets"}`,
      );
    }
  }

  return violations.sort();
}

function main() {
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--locked", "--format-version", "1"], {
      cwd: projectRoot,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    }),
  );
  const violations = findNativeKeyringDependencyViolations(metadata);
  if (violations.length > 0) {
    throw new Error(`native keyring dependency boundary failed:\n- ${violations.join("\n- ")}`);
  }
  console.log("PortMate native keyring dependency boundary passed");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
