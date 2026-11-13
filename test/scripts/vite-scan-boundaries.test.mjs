import { describe, expect, it } from "vitest";
import { resolve } from "node:path";
import config from "../../vite.config.ts";

describe("cold Vite scan boundaries", () => {
  it("scans only the app and explicit browser fixtures, not ignored artifact trees", () => {
    expect(config.optimizeDeps.entries).toEqual(["index.html", "test/fixtures/**/*.html"]);
    for (const directory of ["target", "ref", "dist"]) {
      expect(config.server.watch.ignored).toContain(`**/${directory}/**`);
    }
    expect(config.server.watch.ignored).toContain(`${resolve(import.meta.dirname, "../../tmp").replaceAll("\\", "/")}/**`);
    // Do not disable all file watching when the checkout itself lives in /tmp.
    expect(config.server.watch.ignored).not.toContain("**/tmp/**");
    expect(config.server.watch.usePolling).toBe(true);
  });
});
