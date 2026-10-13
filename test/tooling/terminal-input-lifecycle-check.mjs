import assert from "node:assert/strict";
import { createServer } from "vite";
import { chromium } from "playwright-core";

const moduleId = "virtual:portmate-terminal-input-lifecycle";
const resolvedId = `\0${moduleId}`;
const harness = `
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import TerminalCanvas from "/src/TerminalCanvas.tsx";
import DetachedPaneApp from "/src/DetachedPaneApp.tsx";
import "/src/styles.css";
const profile = {
  id: "serial", name: "Serial lifecycle", kind: "serial", group: "", tags: [],
  connection: { kind: "serial", path: "/dev/not-used", baudRate: 115200, dataBits: 8, stopBits: 1, parity: "none", flowControl: "none" },
  terminal: { term: "xterm-256color", rows: 28, cols: 100, scrollback: 1000, fontFamily: "monospace", fontSize: 13, theme: "portmate-dark", backgroundOpacity: 100 },
  logging: { enabled: false, raw: false, text: false, jsonl: false, redactSecrets: true, pathTemplate: "", retentionDays: 0 },
};
window.__writes = []; window.__history = []; window.__clipboard = "";
Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
  readText: () => window.__readText ? window.__readText() : Promise.resolve(window.__clipboard),
  writeText: async () => {},
} });
function Harness() {
  const [runtime, setRuntime] = useState({ status: "disconnected", connectedSince: null });
  window.__setConnection = (status, connectedSince) => flushSync(() => setRuntime({ status, connectedSince }));
  window.__connection = runtime;
  return React.createElement(TerminalCanvas, {
    viewId: "serial-view", active: { profile, runtime }, events: [], focused: true,
    oneKeyCompletionEnabled: false, completionSettings: { enabled: false },
    onInput: (id, text, origin, options) => {
      window.__writes.push({ id, text, origin, options });
      if (options?.expectedConnectedSince !== undefined && options.expectedConnectedSince !== window.__connection.connectedSince) return Promise.reject(new Error("stale"));
      if (window.__rejectWrite) return Promise.reject(new Error("transport rejected"));
      if (window.__holdAck && options?.awaitWrite) return new Promise((resolve, reject) => { window.__releaseAck = resolve; window.__rejectAck = reject; });
      return Promise.resolve();
    },
    onCommandSubmit: (id, command, source) => { window.__history.push({ id, command, source }); },
  });
}
if (new URL(location.href).searchParams.has("detached")) {
  window.__session = { profile, runtime: { status: "connected", connectedSince: "detached-a", sessionId: "serial", paneId: "serial:main", activeTransport: "serial" } };
  window.__listeners = new Map(); window.__callbacks = new Map(); window.__calls = [];
  let callbackId = 0;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (event, id) => {
    window.__listeners.set(event, (window.__listeners.get(event) || []).filter(value => value !== id));
  } };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "pane-harness" }, currentWebview: { label: "pane-harness" } },
    transformCallback: callback => { window.__callbacks.set(++callbackId, callback); return callbackId; },
    unregisterCallback: id => window.__callbacks.delete(id),
    invoke: async (command, args = {}) => {
      window.__calls.push({ command, args });
      if (command === "plugin:event|listen") {
        window.__listeners.set(args.event, [...(window.__listeners.get(args.event) || []), args.handler]);
        return args.handler;
      }
      if (command === "plugin:event|unlisten") return null;
      if (command === "list_sessions") return [window.__session];
      if (command === "tail_log" || command === "list_one_keys") return [];
      if (command === "begin_terminal_input_stream") return { streamId: "stream-" + window.__session.runtime.connectedSince };
      if (command === "record_command_history") return { entries: [], enabled: true, limit: 100, retentionDays: 30 };
      if (command === "send_text") {
        if (args.expectedConnectedSince !== window.__session.runtime.connectedSince) throw new Error("native runtime changed");
        if (window.__holdAck && args.awaitWrite) return new Promise(resolve => { window.__releaseAck = resolve; });
      }
      return null;
    },
  };
  window.__setDetachedConnection = (status, connectedSince) => {
    window.__session = { ...window.__session, runtime: { ...window.__session.runtime, status, connectedSince } };
    for (const id of window.__listeners.get("portmate-session-profile-updated") || []) {
      window.__callbacks.get(id)?.({ event: "portmate-session-profile-updated", id, payload: window.__session });
    }
  };
  createRoot(document.getElementById("root")).render(React.createElement(DetachedPaneApp, { request: {
    windowId: "pane-harness", ownerWindowId: "main", paneId: "serial-pane", viewId: "serial-view", sessionId: "serial", title: "Serial", color: "", keyMode: "remote",
  } }));
} else createRoot(document.getElementById("root")).render(React.createElement(Harness));
`;

const vite = await createServer({
  configFile: false,
  server: { host: "127.0.0.1", port: 0 },
  logLevel: "error",
  plugins: [{
    name: "terminal-input-lifecycle-harness",
    resolveId(id) { if (id === moduleId) return resolvedId; },
    load(id) { if (id === resolvedId) return harness; },
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        if (req.url?.split("?")[0] !== "/") return next();
        res.setHeader("Content-Type", "text/html");
        res.end(await server.transformIndexHtml("/", '<html><head><style>html,body,#root{margin:0;width:100%;height:100%}</style></head><body><div id="root"></div><script type="module" src="/@id/virtual:portmate-terminal-input-lifecycle"></script></body></html>'));
      });
    },
  }],
});

let browser;
try {
  await vite.listen();
  const address = vite.httpServer.address();
  const url = `http://127.0.0.1:${address.port}/`;
  browser = await chromium.launch({
    executablePath: process.env.PORTMATE_CHROME ?? "/usr/bin/google-chrome",
    headless: true, args: ["--no-sandbox", "--enable-unsafe-swiftshader"],
  });
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  const errors = [];
  page.on("pageerror", error => errors.push(String(error)));
  await page.goto(url);
  await page.locator('.terminal-host[data-terminal-ready="true"]').waitFor();
  const instance = await page.locator(".terminal-host").getAttribute("data-terminal-instance-id");
  const middlePaste = () => page.locator(".terminal-host").evaluate(host => host.dispatchEvent(new MouseEvent("auxclick", { button: 1, bubbles: true })));
  const connect = (status, since) => page.evaluate(([next, timestamp]) => window.__setConnection(next, timestamp), [status, since]);
  await connect("connected", "connection-a");
  await page.evaluate(() => { window.__clipboard = "echo first\n"; });
  await middlePaste();
  await page.waitForFunction(() => window.__history.length === 1);
  assert.deepEqual(await page.evaluate(() => window.__writes[0]), {
    id: "serial", text: "echo first\r", origin: "atomic", options: { awaitWrite: true, expectedConnectedSince: "connection-a" },
  });
  assert.equal(await page.locator(".terminal-host").getAttribute("data-terminal-instance-id"), instance, "first-connect paste must work without recreating xterm");

  await page.evaluate(() => { window.__readText = () => new Promise(resolve => { window.__resolveClipboard = resolve; }); });
  await middlePaste();
  await connect("disconnected", null);
  await connect("connected", "connection-b");
  await page.evaluate(() => { window.__resolveClipboard("echo stale\n"); window.__readText = null; });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(await page.evaluate(() => window.__writes.length), 1, "old async read must not reach input after reconnect");
  assert.equal(await page.evaluate(() => window.__history.length), 1, "old async read must not record history");
  await page.evaluate(() => { window.__clipboard = "echo new\n"; });
  await middlePaste();
  await page.waitForFunction(() => window.__history.length === 2);
  assert.equal(await page.evaluate(() => window.__writes[1].options.expectedConnectedSince), "connection-b");
  assert.equal(await page.locator(".terminal-host").getAttribute("data-terminal-instance-id"), instance, "reconnect must retain xterm while accepting fresh paste");

  await page.evaluate(() => { window.__rejectWrite = true; window.__clipboard = "echo rejected\n"; });
  await middlePaste();
  await page.waitForFunction(() => window.__writes.length === 3);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
  assert.equal(await page.evaluate(() => window.__history.length), 2, "failed writes must not create command history");
  await page.evaluate(() => { window.__rejectWrite = false; window.__readText = async () => { throw new Error("clipboard denied"); }; });
  await middlePaste();
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
  assert.equal(await page.evaluate(() => window.__writes.length), 3, "clipboard denial must not emit input");

  await connect("connected", "connection-b2");
  await page.evaluate(() => { window.__readText = null; window.__holdAck = true; window.__clipboard = "echo late-ack\n"; });
  await middlePaste();
  await page.waitForFunction(() => Boolean(window.__releaseAck));
  await connect("connected", "connection-c");
  await page.evaluate(() => { window.__holdAck = false; window.__releaseAck(); });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
  assert.equal(await page.evaluate(() => window.__history.length), 2, "late old-runtime ack must not publish history");
  await page.evaluate(() => { window.__clipboard = "echo recovered\n"; });
  await middlePaste();
  await page.waitForFunction(() => window.__history.length === 3);
  assert.equal(await page.evaluate(() => window.__history[2].command), "echo recovered", "fresh current input must resume history after reconnect");
  await page.evaluate(() => { window.__holdAck = true; window.__clipboard = "echo failed-partial"; window.__releaseAck = null; });
  await middlePaste();
  await page.waitForFunction(() => Boolean(window.__releaseAck));
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Enter");
  assert.equal(await page.evaluate(() => window.__history.length), 3, "Enter after an unacknowledged paste must wait for its write");
  await page.evaluate(() => { window.__holdAck = false; window.__rejectAck(new Error("partial write rejected")); });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
  assert.equal(await page.evaluate(() => window.__history.length), 3, "Enter must not record a line whose preceding paste failed");
  await connect("connected", "connection-d");
  await page.evaluate(() => { window.__holdAck = true; window.__clipboard = "echo ordered"; window.__releaseAck = null; });
  await middlePaste();
  await page.waitForFunction(() => Boolean(window.__releaseAck));
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Enter");
  await page.evaluate(() => { window.__holdAck = false; window.__releaseAck(); });
  await page.waitForFunction(() => window.__history.length === 4);
  assert.equal(await page.evaluate(() => window.__history[3].command), "echo ordered");
  assert.deepEqual(await page.evaluate(() => window.__writes.slice(-2).map(write => write.text)), ["echo ordered", "\r"], "paste must enter the input lane before the immediate Enter");
  await page.locator(".xterm-helper-textarea").evaluate(input => {
    const data = new DataTransfer();
    data.setData("text/plain", "echo dom-paste\n");
    input.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: data }));
  });
  await page.waitForFunction(() => window.__history.length === 5);
  assert.equal(await page.evaluate(() => window.__history[4].command), "echo dom-paste", "normal DOM paste must preserve acknowledged history");
  assert.deepEqual(errors, [], "targeted real TerminalCanvas lifecycle raised browser errors");
  const detachedPage = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  const detachedErrors = [];
  detachedPage.on("pageerror", error => detachedErrors.push(String(error)));
  await detachedPage.goto(url + "?detached=1");
  try {
    await detachedPage.locator('.terminal-host[data-terminal-ready="true"]').waitFor({ timeout: 10_000 });
  } catch (error) {
    console.error(JSON.stringify({ detachedErrors, state: await detachedPage.evaluate(() => ({ body: document.body.innerText, calls: window.__calls })) }));
    throw error;
  }
  await detachedPage.waitForFunction(() => window.__listeners.get("portmate-session-profile-updated")?.length > 0);
  const detachedMiddlePaste = () => detachedPage.locator(".terminal-host").evaluate(host => host.dispatchEvent(new MouseEvent("auxclick", { button: 1, bubbles: true })));
  await detachedPage.locator(".xterm-helper-textarea").focus();
  await detachedPage.keyboard.type("first keys");
  await detachedPage.waitForFunction(() => window.__calls.some(call => call.command === "send_text"));
  await detachedPage.evaluate(() => { window.__readText = () => new Promise(resolve => { window.__resolveClipboard = resolve; }); });
  await detachedMiddlePaste();
  await detachedPage.evaluate(() => { window.__setDetachedConnection("disconnected", null); });
  await detachedPage.evaluate(() => { window.__setDetachedConnection("connected", "detached-b"); });
  await detachedPage.waitForFunction(() => window.__calls.some(call => call.command === "close_terminal_input_stream"));
  await detachedPage.evaluate(() => { window.__resolveClipboard("old clipboard\n"); window.__readText = null; });
  await detachedPage.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
  assert.equal(await detachedPage.evaluate(() => window.__calls.some(call => call.command === "send_text" && call.args.text.includes("old clipboard"))), false);
  await detachedPage.evaluate(() => { window.__holdAck = true; window.__clipboard = "old in-flight\n"; });
  await detachedMiddlePaste();
  await detachedPage.waitForFunction(() => Boolean(window.__releaseAck));
  await detachedPage.evaluate(() => { window.__clipboard = "old pending\n"; });
  await detachedMiddlePaste();
  await detachedPage.evaluate(() => { window.__setDetachedConnection("connected", "detached-c"); window.__holdAck = false; });
  await detachedPage.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
  await detachedPage.evaluate(() => { window.__clipboard = "new connection\n"; });
  await detachedMiddlePaste();
  await detachedPage.waitForFunction(() => window.__calls.some(call => call.command === "send_text" && call.args.text === "new connection\r"));
  await detachedPage.evaluate(() => { window.__releaseAck(); });
  await detachedPage.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
  const detachedWrites = await detachedPage.evaluate(() => window.__calls.filter(call => call.command === "send_text"));
  assert.equal(detachedWrites.some(call => call.args.text.includes("old pending")), false, "detached pending paste must be cancelled on summary update");
  assert.equal(detachedWrites.at(-1).args.expectedConnectedSince, "detached-c");
  const detachedHistory = await detachedPage.evaluate(() => window.__calls.filter(call => call.command === "record_command_history").map(call => call.args.command));
  assert.equal(detachedHistory.includes("old pending") || detachedHistory.includes("old in-flight"), false, "detached old writes must not publish history after reconnect");
  assert.equal(detachedHistory.includes("new connection"), true);
  assert.deepEqual(detachedErrors, [], "targeted actual DetachedPaneApp lifecycle raised browser errors");
  console.log("TerminalCanvas lifecycle passed: first connect, reconnect, stale read, write rejection, clipboard denial, late acknowledgement, immediate Enter ordering/history and DOM paste.");
  console.log("DetachedPaneApp lifecycle passed: native-stream reset, delayed clipboard discard, pending-lane cancellation, fresh input and acknowledged history after summary update.");
} finally {
  await browser?.close();
  await vite.close();
}
