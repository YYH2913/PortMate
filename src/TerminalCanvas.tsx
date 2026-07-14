import { useEffect, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { SerializeAddon } from "@xterm/addon-serialize";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { ClipboardAddon } from "@xterm/addon-clipboard";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { listen } from "@tauri-apps/api/event";
import { invokeBackend, isBackendAvailable } from "./api";
import type { SyncInputOrigin } from "./sync-input-state";
import { createWriteOnlyClipboardProvider } from "./terminal-clipboard";
import { terminalStateCache } from "./terminal-state-cache";
import type { SessionEvent, SessionSummary } from "./types";

type TerminalCanvasProps = {
  active?: SessionSummary;
  events: SessionEvent[];
  focused?: boolean;
  onInput: (sessionId: string, text: string, origin: SyncInputOrigin) => void;
};

const MAX_SERIALIZED_SCROLLBACK = 2000;
type WebglAddonInstance = import("@xterm/addon-webgl").WebglAddon;

const portmateTerminalTheme = {
  background: "#0d1117",
  foreground: "#d7e1eb",
  cursor: "#5eead4",
  cursorAccent: "#0d1117",
  selectionBackground: "#284457",
  selectionForeground: "#f8fafc",
  black: "#0d1117",
  red: "#f87171",
  green: "#37d67a",
  yellow: "#f4b860",
  blue: "#68a7ff",
  magenta: "#c084fc",
  cyan: "#5eead4",
  white: "#d7e1eb",
  brightBlack: "#6b7280",
  brightRed: "#ff8a8a",
  brightGreen: "#86efac",
  brightYellow: "#fde047",
  brightBlue: "#93c5fd",
  brightMagenta: "#d8b4fe",
  brightCyan: "#67e8f9",
  brightWhite: "#ffffff",
  extendedAnsi: createXterm256Palette(),
};

export default function TerminalCanvas({ active, events, focused = false, onInput }: TerminalCanvasProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XTerm | null>(null);
  const seenEventsRef = useRef<Set<string>>(new Set());
  const pendingInputRef = useRef("");
  const inputFlushTimerRef = useRef<number | null>(null);
  const lastSizeRef = useRef("");
  const lastCopiedSelectionRef = useRef("");
  const onInputRef = useRef(onInput);
  onInputRef.current = onInput;

  function markEventSeen(id: string): boolean {
    if (seenEventsRef.current.size > 4000) {
      seenEventsRef.current.clear();
    }
    if (seenEventsRef.current.has(id)) return true;
    seenEventsRef.current.add(id);
    return false;
  }

  useEffect(() => {
    if (!active || !hostRef.current) return;

    const host = hostRef.current;
    const cachedState = terminalStateCache.get(active.profile.id);
    seenEventsRef.current = new Set(cachedState?.seenEventIds ?? []);
    lastSizeRef.current = "";
    lastCopiedSelectionRef.current = "";
    const term = new XTerm({
      allowProposedApi: true,
      cols: cachedState?.cols ?? active.profile.terminal.cols,
      rows: cachedState?.rows ?? active.profile.terminal.rows,
      cursorBlink: true,
      convertEol: false,
      drawBoldTextInBrightColors: true,
      fontFamily: active.profile.terminal.fontFamily,
      fontSize: active.profile.terminal.fontSize,
      minimumContrastRatio: 1,
      scrollback: active.profile.terminal.scrollback,
      theme: portmateTerminalTheme,
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    const serialize = new SerializeAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.loadAddon(serialize);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    term.loadAddon(new WebLinksAddon());
    term.loadAddon(new ClipboardAddon(undefined, createWriteOnlyClipboardProvider(navigator.clipboard)));
    host.dataset.terminalUnicodeVersion = term.unicode.activeVersion;
    host.dataset.terminalClipboard = "write-only";
    host.dataset.terminalSerialization = "active";
    host.dataset.terminalRenderer = "dom";
    host.dataset.terminalWebgl = "loading";
    host.dataset.terminalRestored = cachedState ? "true" : "false";
    if (cachedState) term.write(cachedState.serialized);
    term.open(host);
    let terminalDisposed = false;
    let webglAddon: WebglAddonInstance | null = null;
    let webglContextLossDisposable: { dispose: () => void } | null = null;
    void import("@xterm/addon-webgl").then(({ WebglAddon }) => {
      if (terminalDisposed) return;
      webglAddon = new WebglAddon();
      term.loadAddon(webglAddon);
      host.dataset.terminalRenderer = "webgl";
      host.dataset.terminalWebgl = "active";
      webglContextLossDisposable = webglAddon.onContextLoss(() => {
        const lostAddon = webglAddon;
        webglAddon = null;
        lostAddon?.dispose();
        host.dataset.terminalRenderer = "dom";
        host.dataset.terminalWebgl = "fallback";
      });
    }).catch(() => {
      webglAddon?.dispose();
      webglAddon = null;
      host.dataset.terminalRenderer = "dom";
      host.dataset.terminalWebgl = "fallback";
    });
    if (focused) term.focus();
    const fitAndReport = () => {
      fit.fit();
      const size = `${term.cols}x${term.rows}`;
      if (lastSizeRef.current !== size) {
        lastSizeRef.current = size;
        if (isBackendAvailable()) {
          void invokeBackend("resize_session", {
            sessionId: active.profile.id,
            cols: term.cols,
            rows: term.rows,
          }).catch(() => {});
        }
      }
    };
    queueMicrotask(fitAndReport);

    const resizeObserver = new ResizeObserver(fitAndReport);
    resizeObserver.observe(host);
    const flushInput = () => {
      inputFlushTimerRef.current = null;
      const text = pendingInputRef.current;
      pendingInputRef.current = "";
      if (text) {
        onInputRef.current(active.profile.id, text, "interactive");
      }
    };
    const inputDisposable = term.onData((text) => {
      pendingInputRef.current += text;
      if (/[\x00-\x1f\x7f]/.test(text)) {
        if (inputFlushTimerRef.current !== null) {
          window.clearTimeout(inputFlushTimerRef.current);
        }
        flushInput();
        return;
      }
      if (inputFlushTimerRef.current === null) {
        inputFlushTimerRef.current = window.setTimeout(flushInput, 12);
      }
    });
    const selectionDisposable = term.onSelectionChange(() => {
      const selected = term.getSelection();
      if (!selected || selected === lastCopiedSelectionRef.current) return;
      lastCopiedSelectionRef.current = selected;
      void navigator.clipboard?.writeText(selected).catch(() => {});
    });
    const pasteFromClipboard = (event: MouseEvent) => {
      event.preventDefault();
      void navigator.clipboard?.readText().then((text) => {
        if (text) onInputRef.current(active.profile.id, text, "atomic");
      }).catch(() => {});
    };
    const pasteOnMiddleClick = (event: MouseEvent) => {
      if (event.button === 1) {
        pasteFromClipboard(event);
      }
    };
    host.addEventListener("auxclick", pasteOnMiddleClick);

    termRef.current = term;

    return () => {
      terminalDisposed = true;
      inputDisposable.dispose();
      selectionDisposable.dispose();
      host.removeEventListener("auxclick", pasteOnMiddleClick);
      if (inputFlushTimerRef.current !== null) {
        window.clearTimeout(inputFlushTimerRef.current);
        inputFlushTimerRef.current = null;
      }
      pendingInputRef.current = "";
      resizeObserver.disconnect();
      webglContextLossDisposable?.dispose();
      try {
        terminalStateCache.save(active.profile.id, {
          serialized: serialize.serialize({
            scrollback: Math.min(MAX_SERIALIZED_SCROLLBACK, active.profile.terminal.scrollback),
          }),
          cols: term.cols,
          rows: term.rows,
          seenEventIds: [...seenEventsRef.current],
        });
      } catch {
        // Serialization must not prevent terminal disposal.
      }
      term.dispose();
      termRef.current = null;
    };
  }, [active?.profile.id]);

  useEffect(() => {
    if (focused) termRef.current?.focus();
  }, [active?.profile.id, focused]);

  useEffect(() => {
    if (!active || !isBackendAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void listen<SessionEvent>("portmate-session-event", (event) => {
      if (disposed || event.payload.sessionId !== active.profile.id) return;
      const term = termRef.current;
      if (!term || markEventSeen(event.payload.id)) return;
      writeTerminalEvent(term, event.payload);
    })
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
          return;
        }
        unlisten = nextUnlisten;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [active?.profile.id]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    for (const event of events) {
      if (markEventSeen(event.id)) continue;
      writeTerminalEvent(term, event);
    }
  }, [events, active?.profile.id]);

  return (
    <div className="terminal-canvas">
      {active ? (
        <div ref={hostRef} className="terminal-host" />
      ) : (
        <div className="terminal-empty">未打开会话</div>
      )}
    </div>
  );
}

function writeTerminalEvent(term: XTerm, event: SessionEvent) {
  if (!event.text || event.direction === "outbound") return;
  if (event.direction === "system" || event.stream === "control" || event.stream === "audit") {
    term.writeln(`\x1b[38;5;245m${event.text}\x1b[0m`);
    return;
  }
  term.write(event.text);
}

function createXterm256Palette() {
  const toHex = (value: number) => value.toString(16).padStart(2, "0");
  const rgb = (red: number, green: number, blue: number) => `#${toHex(red)}${toHex(green)}${toHex(blue)}`;
  const palette: string[] = [];
  const steps = [0, 95, 135, 175, 215, 255];
  for (const red of steps) {
    for (const green of steps) {
      for (const blue of steps) {
        palette.push(rgb(red, green, blue));
      }
    }
  }
  for (let index = 0; index < 24; index += 1) {
    const level = 8 + index * 10;
    palette.push(rgb(level, level, level));
  }
  return palette;
}
