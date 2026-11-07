import { describe, expect, it, vi } from "vitest";
import { inspectTerminalWebglCanvases, inspectTerminalWebglContext, terminalWebglRendererIsSoftware } from "../../src/terminal-renderer";

describe("terminal renderer acceleration", () => {
  it("rejects known software rasterizers without disabling hardware GPUs", () => {
    for (const name of ["ANGLE (SwiftShader Device (Subzero))", "llvmpipe (LLVM 15)", "softpipe", "Mesa lavapipe", "Microsoft Basic Render Driver Direct3D11", "D3D11 WARP", "Software Rasterizer"]) {
      expect(terminalWebglRendererIsSoftware(name)).toBe(true);
    }
    for (const name of ["ANGLE (Intel Iris Xe, D3D11)", "Apple M3", "NVIDIA RTX 4070", "AMD Radeon", "WebKit WebGL"]) {
      expect(terminalWebglRendererIsSoftware(name)).toBe(false);
    }
  });
  it("inspects the context used by the actual xterm canvas, not only a preflight context", () => {
    const gl = {
      RENDERER: 0x1f01,
      VENDOR: 0x1f00,
      isContextLost: () => false,
      getExtension: (name: string) => name === "WEBGL_debug_renderer_info"
        ? { UNMASKED_RENDERER_WEBGL: 0x9246, UNMASKED_VENDOR_WEBGL: 0x9245 }
        : null,
      getParameter: (parameter: number) => parameter === 0x9246 ? "ANGLE (NVIDIA RTX 4070)" : "NVIDIA",
    } as unknown as WebGL2RenderingContext;
    expect(inspectTerminalWebglContext(gl)).toEqual({ support: "accelerated", renderer: "ANGLE (NVIDIA RTX 4070)", vendor: "NVIDIA" });
    expect(inspectTerminalWebglContext(null).support).toBe("unavailable");
    const lost = { ...gl, isContextLost: () => true } as unknown as WebGL2RenderingContext;
    expect(inspectTerminalWebglContext(lost).support).toBe("unavailable");
    const linkCanvas = { getContext: vi.fn(() => null) } as unknown as HTMLCanvasElement;
    const gpuCanvas = { getContext: vi.fn(() => gl) } as unknown as HTMLCanvasElement;
    expect(inspectTerminalWebglCanvases([linkCanvas, gpuCanvas]).support).toBe("accelerated");
    expect(linkCanvas.getContext).toHaveBeenCalledWith("webgl2");
    expect(gpuCanvas.getContext).toHaveBeenCalledWith("webgl2");
    const throwing = { isContextLost: () => false, getExtension: () => { throw Error("driver unavailable"); } } as unknown as WebGL2RenderingContext;
    expect(inspectTerminalWebglContext(throwing).support).toBe("unavailable");
  });
});
