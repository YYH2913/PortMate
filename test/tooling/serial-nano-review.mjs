import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import path from "node:path";

/** Capture real nano output without touching any connected serial device. */
async function captureNano(term, cwd) {
  const child = spawn("script", ["-qfec", "stty rows 24 cols 80; exec nano --ignorercfiles nano-fixture.txt", "/dev/null"], {
    cwd,
    env: { ...process.env, TERM: term, LANG: "C", LC_ALL: "C" },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const output = [];
  const errors = [];
  let size = 0;
  let failure = null;
  const data = await new Promise((resolve, reject) => {
    const quit = setTimeout(() => { child.stdin.end("\x18"); }, 700);
    const deadline = setTimeout(() => {
      failure = new Error(`nano ${term} PTY capture timed out`);
      child.kill("SIGTERM");
    }, 5_000);
    const cleanup = () => { clearTimeout(quit); clearTimeout(deadline); };
    child.stdin.on("error", () => {});
    const collect = (chunks, chunk) => {
      size += chunk.length;
      if (size > 1024 * 1024) {
        failure = new Error(`nano ${term} PTY capture exceeded 1 MiB`);
        child.kill("SIGTERM");
      } else chunks.push(chunk);
    };
    child.stdout.on("data", chunk => collect(output, chunk));
    child.stderr.on("data", chunk => collect(errors, chunk));
    child.once("error", error => { cleanup(); reject(error); });
    child.once("close", (code, signal) => {
      cleanup();
      if (failure) return reject(failure);
      if (code !== 0) return reject(new Error(`nano ${term} exited with ${code ?? signal}: ${Buffer.concat(errors)}`));
      resolve(Buffer.concat(output).toString("utf8"));
    });
  });
  assert(data.includes("GNU nano"), `nano ${term} did not draw its TUI`);
  assert(!data.includes("\x1b[3J") && !data.includes("\x1bc"), `nano ${term} explicitly erased history or reset the terminal`);
  return data;
}

/** Diagnostic: compare standard alt-screen and no-alt-screen nano output. */
export async function checkSerialNanoBehavior(context, appUrl) {
  const scratchDir = path.join(process.cwd(), "tmp");
  await mkdir(scratchDir, { recursive: true });
  const fixtureDir = await mkdtemp(path.join(scratchDir, "serial-nano-review-"));
  const results = [];
  try {
    await writeFile(path.join(fixtureDir, "nano-fixture.txt"), "PORTMATE-NANO-FIXTURE\n");
    const captures = [];
    for (const term of ["xterm-256color", "linux", "vt100"]) {
      captures.push({ term, data: await captureNano(term, fixtureDir) });
    }
    for (const detached of [false, true]) for (const capture of captures) {
      const page = await context.newPage();
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      // Inspect the real public xterm buffers; expose a handle in the test's
      // served module only, not in production source.
      await page.route("**/src/TerminalCanvas.tsx", async route => {
        const response = await route.fetch();
        await route.fulfill({ response, body: (await response.text()).replace(
          "termRef.current = term;", "termRef.current = term; window.__nanoReviewTerminal = term;",
        ) });
      });
      try {
        await page.goto(detached
          ? `${appUrl}?detachedPane=1&windowId=serial-nano&paneId=serial-nano-pane&viewId=serial-nano-view&sessionId=bench-uart&title=&color=&keyMode=remote`
          : appUrl);
        if (!detached) await page.locator(".tree-session", { hasText: "Bench UART" }).click();
        const host = page.locator('.terminal-canvas[data-terminal-focused="true"] .terminal-host');
        await page.waitForFunction(() => window.__nanoReviewTerminal
          && document.querySelector('.terminal-canvas[data-terminal-focused="true"] .terminal-host')?.dataset.terminalReady === "true");
        await page.evaluate(() => { window.__nanoReviewTerminal.resize(80, 24); });
        const instance = await host.getAttribute("data-terminal-instance-id");
        const emit = (text, split = false) => page.evaluate(({ text, split }) => {
          // Split each ANSI sequence across raw packets for the detached case.
          for (const chunk of split ? [...text] : [text]) {
            const event = { id: crypto.randomUUID(), sessionId: "bench-uart", paneId: "bench-uart:main",
              ts: new Date().toISOString(), direction: "inbound", stream: "stdout", text: chunk, bytesRef: null, annotations: {} };
            const bytes = [...new TextEncoder().encode(chunk)];
            window.__events.push(event);
            window.__emitTauriEvent("portmate-terminal-live", { event, bytes, originalLength: bytes.length, truncated: false });
            window.__emitTauriEvent("portmate-session-event", event);
          }
        }, { text, split });
        const readNormal = () => page.evaluate(() => {
          const buffer = window.__nanoReviewTerminal.buffer.normal;
          return Array.from({ length: buffer.length }, (_, index) => buffer.getLine(index)?.translateToString(true) ?? "").join("\n");
        });
        await emit("\x1b[?1049l\x1b[2J\x1b[3J\x1b[H"
          + Array.from({ length: 80 }, (_, index) => `OLDER-SERIAL-ROW-${index}\r\n`).join("")
          + Array.from({ length: 12 }, (_, index) => `RECENT-SERIAL-ROW-${index}\r\n`).join("")
          + "$ nano nano-fixture.txt");
        await page.waitForFunction(() => {
          const b = window.__nanoReviewTerminal.buffer.normal;
          return b.getLine(b.baseY + b.cursorY)?.translateToString(true).includes("$ nano nano-fixture.txt");
        });
        const before = await readNormal();
        assert(before.includes("OLDER-SERIAL-ROW-0") && before.includes("RECENT-SERIAL-ROW-11"));
        await emit(capture.data + "\r\n$ AFTER-NANO> ", detached);
        await page.waitForFunction(() => {
          const b = window.__nanoReviewTerminal.buffer.active;
          return b.type === "normal" && b.getLine(b.baseY + b.cursorY)?.translateToString(true).includes("$ AFTER-NANO> ");
        });
        const after = await readNormal();
        const alternate = capture.data.includes("\x1b[?1049h") && capture.data.includes("\x1b[?1049l");
        const retainedRecentRows = (after.match(/RECENT-SERIAL-ROW-\d+/g) ?? []).length;
        assert(after.includes("OLDER-SERIAL-ROW-0"), "nano erased unrelated prior scrollback");
        if (alternate) {
          assert.equal(retainedRecentRows, 12, "nano alternate-screen exit lost the normal screen");
          assert(after.includes("$ nano nano-fixture.txt"));
          assert(!after.includes("PORTMATE-NANO-FIXTURE"), "nano alternate buffer leaked into normal history");
        } else {
          assert.equal(retainedRecentRows, 0, "no-alt nano diagnostic did not reproduce viewport erasure");
          assert(!after.includes("$ nano nano-fixture.txt"));
          assert(capture.data.includes("\x1b[H\x1b[J"), "no-alt nano did not send full-viewport ED0");
        }
        assert.equal(await host.getAttribute("data-terminal-instance-id"), instance, "nano exit recreated the terminal");
        const serialMutations = await page.evaluate(() => window.__invokeCalls.filter(call =>
          ["send_text", "send_bytes", "run_command", "resize_session", "save_session_profile"].includes(call.command)
          && (call.args.sessionId === "bench-uart" || call.args.profile?.id === "bench-uart")));
        assert.deepEqual(serialMutations, [], "review must not write to the serial device or its profile");
        assert.deepEqual(errors, []);
        results.push({ detached, term: capture.term, alternate, retainedRecentRows, olderScrollbackRetained: true,
          terminalRecreated: false, bytes: Buffer.byteLength(capture.data) });
      } finally { await page.close(); }
    }
    console.log(JSON.stringify({ serialNanoBehavior: results }, null, 2));
  } finally { await rm(fixtureDir, { force: true, recursive: true }); }
}
