import { useEffect, useMemo, useRef, useState } from "react";
import { CaseSensitive, ChevronDown, ChevronUp, KeyRound, Regex, Search, SendHorizontal, WholeWord, X } from "lucide-react";
import { ClipboardAddon } from "@xterm/addon-clipboard";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import type { ISearchOptions } from "@xterm/addon-search";
import { SerializeAddon } from "@xterm/addon-serialize";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { listen } from "@tauri-apps/api/event";
import { invokeBackend, isBackendAvailable } from "./api";
import { emptyOneKeyPromptDetectionState, oneKeyPromptCandidates, oneKeyPromptStateFromEvents, reduceOneKeyPromptDetection } from "./one-key-completion-state";
import type { OneKeyPromptDetectionState, OneKeyPromptField, OneKeyTerminalPrompt } from "./one-key-completion-state";
import type { SyncInputOrigin } from "./sync-input-state";
import { createWriteOnlyClipboardProvider } from "./terminal-clipboard";
import { createTerminalFreeInputPayload, cutTerminalFreeInputRange, MAX_TERMINAL_FREE_INPUT_CHARACTERS, normalizeTerminalFreeInput, terminalFreeInputCharacterCount, TERMINAL_FREE_INPUT_REQUEST_EVENT } from "./terminal-free-input";
import { emptyTerminalKeySequenceState, resolveTerminalKeyModeEvent } from "./terminal-key-mode";
import type { TerminalKeyMode, TerminalKeySequenceState, TerminalLocalCommand } from "./terminal-key-mode";
import { isTerminalFindShortcut, MAX_TERMINAL_SEARCH_QUERY_LENGTH, terminalSearchResultLabel, terminalSearchSeed, TERMINAL_SEARCH_REQUEST_EVENT } from "./terminal-search";
import type { TerminalSearchResult } from "./terminal-search";
import { terminalStateCache } from "./terminal-state-cache";
import type { OneKeySummary, SessionEvent, SessionSummary } from "./types";

type TerminalCanvasProps = {
  viewId?: string;
  active?: SessionSummary;
  events: SessionEvent[];
  focused?: boolean;
  oneKeys?: readonly OneKeySummary[];
  oneKeyCompletionEnabled?: boolean;
  keyMode?: TerminalKeyMode;
  onKeyModeChange?: (mode: TerminalKeyMode) => void;
  onInput: (sessionId: string, text: string, origin: SyncInputOrigin) => void;
  onOneKeyCompletion?: (
    sessionId: string,
    oneKeyId: string,
    field: OneKeyPromptField,
    promptEventId: string,
  ) => Promise<void>;
};

const MAX_SERIALIZED_SCROLLBACK = 2000;
const EMPTY_ONE_KEYS: readonly OneKeySummary[] = [];
type WebglAddonInstance = import("@xterm/addon-webgl").WebglAddon;
type LocalNavigationPosition = { row: number; column: number };
type LocalNavigationState = LocalNavigationPosition & {
  anchor: LocalNavigationPosition | null;
  lineSelection: boolean;
};

const terminalSearchDecorations: NonNullable<ISearchOptions["decorations"]> = {
  matchBackground: "#284457",
  matchBorder: "#68a7ff",
  matchOverviewRuler: "#68a7ff",
  activeMatchBackground: "#f4b860",
  activeMatchBorder: "#ffffff",
  activeMatchColorOverviewRuler: "#f4b860",
};

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

export default function TerminalCanvas({
  viewId = "",
  active,
  events,
  focused = false,
  oneKeys = EMPTY_ONE_KEYS,
  oneKeyCompletionEnabled = true,
  keyMode = "remote",
  onKeyModeChange = () => {},
  onInput,
  onOneKeyCompletion,
}: TerminalCanvasProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XTerm | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const freeInputRef = useRef<HTMLTextAreaElement | null>(null);
  const seenEventsRef = useRef<Set<string>>(new Set());
  const pendingInputRef = useRef("");
  const inputFlushTimerRef = useRef<number | null>(null);
  const lastSizeRef = useRef("");
  const lastCopiedSelectionRef = useRef("");
  const onInputRef = useRef(onInput);
  const keyModeRef = useRef(keyMode);
  const previousKeyModeRef = useRef(keyMode);
  const onKeyModeChangeRef = useRef(onKeyModeChange);
  const keySequenceRef = useRef<TerminalKeySequenceState>(emptyTerminalKeySequenceState());
  const localNavigationRef = useRef<LocalNavigationState | null>(null);
  const openSearchRef = useRef<() => void>(() => {});
  const openFreeInputRef = useRef<() => void>(() => {});
  const runSearchRef = useRef<(direction: "next" | "previous") => void>(() => {});
  const oneKeyPromptStateRef = useRef<OneKeyPromptDetectionState>(emptyOneKeyPromptDetectionState());
  const oneKeyPromptSessionRef = useRef("");
  const dismissedOneKeyPromptEventsRef = useRef<Set<string>>(new Set());
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchCaseSensitive, setSearchCaseSensitive] = useState(false);
  const [searchRegex, setSearchRegex] = useState(false);
  const [searchWholeWord, setSearchWholeWord] = useState(false);
  const [searchResult, setSearchResult] = useState<TerminalSearchResult | null>(null);
  const [searchInvalid, setSearchInvalid] = useState(false);
  const [freeInputSource, setFreeInputSource] = useState<"manual" | "normal" | null>(null);
  const [freeInputValue, setFreeInputValue] = useState("");
  const [oneKeyPrompt, setOneKeyPrompt] = useState<OneKeyTerminalPrompt | null>(null);
  const [oneKeyCompletionId, setOneKeyCompletionId] = useState("");
  const [oneKeyCompletionBusy, setOneKeyCompletionBusy] = useState(false);
  const [oneKeyCompletionError, setOneKeyCompletionError] = useState("");
  const oneKeyCompletionCandidates = useMemo(
    () => oneKeyCompletionEnabled && active && oneKeyPrompt
      ? oneKeyPromptCandidates(oneKeys, active.profile.id, oneKeyPrompt)
      : [],
    [active?.profile.id, oneKeyCompletionEnabled, oneKeyPrompt, oneKeys],
  );
  const selectedOneKeyCompletion = oneKeyCompletionCandidates.find((oneKey) => oneKey.id === oneKeyCompletionId)
    ?? oneKeyCompletionCandidates[0]
    ?? null;
  onInputRef.current = onInput;
  keyModeRef.current = keyMode;
  onKeyModeChangeRef.current = onKeyModeChange;
  const freeInputOpen = freeInputSource !== null;
  openSearchRef.current = () => {
    setFreeInputSource(null);
    setFreeInputValue("");
    const selection = terminalSearchSeed(termRef.current?.getSelection() ?? "");
    if (!searchOpen && selection) setSearchQuery(selection);
    setSearchInvalid(false);
    setSearchOpen(true);
    window.requestAnimationFrame(() => {
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    });
  };
  openFreeInputRef.current = () => {
    dismissOneKeyPrompt();
    searchRef.current?.clearDecorations();
    setSearchOpen(false);
    setSearchResult(null);
    setSearchInvalid(false);
    if (!freeInputOpen) setFreeInputValue("");
    setFreeInputSource("manual");
    window.requestAnimationFrame(() => freeInputRef.current?.focus({ preventScroll: true }));
  };
  runSearchRef.current = (direction) => runTerminalSearch(direction);

  function applyOneKeyPromptState(state: OneKeyPromptDetectionState) {
    const previousEventId = oneKeyPromptStateRef.current.prompt?.eventId;
    oneKeyPromptStateRef.current = state;
    const prompt = state.prompt && !dismissedOneKeyPromptEventsRef.current.has(state.prompt.eventId)
      ? state.prompt
      : null;
    if (previousEventId !== prompt?.eventId) {
      setOneKeyCompletionBusy(false);
      setOneKeyCompletionError("");
    }
    setOneKeyPrompt(prompt);
  }

  function dismissOneKeyPrompt() {
    const prompt = oneKeyPromptStateRef.current.prompt;
    if (prompt) {
      if (dismissedOneKeyPromptEventsRef.current.size >= 256) {
        dismissedOneKeyPromptEventsRef.current.clear();
      }
      dismissedOneKeyPromptEventsRef.current.add(prompt.eventId);
    }
    oneKeyPromptStateRef.current = emptyOneKeyPromptDetectionState();
    setOneKeyPrompt(null);
    setOneKeyCompletionBusy(false);
    setOneKeyCompletionError("");
  }

  async function submitOneKeyCompletion() {
    if (!active || !oneKeyPrompt || !selectedOneKeyCompletion || !onOneKeyCompletion) return;
    const promptEventId = oneKeyPrompt.eventId;
    setOneKeyCompletionBusy(true);
    setOneKeyCompletionError("");
    try {
      await onOneKeyCompletion(
        active.profile.id,
        selectedOneKeyCompletion.id,
        oneKeyPrompt.field,
        promptEventId,
      );
      if (oneKeyPromptStateRef.current.prompt?.eventId === promptEventId) {
        dismissOneKeyPrompt();
      }
    } catch (error) {
      if (oneKeyPromptStateRef.current.prompt?.eventId === promptEventId) {
        setOneKeyCompletionBusy(false);
        setOneKeyCompletionError(formatTerminalCanvasError(error));
      }
    }
  }

  function markEventSeen(id: string): boolean {
    if (seenEventsRef.current.size > 4000) {
      seenEventsRef.current.clear();
    }
    if (seenEventsRef.current.has(id)) return true;
    seenEventsRef.current.add(id);
    return false;
  }

  function runTerminalSearch(direction: "next" | "previous", incremental = false) {
    const search = searchRef.current;
    if (!search || !searchQuery) {
      search?.clearDecorations();
      setSearchResult(null);
      setSearchInvalid(false);
      return;
    }
    if (searchRegex) {
      try {
        new RegExp(searchQuery);
      } catch {
        search.clearDecorations();
        setSearchResult(null);
        setSearchInvalid(true);
        return;
      }
    }
    const options: ISearchOptions = {
      caseSensitive: searchCaseSensitive,
      decorations: terminalSearchDecorations,
      incremental,
      regex: searchRegex,
      wholeWord: searchWholeWord,
    };
    try {
      if (direction === "previous") search.findPrevious(searchQuery, options);
      else search.findNext(searchQuery, options);
      setSearchInvalid(false);
    } catch {
      search.clearDecorations();
      setSearchResult(null);
      setSearchInvalid(true);
    }
  }

  function closeTerminalSearch() {
    setSearchOpen(false);
    searchRef.current?.clearDecorations();
    setSearchResult(null);
    setSearchInvalid(false);
    window.requestAnimationFrame(() => termRef.current?.focus());
  }

  function closeTerminalFreeInput() {
    const wasNormalMode = freeInputSource === "normal";
    setFreeInputSource(null);
    setFreeInputValue("");
    if (wasNormalMode) {
      keyModeRef.current = "remote";
      onKeyModeChangeRef.current("remote");
    }
    window.requestAnimationFrame(() => termRef.current?.focus());
  }

  function submitTerminalFreeInput() {
    if (!active) return;
    const payload = createTerminalFreeInputPayload(freeInputValue);
    if (!payload) return;
    onInputRef.current(active.profile.id, payload, "atomic");
    closeTerminalFreeInput();
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
    searchRef.current = search;
    const searchResultDisposable = search.onDidChangeResults((result) => {
      setSearchResult({ resultCount: result.resultCount, resultIndex: result.resultIndex });
      setSearchInvalid(false);
    });
    term.loadAddon(serialize);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    term.loadAddon(new WebLinksAddon());
    term.loadAddon(new ClipboardAddon(undefined, createWriteOnlyClipboardProvider(navigator.clipboard)));
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      const mode = keyModeRef.current;
      if (isTerminalFindShortcut(event) && mode !== "command") {
        event.preventDefault();
        openSearchRef.current();
        return false;
      }
      const resolution = resolveTerminalKeyModeEvent(mode, event, keySequenceRef.current);
      keySequenceRef.current = resolution.state;
      if (!resolution.handled) return true;
      event.preventDefault();
      if (resolution.nextMode) {
        keyModeRef.current = resolution.nextMode;
        onKeyModeChangeRef.current(resolution.nextMode);
      }
      if (resolution.command) {
        const result = runTerminalLocalCommand(
          term,
          localNavigationRef.current,
          resolution.command,
          resolution.count,
        );
        localNavigationRef.current = result.state;
        if (result.copyText) void navigator.clipboard?.writeText(result.copyText).catch(() => {});
        if (result.search === "open") {
          term.clearSelection();
          openSearchRef.current();
        }
        else if (result.search) runSearchRef.current(result.search);
      }
      return false;
    });
    host.dataset.terminalUnicodeVersion = term.unicode.activeVersion;
    host.dataset.terminalClipboard = "write-only";
    host.dataset.terminalSerialization = "active";
    host.dataset.terminalRenderer = "dom";
    host.dataset.terminalWebgl = "loading";
    host.dataset.terminalKeyMode = keyModeRef.current;
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
      if (keyModeRef.current !== "remote") return;
      dismissOneKeyPrompt();
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
      if (keyModeRef.current !== "remote") return;
      const selected = term.getSelection();
      if (!selected || selected === lastCopiedSelectionRef.current) return;
      lastCopiedSelectionRef.current = selected;
      void navigator.clipboard?.writeText(selected).catch(() => {});
    });
    const pasteFromClipboard = (event: MouseEvent) => {
      event.preventDefault();
      if (keyModeRef.current !== "remote") return;
      void navigator.clipboard?.readText().then((text) => {
        if (text) {
          dismissOneKeyPrompt();
          onInputRef.current(active.profile.id, text, "atomic");
        }
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
      searchResultDisposable.dispose();
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
      if (searchRef.current === search) searchRef.current = null;
      termRef.current = null;
    };
  }, [active?.profile.id]);

  useEffect(() => {
    const sessionId = active?.profile.id ?? "";
    if (oneKeyPromptSessionRef.current !== sessionId) {
      oneKeyPromptSessionRef.current = sessionId;
      dismissedOneKeyPromptEventsRef.current.clear();
      setOneKeyCompletionId("");
    }
    applyOneKeyPromptState(oneKeyCompletionEnabled && sessionId
      ? oneKeyPromptStateFromEvents(events, sessionId)
      : emptyOneKeyPromptDetectionState());
  }, [active?.profile.id, events, oneKeyCompletionEnabled]);

  useEffect(() => {
    if (!oneKeyCompletionCandidates.length) {
      setOneKeyCompletionId("");
      return;
    }
    setOneKeyCompletionId((current) => (
      oneKeyCompletionCandidates.some((oneKey) => oneKey.id === current)
        ? current
        : oneKeyCompletionCandidates[0].id
    ));
  }, [oneKeyPrompt?.eventId, oneKeyCompletionCandidates]);

  useEffect(() => {
    const requestSearch = () => {
      if (active && focused) openSearchRef.current();
    };
    window.addEventListener(TERMINAL_SEARCH_REQUEST_EVENT, requestSearch);
    return () => window.removeEventListener(TERMINAL_SEARCH_REQUEST_EVENT, requestSearch);
  }, [active?.profile.id, focused]);

  useEffect(() => {
    const requestFreeInput = () => {
      if (active && focused) openFreeInputRef.current();
    };
    window.addEventListener(TERMINAL_FREE_INPUT_REQUEST_EVENT, requestFreeInput);
    return () => window.removeEventListener(TERMINAL_FREE_INPUT_REQUEST_EVENT, requestFreeInput);
  }, [active?.profile.id, focused]);

  useEffect(() => {
    setFreeInputSource(null);
    setFreeInputValue("");
  }, [active?.profile.id, viewId]);

  useEffect(() => {
    const previousMode = previousKeyModeRef.current;
    previousKeyModeRef.current = keyMode;
    keySequenceRef.current = emptyTerminalKeySequenceState();
    const term = termRef.current;
    const host = hostRef.current;
    if (host) host.dataset.terminalKeyMode = keyMode;

    if (keyMode === "local" || keyMode === "command") {
      setFreeInputSource(null);
      if (!(previousMode === "normal" && keyMode === "command")) setFreeInputValue("");
      if (term) {
        localNavigationRef.current = clampLocalNavigationState(term, localNavigationRef.current);
        renderTerminalLocalSelection(term, localNavigationRef.current);
        window.requestAnimationFrame(() => term.focus());
      }
      return;
    }

    localNavigationRef.current = null;
    term?.clearSelection();
    if (keyMode === "normal") {
      searchRef.current?.clearDecorations();
      setSearchOpen(false);
      setSearchResult(null);
      if (previousMode !== "command") setFreeInputValue("");
      setFreeInputSource("normal");
      window.requestAnimationFrame(() => freeInputRef.current?.focus({ preventScroll: true }));
      return;
    }

    setFreeInputSource(null);
    setFreeInputValue("");
    window.requestAnimationFrame(() => term?.focus());
  }, [active?.profile.id, keyMode, viewId]);

  useEffect(() => {
    if (!searchOpen) {
      searchRef.current?.clearDecorations();
      setSearchResult(null);
      setSearchInvalid(false);
      return;
    }
    runTerminalSearch("next", true);
  }, [active?.profile.id, searchCaseSensitive, searchOpen, searchQuery, searchRegex, searchWholeWord]);

  useEffect(() => {
    if (!searchOpen) return;
    const frame = window.requestAnimationFrame(() => searchInputRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [active?.profile.id, searchOpen]);

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
      if (oneKeyCompletionEnabled) {
        applyOneKeyPromptState(reduceOneKeyPromptDetection(
          oneKeyPromptStateRef.current,
          event.payload,
        ));
      }
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
  }, [active?.profile.id, oneKeyCompletionEnabled]);

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
        <>
          <div ref={hostRef} className="terminal-host" inert={freeInputOpen} />
          {freeInputOpen ? (
            <form className="terminal-free-input" aria-label="自由输入编辑器" onSubmit={(event) => {
              event.preventDefault();
              submitTerminalFreeInput();
            }}>
              <header>
                <strong>{freeInputSource === "normal" ? "Normal 本地编辑" : "自由输入"}</strong>
                <span className="terminal-free-input-session" title={active.profile.name}>{active.profile.name}</span>
                <span className="terminal-free-input-counter">
                  {terminalFreeInputCharacterCount(freeInputValue)}/{MAX_TERMINAL_FREE_INPUT_CHARACTERS}
                </span>
                <button type="submit" title="发送自由输入" aria-label="发送自由输入" disabled={!freeInputValue}><SendHorizontal size={15} /></button>
                <button type="button" title="取消自由输入" aria-label="取消自由输入" onClick={closeTerminalFreeInput}><X size={15} /></button>
              </header>
              <textarea
                ref={freeInputRef}
                aria-label="自由输入内容"
                value={freeInputValue}
                wrap="off"
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                onChange={(event) => setFreeInputValue(normalizeTerminalFreeInput(event.target.value))}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    event.stopPropagation();
                    if (freeInputSource === "normal") {
                      setFreeInputSource(null);
                      keyModeRef.current = "command";
                      onKeyModeChangeRef.current("command");
                    } else {
                      closeTerminalFreeInput();
                    }
                    return;
                  }
                  if (freeInputSource === "normal" && (event.ctrlKey || event.metaKey) && event.key === "Enter") {
                    event.preventDefault();
                    event.stopPropagation();
                    setFreeInputSource(null);
                    keyModeRef.current = "command";
                    onKeyModeChangeRef.current("command");
                    return;
                  }
                  if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === "x") {
                    const textarea = event.currentTarget;
                    const cut = cutTerminalFreeInputRange(freeInputValue, textarea.selectionStart, textarea.selectionEnd);
                    if (!cut.cutText) return;
                    event.preventDefault();
                    void navigator.clipboard?.writeText(cut.cutText).catch(() => {});
                    setFreeInputValue(cut.value);
                    window.requestAnimationFrame(() => {
                      textarea.focus({ preventScroll: true });
                      textarea.setSelectionRange(cut.caret, cut.caret);
                    });
                    return;
                  }
                  if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    event.stopPropagation();
                    submitTerminalFreeInput();
                  }
                }}
              />
            </form>
          ) : null}
          {focused
            && !freeInputOpen
            && oneKeyPrompt
            && selectedOneKeyCompletion
            && onOneKeyCompletion ? (
              <form className="terminal-one-key-completion" aria-label="OneKey 终端提示补全" onSubmit={(event) => {
                event.preventDefault();
                void submitOneKeyCompletion();
              }}>
                <KeyRound size={15} aria-hidden="true" />
                <span className="terminal-one-key-prompt">
                  <strong>{oneKeyPrompt.field === "username" ? "用户名提示" : "密码提示"}</strong>
                  <small className={oneKeyCompletionError ? "error" : ""} title={oneKeyCompletionError || oneKeyPrompt.line}>
                    {oneKeyCompletionError || oneKeyPrompt.line}
                  </small>
                </span>
                <select
                  aria-label="选择 OneKey"
                  value={selectedOneKeyCompletion.id}
                  disabled={oneKeyCompletionBusy}
                  onChange={(event) => setOneKeyCompletionId(event.target.value)}
                >
                  {oneKeyCompletionCandidates.map((oneKey) => (
                    <option key={oneKey.id} value={oneKey.id}>{oneKey.label} · {oneKey.username}</option>
                  ))}
                </select>
                <button className="primary" type="submit" title="发送 OneKey 凭据" disabled={oneKeyCompletionBusy}>
                  <SendHorizontal size={14} />
                  <span>{oneKeyCompletionBusy ? "发送中" : "发送"}</span>
                </button>
                <button type="button" title="忽略当前提示" aria-label="忽略当前 OneKey 提示" disabled={oneKeyCompletionBusy} onClick={dismissOneKeyPrompt}>
                  <X size={14} />
                </button>
              </form>
            ) : null}
          {searchOpen ? (
            <form className="terminal-search-bar" onSubmit={(event) => {
              event.preventDefault();
              runTerminalSearch("next");
            }}>
              <Search size={14} aria-hidden="true" />
              <input
                ref={searchInputRef}
                aria-label="终端查找"
                value={searchQuery}
                maxLength={MAX_TERMINAL_SEARCH_QUERY_LENGTH}
                placeholder="在当前终端中查找"
                spellCheck={false}
                onChange={(event) => setSearchQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    closeTerminalSearch();
                  } else if (event.key === "Enter" && event.shiftKey) {
                    event.preventDefault();
                    runTerminalSearch("previous");
                  }
                }}
              />
              <span className={searchInvalid ? "terminal-search-status invalid" : "terminal-search-status"} role="status" aria-live="polite">
                {terminalSearchResultLabel(searchQuery, searchResult, searchInvalid)}
              </span>
              <div className="terminal-search-controls">
                <button type="button" className={searchCaseSensitive ? "active" : ""} aria-label="区分大小写" aria-pressed={searchCaseSensitive} title="区分大小写" onClick={() => setSearchCaseSensitive((value) => !value)}><CaseSensitive size={15} /></button>
                <button type="button" className={searchWholeWord ? "active" : ""} aria-label="全词匹配" aria-pressed={searchWholeWord} title="全词匹配" onClick={() => setSearchWholeWord((value) => !value)}><WholeWord size={15} /></button>
                <button type="button" className={searchRegex ? "active" : ""} aria-label="正则表达式" aria-pressed={searchRegex} title="正则表达式" onClick={() => setSearchRegex((value) => !value)}><Regex size={15} /></button>
                <button type="button" aria-label="上一个匹配" title="上一个匹配" disabled={!searchQuery} onClick={() => runTerminalSearch("previous")}><ChevronUp size={15} /></button>
                <button type="button" aria-label="下一个匹配" title="下一个匹配" disabled={!searchQuery} onClick={() => runTerminalSearch("next")}><ChevronDown size={15} /></button>
                <button type="button" aria-label="关闭查找" title="关闭查找" onClick={closeTerminalSearch}><X size={15} /></button>
              </div>
            </form>
          ) : null}
        </>
      ) : (
        <div className="terminal-empty">未打开会话</div>
      )}
    </div>
  );
}

type TerminalLocalCommandResult = {
  state: LocalNavigationState;
  copyText?: string;
  search?: "open" | "next" | "previous";
};

function runTerminalLocalCommand(
  term: XTerm,
  current: LocalNavigationState | null,
  command: TerminalLocalCommand,
  requestedCount: number,
): TerminalLocalCommandResult {
  const state = clampLocalNavigationState(term, current);
  const count = Math.max(1, Math.min(100_000, Math.trunc(requestedCount) || 1));
  const lastRow = Math.max(0, term.buffer.active.length - 1);
  let copyText: string | undefined;
  let search: TerminalLocalCommandResult["search"];

  if (command === "move-left") state.column -= count;
  else if (command === "move-right") state.column += count;
  else if (command === "move-up") state.row -= count;
  else if (command === "move-down") state.row += count;
  else if (command === "line-start") state.column = 0;
  else if (command === "line-end") state.column = terminalBufferLineEnd(term, state.row);
  else if (command === "document-start") {
    state.row = 0;
    state.column = 0;
  } else if (command === "document-end") {
    state.row = lastRow;
    state.column = terminalBufferLineEnd(term, state.row);
  } else if (command === "word-forward" || command === "word-backward" || command === "word-end") {
    const next = moveTerminalWord(term, state, command, count);
    state.row = next.row;
    state.column = next.column;
  } else if (command === "page-up" || command === "page-down") {
    const direction = command === "page-up" ? -1 : 1;
    state.row += direction * term.rows * count;
    term.scrollPages(direction * count);
  } else if (command === "half-page-up" || command === "half-page-down") {
    const direction = command === "half-page-up" ? -1 : 1;
    const lines = Math.max(1, Math.floor(term.rows / 2)) * count;
    state.row += direction * lines;
    term.scrollLines(direction * lines);
  } else if (command === "scroll-line-up" || command === "scroll-line-down") {
    const direction = command === "scroll-line-up" ? -1 : 1;
    state.row += direction * count;
    term.scrollLines(direction * count);
  } else if (command === "toggle-selection") {
    if (state.anchor && !state.lineSelection) state.anchor = null;
    else state.anchor = { row: state.row, column: state.column };
    state.lineSelection = false;
  } else if (command === "toggle-line-selection") {
    if (state.anchor && state.lineSelection) state.anchor = null;
    else state.anchor = { row: state.row, column: 0 };
    state.lineSelection = Boolean(state.anchor);
  } else if (command === "clear-selection") {
    state.anchor = null;
    state.lineSelection = false;
  } else if (command === "yank") {
    copyText = state.anchor
      ? term.getSelection()
      : term.buffer.active.getLine(state.row)?.translateToString(true) ?? "";
    state.anchor = null;
    state.lineSelection = false;
  } else if (command === "open-search") search = "open";
  else if (command === "find-next") search = "next";
  else if (command === "find-previous") search = "previous";

  const clamped = clampLocalNavigationState(term, state);
  renderTerminalLocalSelection(term, clamped);
  return { state: clamped, copyText, search };
}

function clampLocalNavigationState(term: XTerm, current: LocalNavigationState | null): LocalNavigationState {
  const buffer = term.buffer.active;
  const lastRow = Math.max(0, buffer.length - 1);
  const initialRow = Math.min(lastRow, Math.max(0, buffer.baseY + buffer.cursorY));
  const row = Math.min(lastRow, Math.max(0, current?.row ?? initialRow));
  const column = Math.min(terminalBufferLineEnd(term, row), Math.max(0, current?.column ?? buffer.cursorX));
  const anchor = current?.anchor
    ? {
      row: Math.min(lastRow, Math.max(0, current.anchor.row)),
      column: Math.min(term.cols - 1, Math.max(0, current.anchor.column)),
    }
    : null;
  return { row, column, anchor, lineSelection: Boolean(anchor && current?.lineSelection) };
}

function renderTerminalLocalSelection(term: XTerm, state: LocalNavigationState) {
  const viewport = term.buffer.active.viewportY;
  if (state.row < viewport) term.scrollToLine(state.row);
  else if (state.row >= viewport + term.rows) term.scrollToLine(Math.max(0, state.row - term.rows + 1));

  if (!state.anchor) {
    term.select(state.column, state.row, 1);
    return;
  }
  if (state.lineSelection) {
    term.selectLines(Math.min(state.anchor.row, state.row), Math.max(state.anchor.row, state.row));
    return;
  }
  const anchorIndex = state.anchor.row * term.cols + state.anchor.column;
  const cursorIndex = state.row * term.cols + state.column;
  const startIndex = Math.min(anchorIndex, cursorIndex);
  term.select(startIndex % term.cols, Math.floor(startIndex / term.cols), Math.abs(cursorIndex - anchorIndex) + 1);
}

function terminalBufferLineEnd(term: XTerm, row: number): number {
  const text = term.buffer.active.getLine(row)?.translateToString(true) ?? "";
  return Math.min(term.cols - 1, Math.max(0, text.length - 1));
}

function moveTerminalWord(
  term: XTerm,
  position: LocalNavigationPosition,
  command: "word-forward" | "word-backward" | "word-end",
  count: number,
): LocalNavigationPosition {
  const next = { ...position };
  for (let index = 0; index < count; index += 1) {
    const line = term.buffer.active.getLine(next.row)?.translateToString(true) ?? "";
    if (command === "word-backward") {
      const prefix = line.slice(0, Math.max(0, next.column));
      const match = prefix.match(/\w+\W*$/);
      if (match?.index !== undefined) next.column = match.index;
      else if (next.row > 0) {
        next.row -= 1;
        next.column = terminalBufferLineEnd(term, next.row);
      } else next.column = 0;
      continue;
    }
    const suffix = line.slice(Math.min(line.length, next.column + 1));
    const match = command === "word-end" ? suffix.match(/\w\b/) : suffix.match(/\w+/);
    if (match?.index !== undefined) {
      next.column += match.index + (command === "word-end" ? 1 : 1);
    } else if (next.row < term.buffer.active.length - 1) {
      next.row += 1;
      next.column = command === "word-end" ? terminalBufferLineEnd(term, next.row) : 0;
    } else next.column = terminalBufferLineEnd(term, next.row);
  }
  return next;
}

function writeTerminalEvent(term: XTerm, event: SessionEvent) {
  if (!event.text || event.direction === "outbound") return;
  if (event.direction === "system" || event.stream === "control" || event.stream === "audit") {
    term.writeln(`\x1b[38;5;245m${event.text}\x1b[0m`);
    return;
  }
  term.write(event.text);
}

function formatTerminalCanvasError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
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
