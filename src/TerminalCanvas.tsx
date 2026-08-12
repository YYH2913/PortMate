import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { CSSProperties } from "react";
import { AlignLeft, ArrowDownToLine, Binary, CaseSensitive, ChevronDown, ChevronUp, Columns2, CornerDownLeft, KeyRound, ListOrdered, Regex, Search, SendHorizontal, Trash2, WholeWord, X } from "lucide-react";
import { ClipboardAddon } from "@xterm/addon-clipboard";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import type { ISearchOptions } from "@xterm/addon-search";
import { SerializeAddon } from "@xterm/addon-serialize";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal as XTerm } from "@xterm/xterm";
import type { IMarker, ITheme } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { listen } from "@tauri-apps/api/event";
import { invokeBackend, isBackendAvailable } from "./api";
import { formatBytes } from "./display-formatters";
import { emptyOneKeyPromptDetectionState, oneKeyPromptCandidates, oneKeyPromptStateFromEvents, reduceOneKeyPromptDetection } from "./one-key-completion-state";
import type { OneKeyPromptDetectionState, OneKeyPromptField, OneKeyTerminalPrompt } from "./one-key-completion-state";
import type { SyncInputOrigin } from "./sync-input-state";
import { createWriteOnlyClipboardProvider } from "./terminal-clipboard";
import {
  emptyTerminalCompletionInputState,
  reduceTerminalCompletionInput,
  terminalCompletionSourceLabel,
  terminalCompletionSuggestions,
  terminalCompletionSupported,
  terminalCompletionUsageHint,
} from "./terminal-completion-state";
import type {
  TerminalCompletionInputState,
  TerminalCompletionSuggestion,
} from "./terminal-completion-state";
import { defaultTerminalCompletionPreferences, terminalCompletionPreferencesFromSettings } from "./terminal-completion-prefs";
import type { TerminalCompletionPreferences, TerminalCompletionQuickCommand } from "./terminal-completion-prefs";
import { createTerminalFreeInputPayload, cutTerminalFreeInputRange, MAX_TERMINAL_FREE_INPUT_CHARACTERS, normalizeTerminalFreeInput, terminalFreeInputCharacterCount, TERMINAL_FREE_INPUT_REQUEST_EVENT } from "./terminal-free-input";
import { TERMINAL_TEXT_EXPORT_REQUEST_EVENT } from "./terminal-export-event";
import type { TerminalTextExportRequestDetail } from "./terminal-export-event";
import { extractTerminalBufferText, extractTerminalSelectionText, MAX_TERMINAL_EXPORT_BYTES } from "./terminal-export-state";
import { resolveTerminalBufferAction, terminalBufferShortcut, TERMINAL_BUFFER_ACTION_REQUEST_EVENT } from "./terminal-buffer-event";
import type { TerminalBufferActionRequestDetail, TerminalBufferType } from "./terminal-buffer-event";
import { terminalBlockSelectionMouseEventInit, terminalSelectionShortcut, TERMINAL_SELECTION_REQUEST_EVENT } from "./terminal-selection-event";
import type { TerminalSelectionRequestDetail } from "./terminal-selection-event";
import { MAX_TERMINAL_GOTO_LINE_QUERY_LENGTH, resolveTerminalGotoLine, terminalGotoCurrentLine, terminalGotoLineStatus, terminalGotoViewportLine } from "./terminal-goto-line";
import type { TerminalGotoLineResolution } from "./terminal-goto-line";
import { TERMINAL_GOTO_LINE_REQUEST_EVENT } from "./terminal-goto-line-event";
import { emptyTerminalKeySequenceState, resolveTerminalKeyModeEvent, terminalKeyModeCursorStyle } from "./terminal-key-mode";
import type { TerminalKeyMode, TerminalKeySequenceState, TerminalLocalCommand } from "./terminal-key-mode";
import { isTerminalFindShortcut, MAX_TERMINAL_SEARCH_QUERY_LENGTH, terminalSearchResultLabel, terminalSearchSeed, TERMINAL_SEARCH_REQUEST_EVENT } from "./terminal-search";
import type { TerminalSearchResult } from "./terminal-search";
import { normalizeTerminalProfileSettings, shouldEnableTerminalWebgl } from "./terminal-settings-state";
import {
  MAX_TERMINAL_SEMANTIC_LINE_CHARACTERS,
  terminalSemanticHighlightingEnabled,
  terminalSemanticHighlightingSupported,
  terminalSemanticTokens,
} from "./terminal-semantic-highlighting";
import type { TerminalSemanticTokenKind } from "./terminal-semantic-highlighting";
import { rememberTerminalEventId, settleTerminalEventId, terminalStateCache } from "./terminal-state-cache";
import {
  MAX_TERMINAL_TIMESTAMPS,
  changedAlternateTerminalRows,
  formatTerminalTimestampClock,
  normalizeTerminalTimestamps,
  rebaseTerminalTimestamps,
  resizeAlternateTerminalTimestamps,
  updateAlternateTerminalTimestamps,
  visibleTerminalTimestamps,
} from "./terminal-timestamp-state";
import type { TerminalTimestampEntry, VisibleTerminalTimestamp } from "./terminal-timestamp-state";
import {
  clearTerminalByteCache,
  readTerminalDisplayMode,
  subscribeTerminalByteCache,
  terminalByteBufferStats,
  terminalByteCacheSnapshot,
  writeTerminalDisplayMode,
} from "./terminal-byte-state";
import type { TerminalByteSelection, TerminalDisplayMode } from "./terminal-byte-state";
import { isTerminalMouseReport, reduceTerminalMouseEncoding, terminalMouseEncodingSequence } from "./terminal-mouse";
import type { TerminalMouseEncoding } from "./terminal-mouse";
import { applyTerminalPresentation, normalizeTerminalTheme, terminalTheme } from "./terminal-theme";
import { openTerminalWebLink } from "./terminal-web-link";
import type { OneKeySummary, SessionEvent, SessionSummary } from "./types";

type TerminalCanvasProps = {
  viewId?: string;
  active?: SessionSummary;
  events: SessionEvent[];
  focused?: boolean;
  oneKeys?: readonly OneKeySummary[];
  oneKeyCompletionEnabled?: boolean;
  completionSettings?: unknown;
  completionHistory?: readonly string[];
  completionQuickCommands?: readonly TerminalCompletionQuickCommand[];
  mouseReporting?: boolean;
  copyOnSelect?: boolean;
  blockSelection?: boolean;
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
const LazyTerminalByteInspector = lazy(() => import("./TerminalByteInspector"));
const EMPTY_ONE_KEYS: readonly OneKeySummary[] = [];
const EMPTY_COMPLETION_HISTORY: readonly string[] = [];
const EMPTY_COMPLETION_QUICK_COMMANDS: readonly TerminalCompletionQuickCommand[] = [];
let terminalInstanceSequence = 0;
type WebglAddonInstance = import("@xterm/addon-webgl").WebglAddon;
type LocalNavigationPosition = { row: number; column: number };
type LocalNavigationState = LocalNavigationPosition & {
  anchor: LocalNavigationPosition | null;
  lineSelection: boolean;
};
type TerminalGotoLineContext = {
  currentLine: number;
  lineCount: number;
  originViewport: number;
  resumeFreeInputSource: "manual" | "normal" | null;
  resumeFreeInputValue: string;
};
type TerminalTimestampMarker = {
  marker: IMarker;
  ts: string;
};
type TerminalTimestampViewport = {
  bufferType: "normal" | "alternate";
  screenTop: number;
  cellHeight: number;
  entries: VisibleTerminalTimestamp[];
};
type TerminalSemanticCell = {
  row: number;
  column: number;
  width: number;
};
type TerminalSemanticLogicalLine = {
  text: string;
  cells: TerminalSemanticCell[];
  nextRow: number;
};

const terminalSearchDecorations: NonNullable<ISearchOptions["decorations"]> = {
  matchBackground: "#284457",
  matchBorder: "#68a7ff",
  matchOverviewRuler: "#68a7ff",
  activeMatchBackground: "#f4b860",
  activeMatchBorder: "#ffffff",
  activeMatchColorOverviewRuler: "#f4b860",
};
const emptyTerminalTimestampViewport: TerminalTimestampViewport = {
  bufferType: "normal",
  screenTop: 0,
  cellHeight: 0,
  entries: [],
};

function readTerminalSemanticLogicalLine(term: XTerm, startRow: number): TerminalSemanticLogicalLine {
  const buffer = term.buffer.active;
  const characters: string[] = [];
  const cells: TerminalSemanticCell[] = [];
  let row = startRow;
  let oversized = false;

  while (row < buffer.length) {
    const line = buffer.getLine(row);
    if (!line || (row > startRow && !line.isWrapped)) break;
    const physicalCharacters: string[] = [];
    const physicalCells: TerminalSemanticCell[] = [];
    const physicalContent: boolean[] = [];
    const columns = Math.min(term.cols, line.length);
    for (let column = 0; column < columns; column += 1) {
      const cell = line.getCell(column);
      if (!cell || cell.getWidth() === 0) continue;
      const chars = cell.getChars();
      const cellCharacters = Array.from(chars || " ");
      for (const character of cellCharacters) {
        physicalCharacters.push(character);
        physicalCells.push({ row, column, width: Math.max(1, cell.getWidth()) });
        physicalContent.push(Boolean(chars));
      }
    }
    while (physicalCharacters.length && !physicalContent.at(-1)) {
      physicalCharacters.pop();
      physicalCells.pop();
      physicalContent.pop();
    }
    characters.push(...physicalCharacters);
    cells.push(...physicalCells);
    row += 1;
    if (characters.length > MAX_TERMINAL_SEMANTIC_LINE_CHARACTERS) {
      oversized = true;
      break;
    }
  }

  if (oversized) {
    while (row < buffer.length && buffer.getLine(row)?.isWrapped) row += 1;
  }
  return { text: characters.join(""), cells, nextRow: Math.max(startRow + 1, row) };
}

function terminalSemanticCellSegments(
  cells: readonly TerminalSemanticCell[],
  start: number,
  end: number,
): TerminalSemanticCell[] {
  const segments: TerminalSemanticCell[] = [];
  for (const cell of cells.slice(start, end)) {
    const previous = segments.at(-1);
    if (previous && previous.row === cell.row && cell.column <= previous.column + previous.width) {
      previous.width = Math.max(previous.width, cell.column + cell.width - previous.column);
    } else {
      segments.push({ ...cell });
    }
  }
  return segments;
}

function alternateTerminalScreenSnapshot(term: XTerm): string[] {
  const buffer = term.buffer.active;
  if (buffer.type !== "alternate") return [];
  const snapshot: string[] = [];
  const reusableCell = buffer.getNullCell();
  for (let row = 0; row < term.rows; row += 1) {
    const line = buffer.getLine(row);
    if (!line) {
      snapshot.push("missing");
      continue;
    }
    const cells: unknown[] = [line.isWrapped];
    for (let column = 0; column < Math.min(term.cols, line.length); column += 1) {
      const cell = line.getCell(column, reusableCell);
      if (!cell) continue;
      const chars = cell.getChars();
      const width = cell.getWidth();
      if (!chars && width === 1 && cell.isAttributeDefault()) continue;
      const flags = [
        cell.isBold(),
        cell.isItalic(),
        cell.isDim(),
        cell.isUnderline(),
        cell.isBlink(),
        cell.isInverse(),
        cell.isInvisible(),
        cell.isStrikethrough(),
        cell.isOverline(),
      ].map(Number).join("");
      cells.push([
        column,
        chars,
        cell.getCode(),
        width,
        cell.getFgColorMode(),
        cell.getFgColor(),
        cell.getBgColorMode(),
        cell.getBgColor(),
        flags,
      ]);
    }
    snapshot.push(JSON.stringify(cells));
  }
  return snapshot;
}

function terminalSemanticColor(kind: TerminalSemanticTokenKind, theme: ITheme): string {
  switch (kind) {
    case "command": return theme.brightGreen ?? theme.green ?? "#86efac";
    case "option": return theme.brightBlue ?? theme.blue ?? "#93c5fd";
    case "string": return theme.brightYellow ?? theme.yellow ?? "#fde047";
    case "path": return theme.brightCyan ?? theme.cyan ?? "#67e8f9";
    case "address": return theme.cyan ?? "#5eead4";
    case "number": return theme.brightMagenta ?? theme.magenta ?? "#d8b4fe";
    case "variable": return theme.magenta ?? "#c084fc";
    case "operator": return theme.brightRed ?? theme.red ?? "#ff8a8a";
  }
}

export default function TerminalCanvas({
  viewId = "",
  active,
  events,
  focused = false,
  oneKeys = EMPTY_ONE_KEYS,
  oneKeyCompletionEnabled = true,
  completionSettings,
  completionHistory = EMPTY_COMPLETION_HISTORY,
  completionQuickCommands = EMPTY_COMPLETION_QUICK_COMMANDS,
  mouseReporting = true,
  copyOnSelect = true,
  blockSelection = false,
  keyMode = "remote",
  onKeyModeChange = () => {},
  onInput,
  onOneKeyCompletion,
}: TerminalCanvasProps) {
  const themeId = normalizeTerminalTheme(active?.profile.terminal.theme);
  const backgroundOpacity = active?.profile.terminal.backgroundOpacity ?? 100;
  const activeTerminalTheme = terminalTheme(themeId, backgroundOpacity);
  const canvasBackground = backgroundOpacity >= 100 ? activeTerminalTheme.background : "transparent";
  const sessionId = active?.profile.id ?? "";
  const displayModeKey = viewId || sessionId;
  const [displayMode, setDisplayMode] = useState<TerminalDisplayMode>(() => (
    typeof window === "undefined" ? "text" : readTerminalDisplayMode(window.localStorage, displayModeKey)
  ));
  const displayModeRef = useRef(displayMode);
  const [byteFollow, setByteFollow] = useState(true);
  const [byteSelection, setByteSelection] = useState<TerminalByteSelection | null>(null);
  const subscribeByteSnapshot = useCallback(
    (listener: () => void) => subscribeTerminalByteCache(sessionId, listener),
    [sessionId],
  );
  const getByteSnapshot = useCallback(() => terminalByteCacheSnapshot(sessionId), [sessionId]);
  const byteSnapshot = useSyncExternalStore(subscribeByteSnapshot, getByteSnapshot, getByteSnapshot);
  const byteStats = useMemo(() => terminalByteBufferStats(byteSnapshot), [byteSnapshot]);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XTerm | null>(null);
  const terminalMountGenerationRef = useRef(0);
  const configureWebglRef = useRef<(enabled: boolean) => void>(() => {});
  const refreshSemanticHighlightingRef = useRef<() => void>(() => {});
  const refreshTimestampGutterRef = useRef<() => void>(() => {});
  const refreshCompletionAnchorRef = useRef<() => void>(() => {});
  const writeEventRef = useRef<(event: SessionEvent) => boolean>(() => false);
  const searchRef = useRef<SearchAddon | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const gotoLineInputRef = useRef<HTMLInputElement | null>(null);
  const freeInputRef = useRef<HTMLTextAreaElement | null>(null);
  const seenEventsRef = useRef<Set<string>>(new Set());
  const pendingInputRef = useRef("");
  const inputFlushTimerRef = useRef<number | null>(null);
  const flushInputRef = useRef<() => void>(() => {});
  const lastSizeRef = useRef("");
  const focusedRef = useRef(focused);
  const fitAndReportRef = useRef<() => void>(() => {});
  const mouseReportingRef = useRef(mouseReporting);
  const copyOnSelectRef = useRef(copyOnSelect);
  const blockSelectionRef = useRef(blockSelection);
  const lastCopiedSelectionRef = useRef("");
  const onInputRef = useRef(onInput);
  const keyModeRef = useRef(keyMode);
  const previousKeyModeRef = useRef(keyMode);
  const onKeyModeChangeRef = useRef(onKeyModeChange);
  const keySequenceRef = useRef<TerminalKeySequenceState>(emptyTerminalKeySequenceState());
  const semanticHighlightingEnabledRef = useRef(terminalSemanticHighlightingEnabled(completionSettings));
  const semanticHighlightingSupportedRef = useRef(false);
  const semanticThemeRef = useRef(activeTerminalTheme);
  const localNavigationRef = useRef<LocalNavigationState | null>(null);
  const openSearchRef = useRef<() => void>(() => {});
  const openFreeInputRef = useRef<() => void>(() => {});
  const openGotoLineRef = useRef<() => void>(() => {});
  const runSearchRef = useRef<(direction: "next" | "previous") => void>(() => {});
  const oneKeyPromptStateRef = useRef<OneKeyPromptDetectionState>(emptyOneKeyPromptDetectionState());
  const oneKeyPromptSessionRef = useRef("");
  const dismissedOneKeyPromptEventsRef = useRef<Set<string>>(new Set());
  const completionInputRef = useRef<TerminalCompletionInputState>(emptyTerminalCompletionInputState);
  const completionSuggestionsRef = useRef<readonly TerminalCompletionSuggestion[]>([]);
  const completionSurfaceOpenRef = useRef(false);
  const completionSelectionRef = useRef(0);
  const acceptCompletionRef = useRef<(suggestion: TerminalCompletionSuggestion) => void>(() => {});
  const dismissCompletionRef = useRef<() => void>(() => {});
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchCaseSensitive, setSearchCaseSensitive] = useState(false);
  const [searchRegex, setSearchRegex] = useState(false);
  const [searchWholeWord, setSearchWholeWord] = useState(false);
  const [searchResult, setSearchResult] = useState<TerminalSearchResult | null>(null);
  const [searchInvalid, setSearchInvalid] = useState(false);
  const [gotoLineContext, setGotoLineContext] = useState<TerminalGotoLineContext | null>(null);
  const [gotoLineQuery, setGotoLineQuery] = useState("");
  const [freeInputSource, setFreeInputSource] = useState<"manual" | "normal" | null>(null);
  const [freeInputValue, setFreeInputValue] = useState("");
  const [oneKeyPrompt, setOneKeyPrompt] = useState<OneKeyTerminalPrompt | null>(null);
  const [oneKeyCompletionId, setOneKeyCompletionId] = useState("");
  const [oneKeyCompletionBusy, setOneKeyCompletionBusy] = useState(false);
  const [oneKeyCompletionError, setOneKeyCompletionError] = useState("");
  const [completionInput, setCompletionInput] = useState<TerminalCompletionInputState>(emptyTerminalCompletionInputState);
  const [completionDismissedLine, setCompletionDismissedLine] = useState("");
  const [completionSelection, setCompletionSelection] = useState(0);
  const [completionAnchor, setCompletionAnchor] = useState({ top: 8, cursorBottom: 0, shift: 0 });
  const [timestampViewport, setTimestampViewport] = useState<TerminalTimestampViewport>(emptyTerminalTimestampViewport);
  displayModeRef.current = displayMode;
  const gotoLineOpen = gotoLineContext !== null;
  const gotoLineResolution: TerminalGotoLineResolution = gotoLineContext
    ? resolveTerminalGotoLine(
      gotoLineQuery,
      gotoLineContext.currentLine,
      gotoLineContext.lineCount,
    )
    : { kind: "empty" };
  const completionPreferences: TerminalCompletionPreferences = useMemo(
    () => completionSettings === undefined
      ? defaultTerminalCompletionPreferences
      : terminalCompletionPreferencesFromSettings(completionSettings),
    [completionSettings],
  );
  const oneKeyCompletionCandidates = useMemo(
    () => oneKeyCompletionEnabled && active && oneKeyPrompt
      ? oneKeyPromptCandidates(oneKeys, active.profile.id, oneKeyPrompt)
      : [],
    [active?.profile.id, oneKeyCompletionEnabled, oneKeyPrompt, oneKeys],
  );
  const selectedOneKeyCompletion = oneKeyCompletionCandidates.find((oneKey) => oneKey.id === oneKeyCompletionId)
    ?? oneKeyCompletionCandidates[0]
    ?? null;
  const completionSupported = terminalCompletionSupported(active?.profile.kind);
  const semanticHighlightingSupported = terminalSemanticHighlightingSupported(active?.profile.kind);
  semanticHighlightingEnabledRef.current = terminalSemanticHighlightingEnabled(completionSettings);
  semanticHighlightingSupportedRef.current = semanticHighlightingSupported;
  semanticThemeRef.current = activeTerminalTheme;
  const completionContextActive = focused
    && displayMode !== "hex"
    && completionSupported
    && keyMode === "remote"
    && !searchOpen
    && !gotoLineOpen
    && !freeInputSource
    && !oneKeyPrompt
    && completionInput.synchronized
    && completionInput.line !== completionDismissedLine;
  const completionCandidates = useMemo(() => (
    completionContextActive
      ? terminalCompletionSuggestions({
        line: completionInput.line,
        preferences: completionPreferences,
        history: completionHistory,
        quickCommands: completionQuickCommands,
      }).slice(0, completionPreferences.listRows)
      : []
  ), [
    completionDismissedLine,
    completionHistory,
    completionInput,
    completionPreferences,
    completionQuickCommands,
    completionContextActive,
  ]);
  const completionUsageHint = useMemo(() => (
    completionContextActive
      ? terminalCompletionUsageHint({
        line: completionInput.line,
        preferences: completionPreferences,
      })
      : null
  ), [completionContextActive, completionInput.line, completionPreferences]);
  const activeCompletionIndex = completionCandidates.length
    ? Math.min(completionSelection, completionCandidates.length - 1)
    : 0;
  const selectedCompletion = completionCandidates[activeCompletionIndex] ?? null;
  const completionSurfaceOpen = Boolean(completionCandidates.length || completionUsageHint);
  completionSurfaceOpenRef.current = completionSurfaceOpen;
  const completionPanelHeight = completionSurfaceOpen
    ? completionCandidates.length * 30
      + (completionUsageHint ? 30 : 0)
      + (completionPreferences.previewMode === "input" && selectedCompletion ? 28 : 0)
      + 16
    : 0;
  refreshCompletionAnchorRef.current = () => {
    if (!completionSurfaceOpenRef.current) return;
    const term = termRef.current;
    const host = hostRef.current;
    const canvas = host?.parentElement;
    const screen = host?.querySelector<HTMLElement>(".xterm-screen");
    if (!term || !host || !canvas || !screen) return;
    const canvasRect = canvas.getBoundingClientRect();
    const hostRect = host.getBoundingClientRect();
    const screenRect = screen.getBoundingClientRect();
    const cellHeight = screenRect.height / Math.max(1, term.rows);
    const naturalCursorBottom = host.offsetTop + screenRect.top - hostRect.top
      + (term.buffer.active.cursorY + 1) * cellHeight;
    const reservedHeight = Math.min(completionPanelHeight, canvasRect.height * 0.45);
    const requiredShift = Math.max(
      0,
      naturalCursorBottom + reservedHeight + 10 - canvasRect.height,
    );
    const shift = Math.min(requiredShift, Math.max(0, naturalCursorBottom - 8));
    const cursorBottom = naturalCursorBottom - shift;
    const top = Math.max(8, Math.min(cursorBottom + 2, canvasRect.height - reservedHeight - 8));
    setCompletionAnchor((current) => (
      Math.abs(current.top - top) < 0.5
        && Math.abs(current.cursorBottom - cursorBottom) < 0.5
        && Math.abs(current.shift - shift) < 0.5
        ? current
        : { top, cursorBottom, shift }
    ));
  };
  onInputRef.current = onInput;
  focusedRef.current = focused;
  mouseReportingRef.current = mouseReporting;
  copyOnSelectRef.current = copyOnSelect;
  blockSelectionRef.current = blockSelection;
  keyModeRef.current = keyMode;
  onKeyModeChangeRef.current = onKeyModeChange;
  completionSuggestionsRef.current = completionCandidates;
  completionSelectionRef.current = activeCompletionIndex;
  const freeInputOpen = freeInputSource !== null;
  openSearchRef.current = () => {
    if (displayModeRef.current === "hex") return;
    closeTerminalGotoLine(true, false);
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
    if (displayModeRef.current === "hex") return;
    closeTerminalGotoLine(true, false);
    dismissOneKeyPrompt();
    searchRef.current?.clearDecorations();
    setSearchOpen(false);
    setSearchResult(null);
    setSearchInvalid(false);
    if (!freeInputOpen && !gotoLineContext?.resumeFreeInputSource) setFreeInputValue("");
    setFreeInputSource("manual");
    window.requestAnimationFrame(() => freeInputRef.current?.focus({ preventScroll: true }));
  };
  openGotoLineRef.current = () => {
    if (displayModeRef.current === "hex") return;
    const term = termRef.current;
    if (!term) return;
    if (gotoLineContext) {
      window.requestAnimationFrame(() => gotoLineInputRef.current?.focus({ preventScroll: true }));
      return;
    }
    searchRef.current?.clearDecorations();
    setSearchOpen(false);
    setSearchResult(null);
    setSearchInvalid(false);
    setFreeInputSource(null);
    const buffer = term.buffer.active;
    const lineCount = Math.max(1, buffer.length);
    const currentLine = localNavigationRef.current
      ? Math.min(lineCount, localNavigationRef.current.row + 1)
      : terminalGotoCurrentLine(buffer.viewportY, term.rows, lineCount);
    setGotoLineQuery("");
    setGotoLineContext({
      currentLine,
      lineCount,
      originViewport: buffer.viewportY,
      resumeFreeInputSource: freeInputSource,
      resumeFreeInputValue: freeInputValue,
    });
    window.requestAnimationFrame(() => gotoLineInputRef.current?.focus({ preventScroll: true }));
  };
  runSearchRef.current = (direction) => runTerminalSearch(direction);

  function storeCompletionInput(next: TerminalCompletionInputState) {
    completionInputRef.current = next;
    setCompletionInput(next);
  }

  function resetCompletionInput(synchronized = true) {
    storeCompletionInput({ line: "", synchronized });
    setCompletionDismissedLine("");
    completionSelectionRef.current = 0;
    setCompletionSelection(0);
    refreshSemanticHighlightingRef.current();
  }

  function updateCompletionInput(text: string) {
    const current = oneKeyPromptStateRef.current.prompt
      ? { line: "", synchronized: false }
      : completionInputRef.current;
    const next = reduceTerminalCompletionInput(current, text);
    storeCompletionInput(next);
    setCompletionDismissedLine("");
    completionSelectionRef.current = 0;
    setCompletionSelection(0);
    refreshSemanticHighlightingRef.current();
  }

  acceptCompletionRef.current = (suggestion) => {
    if (!active || !suggestion.appendText) return;
    const next = reduceTerminalCompletionInput(completionInputRef.current, suggestion.appendText);
    storeCompletionInput(next);
    setCompletionDismissedLine(suggestion.source === "history" || suggestion.source === "quick" ? next.line : "");
    completionSelectionRef.current = 0;
    setCompletionSelection(0);
    pendingInputRef.current += suggestion.appendText;
    if (inputFlushTimerRef.current !== null) {
      window.clearTimeout(inputFlushTimerRef.current);
      inputFlushTimerRef.current = null;
    }
    flushInputRef.current();
    window.requestAnimationFrame(() => termRef.current?.focus());
  };
  dismissCompletionRef.current = () => {
    setCompletionDismissedLine(completionInputRef.current.line);
    completionSelectionRef.current = 0;
    setCompletionSelection(0);
  };

  function applyOneKeyPromptState(state: OneKeyPromptDetectionState) {
    const previousEventId = oneKeyPromptStateRef.current.prompt?.eventId;
    oneKeyPromptStateRef.current = state;
    refreshSemanticHighlightingRef.current();
    const prompt = state.prompt && !dismissedOneKeyPromptEventsRef.current.has(state.prompt.eventId)
      ? state.prompt
      : null;
    if (previousEventId !== prompt?.eventId) {
      setOneKeyCompletionBusy(false);
      setOneKeyCompletionError("");
    }
    if (prompt) resetCompletionInput(false);
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

  function scrollToTerminalGotoLine(targetLine: number) {
    const term = termRef.current;
    if (!term || !gotoLineContext) return;
    term.scrollToLine(terminalGotoViewportLine(
      targetLine,
      term.rows,
      gotoLineContext.lineCount,
    ));
  }

  function previewTerminalGotoLine(query: string) {
    setGotoLineQuery(query);
    if (!gotoLineContext) return;
    const resolution = resolveTerminalGotoLine(
      query,
      gotoLineContext.currentLine,
      gotoLineContext.lineCount,
    );
    if (resolution.kind === "valid") scrollToTerminalGotoLine(resolution.targetLine);
    else termRef.current?.scrollToLine(gotoLineContext.originViewport);
  }

  function closeTerminalGotoLine(restoreViewport: boolean, focusTerminal = true) {
    if (restoreViewport && gotoLineContext) {
      termRef.current?.scrollToLine(gotoLineContext.originViewport);
    }
    const resumeFreeInputSource = gotoLineContext?.resumeFreeInputSource ?? null;
    const resumeFreeInputValue = gotoLineContext?.resumeFreeInputValue ?? "";
    setGotoLineContext(null);
    setGotoLineQuery("");
    if (resumeFreeInputSource) {
      setFreeInputSource(resumeFreeInputSource);
      setFreeInputValue(resumeFreeInputValue);
      if (focusTerminal) {
        window.requestAnimationFrame(() => freeInputRef.current?.focus({ preventScroll: true }));
      }
    } else if (focusTerminal) {
      window.requestAnimationFrame(() => termRef.current?.focus());
    }
  }

  function submitTerminalGotoLine() {
    if (gotoLineResolution.kind !== "valid") return;
    scrollToTerminalGotoLine(gotoLineResolution.targetLine);
    closeTerminalGotoLine(false);
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

  function changeTerminalDisplayMode(next: TerminalDisplayMode) {
    setDisplayMode(next);
    displayModeRef.current = next;
    if (typeof window !== "undefined" && displayModeKey) {
      writeTerminalDisplayMode(window.localStorage, displayModeKey, next);
    }
    window.requestAnimationFrame(() => {
      fitAndReportRef.current();
      if (next !== "hex" && focused && !freeInputOpen && !gotoLineOpen && !searchOpen) {
        termRef.current?.focus();
      }
    });
  }

  useEffect(() => {
    const next = typeof window === "undefined"
      ? "text"
      : readTerminalDisplayMode(window.localStorage, displayModeKey);
    setDisplayMode(next);
    displayModeRef.current = next;
    setByteFollow(true);
    setByteSelection(null);
  }, [displayModeKey, sessionId]);

  useEffect(() => {
    if (!active || !hostRef.current) return;

    const host = hostRef.current;
    const mountGeneration = ++terminalMountGenerationRef.current;
    const terminalInstanceId = String(++terminalInstanceSequence);
    host.dataset.terminalInstanceId = terminalInstanceId;
    host.dataset.terminalReady = "false";
    const cachedState = terminalStateCache.get(active.profile.id);
    const terminalSettings = normalizeTerminalProfileSettings(active.profile.terminal);
    seenEventsRef.current = new Set(cachedState?.seenEventIds ?? []);
    lastSizeRef.current = "";
    lastCopiedSelectionRef.current = "";
    const term = new XTerm({
      allowProposedApi: true,
      allowTransparency: true,
      cols: cachedState?.cols ?? terminalSettings.cols,
      rows: cachedState?.rows ?? terminalSettings.rows,
      cursorBlink: true,
      cursorStyle: terminalKeyModeCursorStyle(keyModeRef.current),
      convertEol: false,
      drawBoldTextInBrightColors: true,
      fontFamily: terminalSettings.fontFamily,
      fontSize: terminalSettings.fontSize,
      minimumContrastRatio: 1,
      scrollback: terminalSettings.scrollback,
      theme: terminalTheme(active.profile.terminal.theme, terminalSettings.backgroundOpacity),
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
    term.loadAddon(new WebLinksAddon((event, uri) => {
      openTerminalWebLink(event, uri);
    }));
    term.loadAddon(new ClipboardAddon(undefined, createWriteOnlyClipboardProvider(navigator.clipboard)));
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      const mode = keyModeRef.current;
      const selectionAction = terminalSelectionShortcut(event, mode);
      if (selectionAction) {
        event.preventDefault();
        if (!event.repeat) {
          if (selectionAction === "select-all") term.selectAll();
          else {
            const selected = term.getSelection();
            if (selected) void navigator.clipboard?.writeText(selected).catch(() => {});
          }
        }
        return false;
      }
      const bufferAction = terminalBufferShortcut(event, mode);
      if (bufferAction) {
        event.preventDefault();
        if (!event.repeat) {
          const resolution = resolveTerminalBufferAction(bufferAction, terminalBufferType(term));
          if (resolution.ok) term.write(resolution.sequence, () => term.focus());
        }
        return false;
      }
      const completions = completionSuggestionsRef.current;
      if (mode === "remote" && completionSurfaceOpenRef.current && !event.altKey && !event.ctrlKey && !event.metaKey) {
        if (completions.length && (event.key === "ArrowDown" || event.key === "ArrowUp" || (event.key === "Tab" && event.shiftKey))) {
          event.preventDefault();
          const offset = event.key === "ArrowDown" ? 1 : -1;
          const next = (completionSelectionRef.current + offset + completions.length) % completions.length;
          completionSelectionRef.current = next;
          setCompletionSelection(next);
          return false;
        }
        if (completions.length && event.key === "Tab") {
          event.preventDefault();
          acceptCompletionRef.current(completions[completionSelectionRef.current] ?? completions[0]);
          return false;
        }
        if (event.key === "Escape") {
          event.preventDefault();
          dismissCompletionRef.current();
        }
      }
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
    host.dataset.terminalCursorStyle = terminalKeyModeCursorStyle(keyModeRef.current);
    host.dataset.terminalMouseReporting = mouseReportingRef.current ? "enabled" : "disabled";
    host.dataset.terminalRestored = cachedState ? "true" : "false";
    let mouseEncoding: TerminalMouseEncoding = cachedState?.mouseEncoding ?? "default";
    const mouseEncodingSetDisposable = term.parser.registerCsiHandler({ prefix: "?", final: "h" }, (params) => {
      mouseEncoding = reduceTerminalMouseEncoding(mouseEncoding, params, true);
      return false;
    });
    const mouseEncodingResetDisposable = term.parser.registerCsiHandler({ prefix: "?", final: "l" }, (params) => {
      mouseEncoding = reduceTerminalMouseEncoding(mouseEncoding, params, false);
      return false;
    });
    let terminalDisposed = false;
    let timestampFrame: number | null = null;
    const timestampMarkerLimit = Math.min(
      MAX_TERMINAL_TIMESTAMPS,
      terminalSettings.scrollback + Math.max(term.rows, terminalSettings.rows) + 1,
    );
    let timestampMarkers: TerminalTimestampMarker[] = [];
    const cachedAlternateTimestamp = normalizeTerminalTimestamps([
      { line: 0, ts: cachedState?.alternateTimestamp },
    ], 1)[0]?.ts ?? null;
    let alternateTimestamps = resizeAlternateTerminalTimestamps(
      cachedState?.alternateTimestamps,
      term.rows,
      cachedAlternateTimestamp,
    );
    let alternateLatestTimestamp: string | null = cachedAlternateTimestamp
      ?? alternateTimestamps.reduce<string | null>((latest, entry) => (
      !latest || entry.ts > latest ? entry.ts : latest
      ), null);
    let pendingRestoredTimestamps = normalizeTerminalTimestamps(cachedState?.timestamps);
    let drainEventWrites = () => {};
    let scheduleSemanticHighlighting = () => {};

    const compactTimestampMarkers = () => {
      timestampMarkers = timestampMarkers.filter((entry) => !entry.marker.isDisposed && entry.marker.line >= 0);
      while (timestampMarkers.length > timestampMarkerLimit) {
        timestampMarkers.shift()?.marker.dispose();
      }
    };
    const registerTimestampLine = (line: number, ts: string) => {
      if (term.buffer.active.type !== "normal") return;
      const normalized = normalizeTerminalTimestamps([{ line, ts }], 1)[0];
      if (!normalized) return;
      compactTimestampMarkers();
      const existing = timestampMarkers.find((entry) => entry.marker.line === normalized.line);
      if (existing) {
        existing.ts = normalized.ts;
        return;
      }
      const normal = term.buffer.normal;
      const cursorLine = normal.baseY + normal.cursorY;
      const marker = term.registerMarker(normalized.line - cursorLine);
      if (!marker || marker.line < 0) return;
      timestampMarkers.push({ marker, ts: normalized.ts });
      compactTimestampMarkers();
    };
    const registerTimestampRange = (startLine: number, endLine: number, ts: string) => {
      const lastLine = Math.max(0, Math.max(startLine, endLine));
      const firstLine = Math.max(0, Math.min(startLine, endLine), lastLine - timestampMarkerLimit + 1);
      registerTimestampLine(firstLine, ts);
    };
    const restoreTimestampMarkers = () => {
      if (term.buffer.active.type !== "normal" || !pendingRestoredTimestamps.length) return;
      const restored = pendingRestoredTimestamps;
      pendingRestoredTimestamps = [];
      for (const entry of restored) registerTimestampLine(entry.line, entry.ts);
    };
    const timestampSnapshot = (firstSerializedLine: number): TerminalTimestampEntry[] => {
      compactTimestampMarkers();
      return rebaseTerminalTimestamps([
        ...pendingRestoredTimestamps,
        ...timestampMarkers.map((entry) => ({ line: entry.marker.line, ts: entry.ts })),
      ], firstSerializedLine);
    };
    const renderTimestampGutter = () => {
      timestampFrame = null;
      if (terminalDisposed) return;
      compactTimestampMarkers();
      const buffer = term.buffer.active;
      const bufferType = buffer.type === "alternate" ? "alternate" : "normal";
      const screen = host.querySelector<HTMLElement>(".xterm-screen");
      const region = host.parentElement;
      const hostRect = host.getBoundingClientRect();
      const screenRect = screen?.getBoundingClientRect();
      const screenTop = screenRect ? host.offsetTop + screenRect.top - hostRect.top : 0;
      const cellHeight = screenRect ? screenRect.height / Math.max(1, term.rows) : 0;
      const entries = bufferType === "normal"
        ? visibleTerminalTimestamps(
          timestampMarkers.map((entry) => ({ line: entry.marker.line, ts: entry.ts })),
          buffer.viewportY,
          term.rows,
        )
        : visibleTerminalTimestamps(alternateTimestamps, 0, term.rows);
      host.dataset.terminalTimestampBuffer = bufferType;
      host.dataset.terminalTimestampCount = String(entries.length);
      host.dataset.terminalTimestampMarkerCount = String(timestampMarkers.length);
      host.dataset.terminalTimestampRows = String(term.rows);
      if (!region || !screen || cellHeight <= 0) return;
      const next: TerminalTimestampViewport = { bufferType, screenTop, cellHeight, entries };
      setTimestampViewport((current) => sameTerminalTimestampViewport(current, next) ? current : next);
    };
    const scheduleTimestampGutter = () => {
      if (terminalDisposed || timestampFrame !== null) return;
      timestampFrame = window.requestAnimationFrame(renderTimestampGutter);
    };
    refreshTimestampGutterRef.current = scheduleTimestampGutter;

    let restorePending = Boolean(cachedState);
    if (cachedState) {
      term.write(cachedState.serialized + terminalMouseEncodingSequence(mouseEncoding), () => {
        restorePending = false;
        restoreTimestampMarkers();
        scheduleTimestampGutter();
        drainEventWrites();
      });
    }
    term.open(host);
    if (!cachedState) {
      registerTimestampRange(0, term.buffer.normal.baseY + term.buffer.normal.cursorY, new Date().toISOString());
    }
    host.dataset.terminalBuffer = terminalBufferType(term);
    host.dataset.terminalHasSelection = "false";
    const bufferChangeDisposable = term.buffer.onBufferChange((buffer) => {
      host.dataset.terminalBuffer = buffer.type === "alternate" ? "alternate" : "normal";
      if (buffer.type === "alternate" && !restorePending) {
        alternateTimestamps = [];
        alternateLatestTimestamp = null;
      }
      restoreTimestampMarkers();
      scheduleSemanticHighlighting();
      scheduleTimestampGutter();
    });
    const pendingEventIds = new Set<string>();
    const pendingEventWrites: SessionEvent[] = [];
    let eventWriteActive = false;
    drainEventWrites = () => {
      if (terminalDisposed || restorePending || eventWriteActive) return;
      while (pendingEventWrites.length) {
        const event = pendingEventWrites.shift();
        if (!event) break;
        eventWriteActive = true;
        const beforeBuffer = term.buffer.active;
        const beforeBufferType = beforeBuffer.type === "alternate" ? "alternate" : "normal";
        const beforeLine = beforeBufferType === "normal"
          ? term.buffer.normal.baseY + term.buffer.normal.cursorY
          : beforeBuffer.cursorY;
        const beforeAlternateSnapshot = beforeBufferType === "alternate"
          ? alternateTerminalScreenSnapshot(term)
          : [];
        let callbackCompleted = false;
        let writeReturned = false;
        writeTerminalEvent(term, event, () => {
          if (event.text && event.direction !== "outbound") {
            const afterBuffer = term.buffer.active;
            if (afterBuffer.type === "normal") {
              const afterLine = term.buffer.normal.baseY + term.buffer.normal.cursorY;
              registerTimestampRange(beforeBufferType === "normal" ? beforeLine : afterLine, afterLine, event.ts);
            } else {
              const normalizedTimestamp = normalizeTerminalTimestamps([
                { line: 0, ts: event.ts },
              ], 1)[0]?.ts ?? null;
              const afterSnapshot = alternateTerminalScreenSnapshot(term);
              alternateLatestTimestamp = normalizedTimestamp ?? alternateLatestTimestamp;
              alternateTimestamps = beforeBufferType === "alternate"
                ? updateAlternateTerminalTimestamps(
                  alternateTimestamps,
                  term.rows,
                  changedAlternateTerminalRows(
                    beforeAlternateSnapshot,
                    afterSnapshot,
                    beforeLine,
                    afterBuffer.cursorY,
                  ),
                  event.ts,
                )
                : resizeAlternateTerminalTimestamps([], term.rows, event.ts);
            }
          }
          settleTerminalEventId(seenEventsRef.current, pendingEventIds, event.id);
          eventWriteActive = false;
          callbackCompleted = true;
          scheduleTimestampGutter();
          if (writeReturned) drainEventWrites();
        });
        writeReturned = true;
        if (!callbackCompleted) return;
      }
    };
    const writeEvent = (event: SessionEvent) => {
      if (!rememberTerminalEventId(seenEventsRef.current, pendingEventIds, event.id)) return false;
      pendingEventWrites.push(event);
      drainEventWrites();
      return true;
    };
    writeEventRef.current = writeEvent;
    let webglAddon: WebglAddonInstance | null = null;
    let webglContextLossDisposable: { dispose: () => void } | null = null;
    let webglGeneration = 0;
    const webglEnabled = shouldEnableTerminalWebgl(isBackendAvailable(), navigator.userAgent);
    const configureWebgl = (enabled: boolean) => {
      const generation = ++webglGeneration;
      if (!enabled) {
        webglContextLossDisposable?.dispose();
        webglContextLossDisposable = null;
        webglAddon?.dispose();
        webglAddon = null;
        host.dataset.terminalRenderer = "dom";
        host.dataset.terminalWebgl = webglEnabled ? "disabled-transparency" : "disabled-webkitgtk";
        return;
      }
      if (webglAddon) return;
      void import("@xterm/addon-webgl").then(({ WebglAddon }) => {
        if (terminalDisposed || generation !== webglGeneration || termRef.current !== term) return;
        const nextAddon = new WebglAddon();
        term.loadAddon(nextAddon);
        webglAddon = nextAddon;
        host.dataset.terminalRenderer = "webgl";
        host.dataset.terminalWebgl = "active";
        webglContextLossDisposable = nextAddon.onContextLoss(() => {
          const lostAddon = webglAddon;
          webglAddon = null;
          webglContextLossDisposable?.dispose();
          webglContextLossDisposable = null;
          lostAddon?.dispose();
          host.dataset.terminalRenderer = "dom";
          host.dataset.terminalWebgl = "fallback";
        });
      }).catch(() => {
        if (generation !== webglGeneration) return;
        webglContextLossDisposable?.dispose();
        webglContextLossDisposable = null;
        webglAddon?.dispose();
        webglAddon = null;
        host.dataset.terminalRenderer = "dom";
        host.dataset.terminalWebgl = "fallback";
      });
    };
    configureWebglRef.current = configureWebgl;
    configureWebgl(webglEnabled && terminalSettings.backgroundOpacity === 100);
    if (focused && displayModeRef.current !== "hex") term.focus();
    const fitAndReport = () => {
      fit.fit();
      scheduleTimestampGutter();
      const size = `${term.cols}x${term.rows}`;
      host.dataset.terminalSize = size;
      if (displayModeRef.current === "hex" || !focusedRef.current || lastSizeRef.current === size) return;
      lastSizeRef.current = size;
      if (isBackendAvailable()) {
        void invokeBackend("resize_session", {
          sessionId: active.profile.id,
          cols: term.cols,
          rows: term.rows,
        }).catch(() => {});
      }
    };
    fitAndReportRef.current = fitAndReport;
    queueMicrotask(fitAndReport);

    const resizeObserver = new ResizeObserver(fitAndReport);
    resizeObserver.observe(host);
    let semanticFrame: number | null = null;
    let semanticDecorations: Array<{ dispose: () => void }> = [];
    let semanticMarkers: Array<{ dispose: () => void }> = [];
    const clearSemanticHighlighting = () => {
      for (const decoration of semanticDecorations.splice(0)) decoration.dispose();
      for (const marker of semanticMarkers.splice(0)) marker.dispose();
      host.dataset.terminalSemanticDecorationCount = "0";
    };
    const renderSemanticHighlighting = () => {
      semanticFrame = null;
      clearSemanticHighlighting();
      if (!semanticHighlightingEnabledRef.current) {
        host.dataset.terminalSemanticHighlighting = "disabled";
        return;
      }
      if (!semanticHighlightingSupportedRef.current) {
        host.dataset.terminalSemanticHighlighting = "unsupported";
        return;
      }
      if (!completionInputRef.current.synchronized || oneKeyPromptStateRef.current.prompt) {
        host.dataset.terminalSemanticHighlighting = "paused";
        return;
      }
      const buffer = term.buffer.active;
      if (buffer.type !== "normal") {
        host.dataset.terminalSemanticHighlighting = "alternate";
        return;
      }

      const cursorRow = buffer.baseY + buffer.cursorY;
      const markerByRow = new Map<number, ReturnType<XTerm["registerMarker"]>>();
      let firstRow = buffer.viewportY;
      while (firstRow > 0 && buffer.getLine(firstRow)?.isWrapped) firstRow -= 1;
      const viewportEnd = Math.min(buffer.length, buffer.viewportY + term.rows);
      let row = firstRow;
      let decorationCount = 0;
      while (row < viewportEnd) {
        const logicalLine = readTerminalSemanticLogicalLine(term, row);
        row = logicalLine.nextRow;
        for (const token of terminalSemanticTokens(logicalLine.text)) {
          for (const segment of terminalSemanticCellSegments(logicalLine.cells, token.start, token.end)) {
            let marker = markerByRow.get(segment.row);
            if (marker === undefined) {
              marker = term.registerMarker(segment.row - cursorRow);
              markerByRow.set(segment.row, marker);
              if (marker) semanticMarkers.push(marker);
            }
            if (!marker) continue;
            const decoration = term.registerDecoration({
              marker,
              x: segment.column,
              width: segment.width,
              foregroundColor: terminalSemanticColor(token.kind, semanticThemeRef.current),
              layer: "top",
            });
            if (!decoration) continue;
            semanticDecorations.push(decoration);
            decorationCount += 1;
          }
        }
      }
      host.dataset.terminalSemanticHighlighting = "active";
      host.dataset.terminalSemanticDecorationCount = String(decorationCount);
    };
    scheduleSemanticHighlighting = () => {
      if (terminalDisposed || semanticFrame !== null) return;
      semanticFrame = window.requestAnimationFrame(renderSemanticHighlighting);
    };
    refreshSemanticHighlightingRef.current = scheduleSemanticHighlighting;
    const semanticWriteDisposable = term.onWriteParsed(() => {
      scheduleSemanticHighlighting();
      scheduleTimestampGutter();
      refreshCompletionAnchorRef.current();
    });
    const semanticScrollDisposable = term.onScroll(() => {
      scheduleSemanticHighlighting();
      scheduleTimestampGutter();
    });
    const semanticResizeDisposable = term.onResize(() => {
      if (term.buffer.active.type === "alternate") {
        alternateTimestamps = resizeAlternateTerminalTimestamps(
          alternateTimestamps,
          term.rows,
          alternateLatestTimestamp,
        );
      }
      scheduleSemanticHighlighting();
      scheduleTimestampGutter();
      refreshCompletionAnchorRef.current();
    });
    scheduleSemanticHighlighting();
    scheduleTimestampGutter();
    const flushInput = () => {
      inputFlushTimerRef.current = null;
      const text = pendingInputRef.current;
      pendingInputRef.current = "";
      if (text) {
        onInputRef.current(active.profile.id, text, "interactive");
      }
    };
    flushInputRef.current = flushInput;
    const inputDisposable = term.onData((text) => {
      if (keyModeRef.current !== "remote") return;
      if (isTerminalMouseReport(text)
        && (!mouseReportingRef.current || host.querySelector(".xterm-cursor-pointer"))) return;
      updateCompletionInput(text);
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
      host.dataset.terminalHasSelection = term.hasSelection() ? "true" : "false";
      if (keyModeRef.current !== "remote") return;
      if (!copyOnSelectRef.current) return;
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
          resetCompletionInput(false);
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
    const pauseCompletionOnPaste = () => {
      resetCompletionInput(false);
    };
    const forceBlockSelection = (event: MouseEvent) => {
      if (!blockSelectionRef.current || event.altKey || event.button !== 0 || !event.target) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const forceSelection = term.modes.mouseTrackingMode !== "none";
      event.target.dispatchEvent(new MouseEvent(
        "mousedown",
        terminalBlockSelectionMouseEventInit(event, forceSelection),
      ));
    };
    host.addEventListener("paste", pauseCompletionOnPaste, true);
    host.addEventListener("mousedown", forceBlockSelection, true);
    host.addEventListener("auxclick", pasteOnMiddleClick);

    termRef.current = term;
    let readyFrame = window.requestAnimationFrame(() => {
      readyFrame = window.requestAnimationFrame(() => {
        if (!terminalDisposed
          && termRef.current === term
          && (!import.meta.env.DEV || mountGeneration > 1)) {
          host.dataset.terminalReady = "true";
        }
      });
    });

    return () => {
      terminalDisposed = true;
      window.cancelAnimationFrame(readyFrame);
      if (host.dataset.terminalInstanceId === terminalInstanceId) host.dataset.terminalReady = "false";
      searchResultDisposable.dispose();
      inputDisposable.dispose();
      selectionDisposable.dispose();
      bufferChangeDisposable.dispose();
      host.removeEventListener("paste", pauseCompletionOnPaste, true);
      host.removeEventListener("mousedown", forceBlockSelection, true);
      host.removeEventListener("auxclick", pasteOnMiddleClick);
      if (inputFlushTimerRef.current !== null) {
        window.clearTimeout(inputFlushTimerRef.current);
        inputFlushTimerRef.current = null;
      }
      pendingInputRef.current = "";
      pendingEventWrites.length = 0;
      if (flushInputRef.current === flushInput) flushInputRef.current = () => {};
      resizeObserver.disconnect();
      semanticWriteDisposable.dispose();
      semanticScrollDisposable.dispose();
      semanticResizeDisposable.dispose();
      if (semanticFrame !== null) window.cancelAnimationFrame(semanticFrame);
      if (timestampFrame !== null) window.cancelAnimationFrame(timestampFrame);
      clearSemanticHighlighting();
      if (refreshSemanticHighlightingRef.current === scheduleSemanticHighlighting) {
        refreshSemanticHighlightingRef.current = () => {};
      }
      if (refreshTimestampGutterRef.current === scheduleTimestampGutter) {
        refreshTimestampGutterRef.current = () => {};
      }
      if (fitAndReportRef.current === fitAndReport) fitAndReportRef.current = () => {};
      if (writeEventRef.current === writeEvent) writeEventRef.current = () => false;
      mouseEncodingSetDisposable.dispose();
      mouseEncodingResetDisposable.dispose();
      webglContextLossDisposable?.dispose();
      if (restorePending || pendingEventIds.size) {
        for (const eventId of pendingEventIds) seenEventsRef.current.delete(eventId);
      } else {
        try {
          const serializedScrollback = Math.min(
            MAX_SERIALIZED_SCROLLBACK,
            active.profile.terminal.scrollback,
          );
          const normalBuffer = term.buffer.normal;
          const serializedLineCount = Math.min(
            normalBuffer.length,
            serializedScrollback + term.rows,
          );
          const firstSerializedLine = Math.max(0, normalBuffer.length - serializedLineCount);
          terminalStateCache.save(active.profile.id, {
            serialized: serialize.serialize({
              scrollback: serializedScrollback,
            }),
            cols: term.cols,
            rows: term.rows,
            seenEventIds: [...seenEventsRef.current],
            mouseEncoding,
            timestamps: timestampSnapshot(firstSerializedLine),
            alternateTimestamps: term.buffer.active.type === "alternate"
              ? alternateTimestamps
              : undefined,
            alternateTimestamp: term.buffer.active.type === "alternate"
              ? alternateLatestTimestamp ?? undefined
              : undefined,
          });
        } catch {
          // Serialization must not prevent terminal disposal.
        }
      }
      term.dispose();
      if (searchRef.current === search) searchRef.current = null;
      if (configureWebglRef.current === configureWebgl) configureWebglRef.current = () => {};
      webglGeneration += 1;
      webglContextLossDisposable?.dispose();
      webglAddon?.dispose();
      termRef.current = null;
    };
  }, [active?.profile.id]);

  useEffect(() => {
    setTimestampViewport(emptyTerminalTimestampViewport);
  }, [active?.profile.id]);

  useEffect(() => {
    refreshSemanticHighlightingRef.current();
  }, [
    completionInput.synchronized,
    completionSettings,
    semanticHighlightingSupported,
    oneKeyPrompt?.eventId,
    themeId,
  ]);

  useEffect(() => {
    if (!active) return;
    const term = termRef.current;
    const host = hostRef.current;
    if (!term || !host) return;
    const normalized = normalizeTerminalProfileSettings(active.profile.terminal);
    const appliedTheme = applyTerminalPresentation(term, normalized);
    host.dataset.terminalTheme = appliedTheme;
    host.dataset.terminalOpacity = String(normalized.backgroundOpacity);
    configureWebglRef.current(
      shouldEnableTerminalWebgl(isBackendAvailable(), navigator.userAgent)
        && normalized.backgroundOpacity === 100,
    );
    const frame = window.requestAnimationFrame(() => fitAndReportRef.current());
    return () => window.cancelAnimationFrame(frame);
  }, [
    active?.profile.id,
    active?.profile.terminal.fontFamily,
    active?.profile.terminal.fontSize,
    active?.profile.terminal.scrollback,
    active?.profile.terminal.theme,
    active?.profile.terminal.backgroundOpacity,
  ]);

  useEffect(() => {
    const sessionId = active?.profile.id ?? "";
    if (oneKeyPromptSessionRef.current !== sessionId) {
      oneKeyPromptSessionRef.current = sessionId;
      dismissedOneKeyPromptEventsRef.current.clear();
      setOneKeyCompletionId("");
    }
    applyOneKeyPromptState(sessionId
      ? oneKeyPromptStateFromEvents(events, sessionId)
      : emptyOneKeyPromptDetectionState());
  }, [active?.profile.id, events]);

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
    const requestGotoLine = () => {
      if (active && focused) openGotoLineRef.current();
    };
    window.addEventListener(TERMINAL_GOTO_LINE_REQUEST_EVENT, requestGotoLine);
    return () => window.removeEventListener(TERMINAL_GOTO_LINE_REQUEST_EVENT, requestGotoLine);
  }, [active?.profile.id, focused]);

  useEffect(() => {
    const requestFreeInput = () => {
      if (active && focused) openFreeInputRef.current();
    };
    window.addEventListener(TERMINAL_FREE_INPUT_REQUEST_EVENT, requestFreeInput);
    return () => window.removeEventListener(TERMINAL_FREE_INPUT_REQUEST_EVENT, requestFreeInput);
  }, [active?.profile.id, focused]);

  useEffect(() => {
    const requestExport = (event: Event) => {
      const detail = (event as CustomEvent<TerminalTextExportRequestDetail>).detail;
      if (!detail || typeof detail.respond !== "function" || !active || !focused || !viewId
        || detail.sessionId !== active.profile.id || detail.viewId !== viewId) return;
      const source = (detail as { source?: unknown }).source;
      if (source !== "buffer" && source !== "selection") {
        detail.respond({ ok: false, error: "不支持的终端文本导出来源。" });
        return;
      }
      const term = termRef.current;
      if (!term) {
        detail.respond({ ok: false, error: "终端尚未完成加载。" });
        return;
      }
      const extracted = source === "selection"
        ? extractTerminalSelectionText(term.getSelection())
        : extractTerminalBufferText(term.buffer.active);
      if (!extracted.ok) {
        detail.respond({
          ok: false,
          error: extracted.reason === "empty"
            ? source === "selection" ? "当前终端没有选中文本。" : "当前终端缓冲为空。"
            : `终端文本超过 ${MAX_TERMINAL_EXPORT_BYTES / (1024 * 1024)} MiB 导出上限。`,
        });
        return;
      }
      detail.respond({
        ok: true,
        payload: {
          sessionId: active.profile.id,
          viewId,
          source,
          text: extracted.text,
          bytes: extracted.bytes,
          logicalLines: extracted.logicalLines,
        },
      });
    };
    window.addEventListener(TERMINAL_TEXT_EXPORT_REQUEST_EVENT, requestExport);
    return () => window.removeEventListener(TERMINAL_TEXT_EXPORT_REQUEST_EVENT, requestExport);
  }, [active?.profile.id, focused, viewId]);

  useEffect(() => {
    const requestBufferAction = (event: Event) => {
      const detail = (event as CustomEvent<TerminalBufferActionRequestDetail>).detail;
      if (!detail || typeof detail.respond !== "function" || !active || !focused || !viewId
        || detail.sessionId !== active.profile.id || detail.viewId !== viewId) return;
      const action = (detail as { action?: unknown }).action;
      if (action !== "clear-scrollback" && action !== "clear-screen" && action !== "clear-all") {
        detail.respond({ ok: false, error: "不支持的终端缓冲操作。" });
        return;
      }
      const term = termRef.current;
      if (!term) {
        detail.respond({ ok: false, error: "终端尚未完成加载。" });
        return;
      }
      const bufferType = terminalBufferType(term);
      const resolution = resolveTerminalBufferAction(action, bufferType);
      if (!resolution.ok) {
        detail.respond({ ok: false, error: resolution.error });
        return;
      }
      term.write(resolution.sequence, () => {
        term.focus();
        detail.respond({
          ok: true,
          payload: { sessionId: active.profile.id, viewId, action, bufferType },
        });
      });
    };
    window.addEventListener(TERMINAL_BUFFER_ACTION_REQUEST_EVENT, requestBufferAction);
    return () => window.removeEventListener(TERMINAL_BUFFER_ACTION_REQUEST_EVENT, requestBufferAction);
  }, [active?.profile.id, focused, viewId]);

  useEffect(() => {
    const requestSelection = (event: Event) => {
      const detail = (event as CustomEvent<TerminalSelectionRequestDetail>).detail;
      if (!detail || typeof detail.respond !== "function" || !active || !focused || !viewId
        || detail.sessionId !== active.profile.id || detail.viewId !== viewId) return;
      const action = (detail as { action?: unknown }).action;
      if (action !== "read" && action !== "copy" && action !== "select-all" && action !== "clear") {
        detail.respond({ ok: false, error: "不支持的终端选择命令。" });
        return;
      }
      const term = termRef.current;
      if (!term) {
        detail.respond({ ok: false, error: "终端尚未完成加载。" });
        return;
      }
      if (action === "read" || action === "copy") {
        const selection = term.getSelection() || null;
        if (action === "copy" && !selection) {
          detail.respond({ ok: false, error: "当前终端没有选中文本。" });
          return;
        }
        detail.respond({
          ok: true,
          payload: { sessionId: active.profile.id, viewId, action, selection },
        });
        return;
      }
      if (action === "select-all") term.selectAll();
      else term.clearSelection();
      term.focus();
      detail.respond({
        ok: true,
        payload: { sessionId: active.profile.id, viewId, action, selection: null },
      });
    };
    window.addEventListener(TERMINAL_SELECTION_REQUEST_EVENT, requestSelection);
    return () => window.removeEventListener(TERMINAL_SELECTION_REQUEST_EVENT, requestSelection);
  }, [active?.profile.id, focused, viewId]);

  useEffect(() => {
    resetCompletionInput();
    setFreeInputSource(null);
    setFreeInputValue("");
    setGotoLineContext(null);
    setGotoLineQuery("");
  }, [active?.profile.id, viewId]);

  useEffect(() => {
    const previousMode = previousKeyModeRef.current;
    previousKeyModeRef.current = keyMode;
    keySequenceRef.current = emptyTerminalKeySequenceState();
    const term = termRef.current;
    const host = hostRef.current;
    if (host) {
      host.dataset.terminalKeyMode = keyMode;
      host.dataset.terminalCursorStyle = terminalKeyModeCursorStyle(keyMode);
      host.dataset.terminalMouseReporting = mouseReporting ? "enabled" : "disabled";
    }
    if (term) term.options.cursorStyle = terminalKeyModeCursorStyle(keyMode);

    if (keyMode === "local" || keyMode === "command") {
      resetCompletionInput();
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
      resetCompletionInput();
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
  }, [active?.profile.id, keyMode, mouseReporting, viewId]);

  useEffect(() => {
    const measureFrame = window.requestAnimationFrame(() => refreshCompletionAnchorRef.current());
    return () => {
      window.cancelAnimationFrame(measureFrame);
    };
  }, [completionCandidates.length, completionInput.line, completionPanelHeight, completionPreferences.previewMode, completionUsageHint?.label]);

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
    const host = hostRef.current;
    if (host) host.dataset.terminalResizeOwner = focused ? "active" : "inactive";
    if (focused && displayMode !== "hex") {
      // Another view of this session may have resized the shared PTY while this view was inactive.
      lastSizeRef.current = "";
      termRef.current?.focus();
      fitAndReportRef.current();
    }
  }, [active?.profile.id, displayMode, focused]);

  useEffect(() => {
    if (!focused && gotoLineOpen) closeTerminalGotoLine(true, false);
  }, [focused, gotoLineOpen]);

  useEffect(() => {
    if (!active || !isBackendAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void listen<SessionEvent>("portmate-session-event", (event) => {
      if (disposed || event.payload.sessionId !== active.profile.id) return;
      const term = termRef.current;
      if (!term || !writeEventRef.current(event.payload)) return;
      applyOneKeyPromptState(reduceOneKeyPromptDetection(
        oneKeyPromptStateRef.current,
        event.payload,
      ));
    })
      .then((nextUnlisten) => {
        if (disposed) nextUnlisten();
        else unlisten = nextUnlisten;
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
      writeEventRef.current(event);
    }
  }, [events, active?.profile.id]);

  return (
    <div
      className={`terminal-canvas${active ? " has-terminal-view" : ""}${completionSurfaceOpen ? " completion-open" : ""}`}
      data-terminal-display-mode={active ? displayMode : undefined}
      data-completion-placement={completionSurfaceOpen ? "below" : undefined}
      data-completion-cursor-bottom={completionSurfaceOpen ? completionAnchor.cursorBottom : undefined}
      data-completion-shift={completionSurfaceOpen ? completionAnchor.shift : undefined}
      style={{
        "--terminal-background": canvasBackground ?? "#0d1117",
        "--terminal-completion-height": `${completionPanelHeight}px`,
        "--terminal-completion-shift": `${completionAnchor.shift}px`,
      } as CSSProperties}
    >
      {active ? (
        <>
          <div className="terminal-view-toolbar" role="toolbar" aria-label="终端显示">
            <div className="terminal-view-modes" role="group" aria-label="终端显示模式">
              <button type="button" className={displayMode === "text" ? "active" : ""} aria-label="文本" aria-pressed={displayMode === "text"} title="文本视图" onClick={() => changeTerminalDisplayMode("text")}><AlignLeft size={13} /><span>文本</span></button>
              <button type="button" className={displayMode === "hex" ? "active" : ""} aria-label="Hex" aria-pressed={displayMode === "hex"} title="Hex 视图" onClick={() => changeTerminalDisplayMode("hex")}><Binary size={13} /><span>Hex</span></button>
              <button type="button" className={displayMode === "split" ? "active" : ""} aria-label="对照" aria-pressed={displayMode === "split"} title="文本与 Hex 对照视图" onClick={() => changeTerminalDisplayMode("split")}><Columns2 size={13} /><span>对照</span></button>
            </div>
            <span className="terminal-byte-summary" title={`实时窗口 ${formatBytes(byteSnapshot.capturedBytes)} · ${byteSnapshot.frames.length} 帧${byteSnapshot.droppedFrames ? ` · 已淘汰 ${byteSnapshot.droppedFrames} 帧` : ""}${byteStats.omittedBytes ? ` · 帧内截断 ${formatBytes(byteStats.omittedBytes)}` : ""}`}>
              <span className="rx">RX {formatBytes(byteStats.rxBytes)}</span>
              <span className="tx">TX {formatBytes(byteStats.txBytes)}</span>
            </span>
            <button type="button" className={byteFollow ? "terminal-byte-tool active" : "terminal-byte-tool"} aria-label="跟随最新字节" aria-pressed={byteFollow} title="跟随最新字节" disabled={!byteSnapshot.frames.length} onClick={() => setByteFollow((current) => !current)}><ArrowDownToLine size={13} /></button>
            <button type="button" className="terminal-byte-tool" aria-label="清空实时字节" title="清空实时字节" disabled={!byteSnapshot.frames.length} onClick={() => { clearTerminalByteCache(active.profile.id); setByteSelection(null); setByteFollow(true); }}><Trash2 size={13} /></button>
          </div>
          <div className={`terminal-workspace mode-${displayMode}`}>
            <div className="terminal-terminal-region" aria-hidden={displayMode === "hex"} inert={displayMode === "hex"}>
              <div
                className="terminal-timestamp-gutter"
                role="list"
                aria-label="终端行时间戳"
                data-buffer-type={timestampViewport.bufferType}
                data-timestamp-count={timestampViewport.entries.length}
                style={{
                  "--terminal-timestamp-cell-height": `${timestampViewport.cellHeight}px`,
                } as CSSProperties}
              >
                {timestampViewport.entries.map((entry) => (
                  <time
                    key={`${entry.line}:${entry.ts}`}
                    role="listitem"
                    dateTime={entry.ts}
                    title={formatTerminalTimestampTitle(entry.ts)}
                    style={{
                      "--terminal-timestamp-offset": `${timestampViewport.screenTop + entry.row * timestampViewport.cellHeight}px`,
                    } as CSSProperties}
                  >
                    {formatTerminalTimestampClock(entry.ts)}
                  </time>
                ))}
              </div>
              <div ref={hostRef} className="terminal-host" inert={displayMode === "hex" || freeInputOpen || gotoLineOpen} />
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
            && !gotoLineOpen
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
          {completionSurfaceOpen ? (
            <section
              className="terminal-completion"
              aria-label="终端命令补全"
              data-preview-mode={completionPreferences.previewMode}
              style={{
                "--terminal-completion-rows": completionPreferences.listRows,
                top: `${completionAnchor.top}px`,
              } as CSSProperties}
            >
              {completionPreferences.previewMode === "input" && selectedCompletion ? (
                <div className="terminal-completion-preview" aria-hidden="true">
                  <code>{completionInput.line}</code><code>{selectedCompletion.appendText}</code>
                </div>
              ) : null}
              {completionUsageHint ? (
                <div className="terminal-completion-usage" aria-label="命令用法">
                  <span>用法</span>
                  <code title={completionUsageHint.label}>{completionUsageHint.label}</code>
                  <small>{completionUsageHint.detail}</small>
                </div>
              ) : null}
              {completionCandidates.length ? (
                <div className="terminal-completion-list" role="listbox" aria-label="命令候选">
                  {completionCandidates.map((suggestion, index) => (
                    <button
                      key={suggestion.id}
                      type="button"
                      role="option"
                      aria-selected={index === activeCompletionIndex}
                      className={index === activeCompletionIndex ? "active" : ""}
                      onMouseDown={(event) => event.preventDefault()}
                      onMouseEnter={() => {
                        completionSelectionRef.current = index;
                        setCompletionSelection(index);
                      }}
                      onClick={() => acceptCompletionRef.current(suggestion)}
                    >
                      <span>{terminalCompletionSourceLabel(suggestion.source)}</span>
                      <code>{suggestion.label}</code>
                      <small>{suggestion.detail}</small>
                    </button>
                  ))}
                </div>
              ) : null}
            </section>
          ) : null}
          {gotoLineContext ? (
            <form className="terminal-goto-line" aria-label="跳转到终端行" onSubmit={(event) => {
              event.preventDefault();
              submitTerminalGotoLine();
            }}>
              <ListOrdered size={14} aria-hidden="true" />
              <input
                ref={gotoLineInputRef}
                aria-label="终端行号"
                aria-invalid={gotoLineResolution.kind === "invalid" || gotoLineResolution.kind === "out-of-range"}
                value={gotoLineQuery}
                maxLength={MAX_TERMINAL_GOTO_LINE_QUERY_LENGTH}
                placeholder="行号，或 +20 / -10"
                autoComplete="off"
                spellCheck={false}
                onChange={(event) => previewTerminalGotoLine(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    event.stopPropagation();
                    closeTerminalGotoLine(true);
                  }
                }}
              />
              <span
                className={gotoLineResolution.kind === "invalid" || gotoLineResolution.kind === "out-of-range" ? "terminal-goto-line-status invalid" : "terminal-goto-line-status"}
                role="status"
                aria-live="polite"
              >{terminalGotoLineStatus(
                gotoLineResolution,
                gotoLineContext.currentLine,
                gotoLineContext.lineCount,
              )}</span>
              <div className="terminal-goto-line-controls">
                <button type="submit" aria-label="确认跳转" title="确认跳转" disabled={gotoLineResolution.kind !== "valid"}><CornerDownLeft size={15} /></button>
                <button type="button" aria-label="取消跳转" title="取消跳转" onClick={() => closeTerminalGotoLine(true)}><X size={15} /></button>
              </div>
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
            </div>
            {displayMode !== "text" ? (
              <Suspense fallback={<section className="terminal-byte-inspector" aria-label="终端字节检查器" aria-busy="true" />}>
                <LazyTerminalByteInspector
                  snapshot={byteSnapshot}
                  bytesPerRow={displayMode === "split" ? 8 : 16}
                  follow={byteFollow}
                  selection={byteSelection}
                  onFollowChange={setByteFollow}
                  onSelectionChange={setByteSelection}
                />
              </Suspense>
            ) : null}
          </div>
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

function terminalBufferType(term: XTerm): TerminalBufferType {
  return term.buffer.active.type === "alternate" ? "alternate" : "normal";
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

function writeTerminalEvent(term: XTerm, event: SessionEvent, onParsed: () => void) {
  if (!event.text || event.direction === "outbound") {
    onParsed();
    return;
  }
  if (event.direction === "system" || event.stream === "control" || event.stream === "audit") {
    term.writeln(`\x1b[38;5;245m${event.text}\x1b[0m`, onParsed);
    return;
  }
  term.write(event.text, onParsed);
}

function sameTerminalTimestampViewport(
  left: TerminalTimestampViewport,
  right: TerminalTimestampViewport,
): boolean {
  if (left.bufferType !== right.bufferType
    || Math.abs(left.screenTop - right.screenTop) >= 0.25
    || Math.abs(left.cellHeight - right.cellHeight) >= 0.25
    || left.entries.length !== right.entries.length) return false;
  return left.entries.every((entry, index) => {
    const other = right.entries[index];
    return entry.line === other?.line && entry.row === other.row && entry.ts === other.ts;
  });
}

function formatTerminalTimestampTitle(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : `${date.toLocaleString()} · ${value}`;
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
