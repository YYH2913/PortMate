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

export async function runSwiftBuildWithRecovery({
  scratch,
  cache,
  build,
  onRetry,
  attempts = 3,
}) {
  if (!Number.isSafeInteger(attempts) || attempts < 1 || attempts > 5) {
    throw new Error("Swift package recovery attempts must be an integer from 1 to 5");
  }
  resetSwiftPackageState(scratch);
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await build();
    } catch (error) {
      if (!isRecoverableSwiftPackageFailure(error) || attempt === attempts) throw error;
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
      onRetry?.(error, { attempt, attempts });
    }
  }
  throw new Error("Swift package recovery exhausted without a build result");
}
