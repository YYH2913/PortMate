export type TerminalWebglSupport = "accelerated" | "software" | "unavailable";

export function terminalWebglRendererIsSoftware(renderer: string): boolean {
  return /swiftshader|llvmpipe|softpipe|lavapipe|software\s*(?:raster|render|adapter)|microsoft basic render|\bwarp\b/i.test(renderer);
}

/** A short-lived probe; software WebGL can block a terminal's entire UI thread. */
export function probeTerminalWebgl(createCanvas = () => document.createElement("canvas")): TerminalWebglSupport {
  let gl: WebGL2RenderingContext | null = null;
  try {
    gl = createCanvas().getContext("webgl2", { failIfMajorPerformanceCaveat: true });
    if (!gl) return "unavailable";
    const info = gl.getExtension("WEBGL_debug_renderer_info");
    const renderer = String(gl.getParameter(info?.UNMASKED_RENDERER_WEBGL ?? gl.RENDERER) ?? "");
    return terminalWebglRendererIsSoftware(renderer) ? "software" : "accelerated";
  } catch {
    return "unavailable";
  } finally {
    // Release the probe immediately rather than spending a context per pane.
    try { gl?.getExtension("WEBGL_lose_context")?.loseContext(); } catch { /* Driver already lost its context. */ }
  }
}
