import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config.ts";

export default mergeConfig(viteConfig, defineConfig({
  test: {
    setupFiles: ["./scripts/test-locale-setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}", "scripts/**/*.test.mjs", "tmp/**/*.test.ts"],
    // The review reproducers under tmp/*.test.mjs are Node test-runner
    // programs, not Vitest suites. Keep the Vitest-only TypeScript
    // characterization files discoverable while avoiding false "no test
    // suite" failures from those standalone scripts.
    exclude: ["tmp/**/*.test.mjs", "node_modules/**", "target/**"],
  },
}));
