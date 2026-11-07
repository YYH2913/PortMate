import assert from "node:assert/strict";

/** Simulated driver eligibility, real xterm WebGL addon and context lifecycle. */
export async function checkTerminalRendererLifecycle(page) {
  const host = page.locator('[data-pane-id="pane-a"] .terminal-host');
  await page.waitForFunction(() => [...document.querySelectorAll(".terminal-host")].every(host => host.dataset.terminalRenderer === "webgl"));
  await page.waitForTimeout(150); // Let the public device-pixel ResizeObserver settle.
  const instance = await host.getAttribute("data-terminal-instance-id");
  const context = await host.evaluate(element => {
    const gl = [...element.querySelectorAll(".xterm-screen canvas")].map(canvas => canvas.getContext("webgl2")).find(Boolean);
    return { attrs: gl.getContextAttributes(), renderer: element.dataset.terminalGpuRenderer, vendor: element.dataset.terminalGpuVendor,
      dpr: devicePixelRatio, canvasWidth: gl.canvas.width, cssWidth: gl.canvas.getBoundingClientRect().width };
  });
  assert.equal(context.renderer, "Test hardware WebGL path");
  assert.equal(await page.evaluate(() => window.__gpuStrictProbes), 0, "temporary probe contexts were created instead of validating the real renderer");
  assert.equal(context.attrs.preserveDrawingBuffer, false, "production GPU path unnecessarily preserved the back buffer");
  assert.equal(context.dpr, 2, "hardware lifecycle fixture did not exercise HiDPI");
  assert(context.canvasWidth >= context.cssWidth * 1.8, `GPU backing buffer did not follow HiDPI dimensions: ${JSON.stringify(context)}`);
  const setOpacity = async opacity => {
    await page.evaluate(value => {
      const session = window.__terminalCompatSessions.find(session => session.profile.id === "session-a");
      session.profile.terminal.backgroundOpacity = value;
      window.__emitTauriEvent("portmate-session-profile-updated", structuredClone(session));
    }, opacity);
    await page.waitForFunction(value => {
      const host = document.querySelector('[data-pane-id="pane-a"] .terminal-host');
      return host.dataset.terminalOpacity === String(value)
        && host.dataset.terminalRenderer === (value === 100 ? "webgl" : "dom");
    }, opacity);
  };
  const sendOutput = async (id, text) => page.evaluate(({ id, text }) => {
    const event = { id, sessionId: "session-a", paneId: "session-a:main", ts: "2026-11-07T05:00:00.123456Z", direction: "inbound", stream: "stdout", text, bytesRef: null, annotations: {} };
    const bytes = Array.from(new TextEncoder().encode(text));
    window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
  }, { id, text });
  const exportText = async () => page.evaluate(async () => {
    const { requestTerminalTextExport } = await import("/src/terminal-export-event.ts");
    return (await requestTerminalTextExport({ sessionId: "session-a", viewId: "view-a", source: "buffer" })).text;
  });
  await sendOutput("gpu-retained", "\r\nGPU-RETAINED-OUTPUT\r\n");
  await page.waitForTimeout(100);
  const before = await exportText();
  assert(before.includes("GPU-RETAINED-OUTPUT"));
  for (let index = 0; index < 3; index++) {
    await setOpacity(55);
    assert.equal(await host.locator(".xterm-screen canvas").count(), 0, "old GPU canvas survived DOM fallback");
    await setOpacity(100);
    const canvasCounts = await host.evaluate(element => ({
      all: element.querySelectorAll(".xterm-screen canvas").length,
      webgl: [...element.querySelectorAll(".xterm-screen canvas")].filter(canvas => canvas.getContext("webgl2")).length,
    }));
    assert.deepEqual(canvasCounts, { all: 2, webgl: 1 }, "GPU/link canvases leaked or were mistaken for each other");
  }
  assert.equal(await host.getAttribute("data-terminal-instance-id"), instance, "changing renderers recreated the terminal");
  assert.equal(await exportText(), before, "changing renderers modified retained output/timestamps");
  await host.evaluate(element => {
    const gl = [...element.querySelectorAll(".xterm-screen canvas")].map(canvas => canvas.getContext("webgl2")).find(Boolean);
    const extension = gl.getExtension("WEBGL_lose_context");
    if (!extension) throw Error("context-loss extension unavailable to lifecycle fixture");
    extension.loseContext();
  });
  await page.waitForFunction(() => document.querySelector('[data-pane-id="pane-a"] .terminal-host')?.dataset.terminalWebgl === "fallback-context-lost", undefined, { timeout: 10_000 });
  assert.equal(await host.locator(".xterm-screen canvas").count(), 0);
  await sendOutput("gpu-after-loss", "AFTER-GPU-CONTEXT-LOSS\r\n");
  await page.waitForTimeout(100);
  assert((await exportText()).includes("AFTER-GPU-CONTEXT-LOSS"), "lost GPU context stopped subsequent output");
  await host.locator(".xterm-helper-textarea").focus();
  await page.keyboard.type("x");
  await page.waitForFunction(() => window.__invokeCalls.some(call => call.command === "send_text" && call.args.text === "x"));
  console.log(JSON.stringify({ simulatedHardwareEligibility: true, strictProbeUnavailable: true, actualAddonContext: context, transparentCycles: 3, singleCanvas: true, retainedOutput: true, contextLossFallback: true, keyboardAfterLoss: true }));
}
