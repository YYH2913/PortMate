export type TerminalWebglSupport = "accelerated" | "software" | "unavailable";

export type TerminalWebglContextInfo = {
  support: TerminalWebglSupport;
  renderer: string;
  vendor: string;
};

export function terminalWebglRendererIsSoftware(renderer: string): boolean {
  return /swiftshader|llvmpipe|softpipe|lavapipe|software\s*(?:raster|render|adapter)|microsoft basic render|\bwarp\b/i.test(renderer);
}

export function inspectTerminalWebglContext(
  gl: WebGL2RenderingContext | null | undefined,
): TerminalWebglContextInfo {
  if (!gl) return { support: "unavailable", renderer: "", vendor: "" };
  try {
    if (gl.isContextLost()) return { support: "unavailable", renderer: "", vendor: "" };
    const info = gl.getExtension("WEBGL_debug_renderer_info");
    const renderer = String(gl.getParameter(info?.UNMASKED_RENDERER_WEBGL ?? gl.RENDERER) ?? "");
    const vendor = String(gl.getParameter(info?.UNMASKED_VENDOR_WEBGL ?? gl.VENDOR) ?? "");
    return {
      support: terminalWebglRendererIsSoftware(renderer) ? "software" : "accelerated",
      renderer,
      vendor,
    };
  } catch {
    return { support: "unavailable", renderer: "", vendor: "" };
  }
}

export function inspectTerminalWebglCanvases(canvases: Iterable<HTMLCanvasElement>): TerminalWebglContextInfo {
  // The addon appends a 2D hyperlink-underlining canvas before its WebGL
  // canvas. The first canvas is not necessarily the terminal renderer.
  for (const canvas of canvases) {
    const gl = canvas.getContext("webgl2");
    if (gl) return inspectTerminalWebglContext(gl);
  }
  return inspectTerminalWebglContext(null);
}
