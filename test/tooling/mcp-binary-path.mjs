import { isAbsolute, join, resolve } from "node:path";

export function resolveCargoTargetDirectory(projectRoot, environment = process.env) {
  const configured = environment.CARGO_TARGET_DIR;
  if (!configured) return resolve(projectRoot, "target");
  return isAbsolute(configured) ? resolve(configured) : resolve(projectRoot, configured);
}

export function resolveCargoProfileDirectory(projectRoot, profile, environment = process.env) {
  if (typeof profile !== "string" || !/^[A-Za-z0-9_-]+$/.test(profile)) {
    throw new Error("Cargo profile must be a simple directory name");
  }
  const target = environment.CARGO_BUILD_TARGET?.trim();
  const targetRoot = resolveCargoTargetDirectory(projectRoot, environment);
  return resolve(targetRoot, ...(target ? [target] : []), profile);
}

export function resolveMcpBinary(projectRoot, environment = process.env) {
  const configured = environment.PORTMATE_MCP_BINARY?.trim();
  if (configured) return resolve(configured);

  const target = environment.CARGO_BUILD_TARGET?.trim();
  const targetRoot = resolveCargoTargetDirectory(projectRoot, environment);
  const outputRoot = target ? join(targetRoot, target) : targetRoot;
  const executable = process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp";
  return join(outputRoot, "debug", executable);
}
