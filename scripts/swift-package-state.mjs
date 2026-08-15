import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";

const scratchStateEntries = [
  "checkouts",
  "repositories",
  "workspace-state.json",
  ".lock",
];

export function resetSwiftPackageState(scratch) {
  for (const entry of scratchStateEntries) {
    rmSync(join(scratch, entry), {
      recursive: true,
      force: true,
      maxRetries: 3,
      retryDelay: 100,
    });
  }
}

export function isRecoverableSwiftPackageFailure(error) {
  const message = error instanceof Error ? error.message : String(error);
  return /failed to clone repository/i.test(message)
    || /fatal: repository .* does not exist/i.test(message)
    || /working cop(?:y|ies).*does not exist/i.test(message)
    || /workspace-state\.json/i.test(message)
    || /repository cache.*(?:corrupt|invalid)/i.test(message);
}

export async function runSwiftBuildWithRecovery({ scratch, cache, build, onRetry }) {
  resetSwiftPackageState(scratch);
  try {
    return await build();
  } catch (error) {
    if (!isRecoverableSwiftPackageFailure(error)) throw error;
    rmSync(scratch, {
      recursive: true,
      force: true,
      maxRetries: 3,
      retryDelay: 100,
    });
    for (const entry of ["repositories", "manifests"]) {
      rmSync(join(cache, entry), {
        recursive: true,
        force: true,
        maxRetries: 3,
        retryDelay: 100,
      });
    }
    mkdirSync(cache, { recursive: true });
    onRetry?.(error);
    return build();
  }
}
