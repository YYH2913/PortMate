import "@xterm/xterm/css/xterm.css";
import { Terminal } from "@xterm/xterm";
import { SerializeAddon } from "@xterm/addon-serialize";
import { Unicode11Addon } from "@xterm/addon-unicode11";

const terminal = new Terminal({
  allowProposedApi: true,
  cols: 80,
  rows: 24,
  scrollback: 20_000,
  convertEol: false,
  cursorBlink: false,
  fontFamily: "DejaVu Sans Mono, monospace",
  fontSize: 14,
  theme: {
    background: "#071015",
    foreground: "#d8e2e8",
    cursor: "#63d8c6",
  },
});
const serialize = new SerializeAddon();
const unicode = new Unicode11Addon();
terminal.loadAddon(serialize);
terminal.loadAddon(unicode);
terminal.unicode.activeVersion = "11";
terminal.open(document.querySelector("#terminal"));
terminal.focus();
terminal.onData((data) => globalThis.portmatePtyInput?.(data));

function bufferText() {
  const buffer = terminal.buffer.active;
  const lines = [];
  for (let row = 0; row < buffer.length; row += 1) {
    lines.push(buffer.getLine(row)?.translateToString(true) ?? "");
  }
  return lines.join("\n");
}

globalThis.__portmateTerminalHarness = {
  reset() {
    terminal.reset();
    terminal.resize(80, 24);
    terminal.clear();
    terminal.focus();
  },
  resize(cols, rows) {
    terminal.resize(cols, rows);
  },
  snapshot() {
    const text = bufferText();
    return {
      cols: terminal.cols,
      rows: terminal.rows,
      text,
      serialized: serialize.serialize(),
      alternate: terminal.buffer.active.type === "alternate",
      nonEmptyLines: text.split("\n").filter((line) => line.trim()).length,
    };
  },
  write(data) {
    return new Promise((resolve) => terminal.write(data, resolve));
  },
};
