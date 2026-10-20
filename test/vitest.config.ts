import { availableParallelism } from "node:os";
import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "../vite.config.ts";

export default mergeConfig(viteConfig, defineConfig({
  test: {
    // Leave CPU time for the runner, Vite transforms, and child-process fixtures
    // so parallel suites do not consume each other's wall-clock deadlines.
    maxWorkers: Math.max(1, Math.min(4, Math.floor(availableParallelism() / 2))),
    setupFiles: ["./test/setup/locale.ts"],
    include: ["test/frontend/**/*.{test,spec}.{ts,tsx}", "test/scripts/**/*.test.mjs"],
    // Local review reproducers under tmp/ are intentionally not part of the suite.
    exclude: ["node_modules/**", "target/**"],
  },
}));
