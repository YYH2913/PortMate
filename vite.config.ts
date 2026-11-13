import { fileURLToPath } from "node:url";
import { defineConfig, normalizePath } from "vite";
import react from "@vitejs/plugin-react";

const localReviewTree = `${normalizePath(fileURLToPath(new URL("./tmp", import.meta.url)))}/**`;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // A cold optimizer must not crawl Cargo/SDK artifacts or local review copies.
  // Keep the application and browser-test entries discoverable explicitly.
  optimizeDeps: {
    entries: ["index.html", "test/fixtures/**/*.html"],
  },
  build: {
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            {
              name: "xterm-core",
              test: /node_modules[\\/]@xterm[\\/](?!addon-webgl[\\/])/,
              priority: 10,
            },
          ],
        },
      },
    },
  },
  server: {
    strictPort: true,
    port: 1420,
    host: "127.0.0.1",
    watch: {
      ignored: ["**/target/**", "**/ref/**", "**/dist/**", localReviewTree],
      usePolling: true,
      interval: 250,
    },
  },
});
