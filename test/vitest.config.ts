import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "../vite.config.ts";

export default mergeConfig(viteConfig, defineConfig({
  test: {
    // Process/IPC fixtures use wall-clock deadlines. Avoid one worker per suite
    // competing with native builds on hosts that report many logical CPUs.
    maxWorkers: 4,
    setupFiles: ["./test/setup/locale.ts"],
    include: ["test/frontend/**/*.{test,spec}.{ts,tsx}", "test/scripts/**/*.test.mjs"],
    // Local review reproducers under tmp/ are intentionally not part of the suite.
    exclude: ["node_modules/**", "target/**"],
  },
}));
