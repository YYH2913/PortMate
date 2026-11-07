import { describe, expect, it, vi } from "vitest";
import { probeTerminalWebgl, terminalWebglRendererIsSoftware } from "../../src/terminal-renderer";

describe("terminal renderer acceleration", () => {
  it("rejects known software rasterizers without disabling hardware GPUs", () => {
    for (const name of ["ANGLE (SwiftShader Device (Subzero))", "llvmpipe (LLVM 15)", "softpipe", "Mesa lavapipe", "Microsoft Basic Render Driver Direct3D11", "D3D11 WARP", "Software Rasterizer"]) {
      expect(terminalWebglRendererIsSoftware(name)).toBe(true);
    }
    for (const name of ["ANGLE (Intel Iris Xe, D3D11)", "Apple M3", "NVIDIA RTX 4070", "AMD Radeon", "WebKit WebGL"]) {
      expect(terminalWebglRendererIsSoftware(name)).toBe(false);
    }
  });
  it("requests acceleration and releases both software and hardware probe contexts", () => {
    for (const [renderer, expected] of [["SwiftShader", "software"], ["Apple M3", "accelerated"]]) {
      const loseContext = vi.fn();
      const getContext = vi.fn(() => ({
        RENDERER: 0x1f01,
        getExtension: (name: string) => name === "WEBGL_debug_renderer_info" ? { UNMASKED_RENDERER_WEBGL: 0x9246 } : { loseContext },
        getParameter: () => renderer,
      }));
      expect(probeTerminalWebgl(() => ({ getContext }) as unknown as HTMLCanvasElement)).toBe(expected);
      expect(getContext).toHaveBeenCalledWith("webgl2", { failIfMajorPerformanceCaveat: true });
      expect(loseContext).toHaveBeenCalledOnce();
    }
  });
  it("falls back safely when an accelerated context is unavailable or throws", () => {
    expect(probeTerminalWebgl(() => ({ getContext: () => null }) as unknown as HTMLCanvasElement)).toBe("unavailable");
    expect(probeTerminalWebgl(() => { throw Error("driver unavailable"); })).toBe("unavailable");
  });
});
