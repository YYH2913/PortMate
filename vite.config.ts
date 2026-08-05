import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
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
      ignored: ["**/target/**", "**/ref/**", "**/dist/**"],
      usePolling: true,
      interval: 250,
    },
  },
});
