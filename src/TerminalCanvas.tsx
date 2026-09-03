import { lazy, memo, startTransition, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { CSSProperties } from "react";
import { AlignLeft, ArrowDownToLine, Binary, CaseSensitive, ChevronDown, ChevronUp, Columns2, CornerDownLeft, KeyRound, ListOrdered, Lock, Regex, Search, SendHorizontal, Trash2, WholeWord, X } from "lucide-react";
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
import type { TerminalInputSendOptions } from "./terminal-input-pump";
import { createWriteOnlyClipboardProvider } from "./terminal-clipboard";
import {
  emptyTerminalCompletionInputState,
  indexTerminalCompletionHistory,
  reduceTerminalCompletionInput,
  reduceTerminalCompletionInputWithSubmissions,
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
import { mapTerminalSemanticRow, terminalSemanticCellSegments } from "./terminal-semantic-cells";
import type { TerminalSemanticBufferCell, TerminalSemanticCell } from "./terminal-semantic-cells";
import { rememberTerminalEventId, settleTerminalEventId, terminalEventSnapshotIds, terminalStateCache, terminalStateCacheKey } from "./terminal-state-cache";
import { activeModalLayer, MODAL_LAYER_ACTIVATED_EVENT } from "./modal-interaction-boundary";
import type { ModalLayerActivatedDetail } from "./modal-interaction-boundary";
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
import { isTerminalMouseReport, reduceTerminalMouseEncoding, terminalBinaryStringToBytes, terminalMouseEncodingSequence } from "./terminal-mouse";
import type { TerminalMouseEncoding } from "./terminal-mouse";
import { applyTerminalPresentation, normalizeTerminalTheme, terminalTheme } from "./terminal-theme";
import { openTerminalWebLink } from "./terminal-web-link";
import { flushTerminalByteEvents, subscribeTerminalByteEvents, subscribeTerminalLiveEvents } from "./terminal-byte-events";
import type { OneKeySummary, SessionEvent, SessionSummary, TerminalBytesEvent, TerminalLiveEvent } from "./types";

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
  onInput: (
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    options?: TerminalInputSendOptions,
  ) => void | Promise<void>;
  onCommandSubmit?: (sessionId: string, command: string) => void;
  onOneKeyCompletion?: (
    sessionId: string,
    oneKeyId: string,
    field: OneKeyPromptField,
    promptEventId: string,
  ) => Promise<void>;
};

const MAX_SERIALIZED_SCROLLBACK = 2000;
const TERMINAL_RESIZE_SETTLE_MS = 64;
const TERMINAL_SEMANTIC_SETTLE_MS = 180;
const TERMINAL_SEMANTIC_INPUT_IDLE_MS = 220;
const TERMINAL_COMPLETION_INPUT_DEBOUNCE_MS = 80;
const TERMINAL_ALTERNATE_SNAPSHOT_MIN_CHARACTERS = 256;
// Canonical live packets are emitted before the legacy session-event fallback.
// Keep only a short cross-channel grace period so a late raw packet cannot
// stall the terminal write queue for hundreds of milliseconds.
const TERMINAL_RAW_EVENT_CORRELATION_WAIT_MS = 32;
const TERMINAL_PRIVATE_INPUT_TIMEOUT_MS = 60_000;
// Keep an Enter-triggered output-follow transaction alive long enough for a
// slow serial/SSH prompt and the corresponding event reconciliation to land.
// A wheel event releases it immediately so manual scrolling always wins.
const TERMINAL_OUTPUT_FOLLOW_WINDOW_MS = 1_500;
const TERMINAL_WRITE_BATCH_MAX_BYTES = 64 * 1024;
const TERMINAL_WRITE_BATCH_MAX_FRAMES = 64;
const TERMINAL_SENSITIVE_INPUT_PATTERN = /\b(?:password|passphrase|secret|token|pin|otp|verification\s+code)\b[^\r\n]{0,80}[:?]\s*\S*$/i;
const LazyTerminalByteInspector = lazy(() => import("./TerminalByteInspector"));
const EMPTY_ONE_KEYS: readonly OneKeySummary[] = [];
const EMPTY_COMPLETION_HISTORY: readonly string[] = [];
const EMPTY_COMPLETION_QUICK_COMMANDS: readonly TerminalCompletionQuickCommand[] = [];
let terminalInstanceSequence = 0;
type WebglAddonInstance = import("@xterm/addon-webgl").WebglAddon;
type WebglAddonModule = typeof import("@xterm/addon-webgl");
let webglAddonModulePromise: Promise<WebglAddonModule> | null = null;
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
  lastLine: number;
};
type TerminalTimestampViewport = {
  bufferType: "normal" | "alternate";
  screenTop: number;
  cellHeight: number;
  entries: VisibleTerminalTimestamp[];
};
type TerminalSemanticLogicalLine = {
  text: string;
  cells: TerminalSemanticCell[];
  nextRow: number;
};
type TerminalSemanticDecorationLine = {
  marker: IMarker;
  fingerprint: string;
  decorations: Array<{ dispose: () => void }>;
  markers: IMarker[];
};
type PendingTerminalWrite = {
  event: SessionEvent;
  rawBytes?: Uint8Array;
  waitingForRaw?: boolean;
  fallbackTimer?: number;
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
    const bufferCells: TerminalSemanticBufferCell[] = [];
    const columns = Math.min(term.cols, line.length);
    for (let column = 0; column < columns; column += 1) {
      const cell = line.getCell(column);
      if (!cell) continue;
      const semanticCell: TerminalSemanticBufferCell = {
        column,
        width: cell.getWidth(),
        chars: cell.getChars(),
      };
      if (!cell.isFgDefault()) semanticCell.colorable = false;
      bufferCells.push(semanticCell);
    }
    const mapped = mapTerminalSemanticRow(row, bufferCells);
    characters.push(...Array.from(mapped.text));
    cells.push(...mapped.cells);
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

function terminalTextMayMoveRows(text: string): boolean {
  return /[\r\n\f\v\x1b]/u.test(text);
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
    case "success": return theme.brightGreen ?? theme.green ?? "#86efac";
    case "warning": return theme.brightYellow ?? theme.yellow ?? "#fde047";
    case "error": return theme.brightRed ?? theme.red ?? "#ff8a8a";
    case "info": return theme.brightBlue ?? theme.blue ?? "#93c5fd";
  }
}

function terminalSemanticPresentationFingerprint(
  term: XTerm,
  theme: ITheme,
  enabled: boolean,
  supported: boolean,
): string {
  const buffer = term.buffer.active;
  return String(enabled ? 1 : 0) + ":" + String(supported ? 1 : 0) + ":"
    + buffer.type + ":" + String(term.cols) + ":" + [
    theme.green, theme.brightGreen, theme.blue, theme.brightBlue,
    theme.yellow, theme.brightYellow, theme.cyan, theme.brightCyan,
    theme.magenta, theme.brightMagenta, theme.red, theme.brightRed,
  ].join("|");
}

function terminalSemanticLineFingerprint(line: TerminalSemanticLogicalLine): string {
  return `${line.text}\u0000${line.cells.map((cell) => cell.colorable === false ? "0" : "1").join("")}`;
}

function loadTerminalWebglAddon(): Promise<WebglAddonModule> {
  if (!webglAddonModulePromise) {
    webglAddonModulePromise = import("@xterm/addon-webgl").catch((error) => {
      webglAddonModulePromise = null;
      throw error;
    });
  }
  return webglAddonModulePromise;
}

function TerminalCanvas({
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
  onCommandSubmit,
  onOneKeyCompletion,
}: TerminalCanvasProps) {
  const themeId = normalizeTerminalTheme(active?.profile.terminal.theme);
  const backgroundOpacity = active?.profile.terminal.backgroundOpacity ?? 100;
  const activeTerminalTheme = terminalTheme(themeId, backgroundOpacity);
  const canvasBackground = backgroundOpacity >= 100 ? activeTerminalTheme.background : "transparent";
  const sessionId = active?.profile.id ?? "";
  const stateCacheKey = terminalStateCacheKey(sessionId, viewId);
  const displayModeKey = viewId || sessionId;
  const [displayMode, setDisplayMode] = useState<TerminalDisplayMode>(() => (
    typeof window === "undefined" ? "text" : readTerminalDisplayMode(window.localStorage, displayModeKey)
  ));
  const displayModeRef = useRef(displayMode);
  const [byteFollow, setByteFollow] = useState(true);
  const [byteSelection, setByteSelection] = useState<TerminalByteSelection | null>(null);
  const [manualPrivateInput, setManualPrivateInput] = useState(false);
  const [detectedPrivateInput, setDetectedPrivateInput] = useState(false);
  const manualPrivateInputRef = useRef(false);
  const detectedPrivateInputRef = useRef(false);
  const privateInputTimerRef = useRef<number | null>(null);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XTerm | null>(null);
  const terminalMountGenerationRef = useRef(0);
  const configureWebglRef = useRef<(enabled: boolean) => void>(() => {});
  const refreshSemanticHighlightingRef = useRef<() => void>(() => {});
  const refreshTimestampGutterRef = useRef<() => void>(() => {});
  const exportTimestampSnapshotRef = useRef<() => TerminalTimestampEntry[]>(() => []);
  const refreshCompletionAnchorRef = useRef<() => void>(() => {});
  const writeEventRef = useRef<(event: SessionEvent, awaitRaw?: boolean) => boolean>(() => false);
  const writeTerminalBytesRef = useRef<((event: TerminalBytesEvent) => boolean) | null>(null);
  const writeTerminalLiveRef = useRef<((event: TerminalLiveEvent) => boolean) | null>(null);
  const deferredTerminalBytesRef = useRef<TerminalBytesEvent[]>([]);
  const deferredTerminalLiveRef = useRef<TerminalLiveEvent[]>([]);
  const polledEventIdsRef = useRef(new Map<string, string[]>());
  const searchRef = useRef<SearchAddon | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const gotoLineInputRef = useRef<HTMLInputElement | null>(null);
  const freeInputRef = useRef<HTMLTextAreaElement | null>(null);
  const seenEventsRef = useRef<Set<string>>(new Set());
  const lastSizeRef = useRef("");
  const focusedRef = useRef(focused);
  const fitAndReportRef = useRef<() => void>(() => {});
  const mouseReportingRef = useRef(mouseReporting);
  const copyOnSelectRef = useRef(copyOnSelect);
  const blockSelectionRef = useRef(blockSelection);
  const lastCopiedSelectionRef = useRef("");
  const onInputRef = useRef(onInput);
  const onCommandSubmitRef = useRef(onCommandSubmit);
  const keyModeRef = useRef(keyMode);
  const previousKeyModeRef = useRef(keyMode);
  const onKeyModeChangeRef = useRef(onKeyModeChange);
  const keySequenceRef = useRef<TerminalKeySequenceState>(emptyTerminalKeySequenceState());
  const semanticHighlightingEnabledRef = useRef(terminalSemanticHighlightingEnabled(completionSettings));
  const semanticHighlightingSupportedRef = useRef(false);
  const semanticThemeRef = useRef(activeTerminalTheme);
  const localNavigationRef = useRef<LocalNavigationState | null>(null);
  const openSearchRef = useRef<() => void>(() => {});
  const openFreeInputRef = useRef<(value?: string) => void>(() => {});
  const openGotoLineRef = useRef<() => void>(() => {});
  const runSearchRef = useRef<(direction: "next" | "previous") => void>(() => {});
  const oneKeyPromptStateRef = useRef<OneKeyPromptDetectionState>(emptyOneKeyPromptDetectionState());
  const oneKeyPromptSessionRef = useRef("");
  const dismissedOneKeyPromptEventsRef = useRef<Set<string>>(new Set());
  const completionInputRef = useRef<TerminalCompletionInputState>(emptyTerminalCompletionInputState);
  const completionEnabledRef = useRef(false);
  const completionInputTimerRef = useRef<number | null>(null);
  const pendingCompletionInputRef = useRef<TerminalCompletionInputState | null>(null);
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
  // The native prompt validator needs the newest event id, but rendering the
  // completion panel for every byte of a prompt creates avoidable React work.
  // Keep detection state current in the ref and only publish visual changes.
  const [oneKeyCompletionId, setOneKeyCompletionId] = useState("");
  const [oneKeyCompletionBusy, setOneKeyCompletionBusy] = useState(false);
  const [oneKeyCompletionError, setOneKeyCompletionError] = useState("");
  const [completionInput, setCompletionInput] = useState<TerminalCompletionInputState>(emptyTerminalCompletionInputState);
  const [completionDismissedLine, setCompletionDismissedLine] = useState("");
  const completionDismissedLineRef = useRef("");
  const [completionSelection, setCompletionSelection] = useState(0);
  const [completionAnchor, setCompletionAnchor] = useState({ top: 8, cursorBottom: 0, shift: 0 });
  const [completionReadyKey, setCompletionReadyKey] = useState("");
  const [timestampViewport, setTimestampViewport] = useState<TerminalTimestampViewport>(emptyTerminalTimestampViewport);
  const privateInputActive = manualPrivateInput || detectedPrivateInput;
  displayModeRef.current = displayMode;
  completionDismissedLineRef.current = completionDismissedLine;

  function commitDetectedPrivateInput(value: boolean) {
    if (detectedPrivateInputRef.current === value) return;
    detectedPrivateInputRef.current = value;
    setDetectedPrivateInput(value);
  }

  function clearPrivateInput() {
    if (privateInputTimerRef.current !== null) {
      window.clearTimeout(privateInputTimerRef.current);
      privateInputTimerRef.current = null;
    }
    if (manualPrivateInputRef.current) {
      manualPrivateInputRef.current = false;
      setManualPrivateInput(false);
    }
    commitDetectedPrivateInput(false);
  }

  function toggleManualPrivateInput() {
    const next = !manualPrivateInputRef.current;
    manualPrivateInputRef.current = next;
    setManualPrivateInput(next);
    if (privateInputTimerRef.current !== null) window.clearTimeout(privateInputTimerRef.current);
    privateInputTimerRef.current = next
      ? window.setTimeout(clearPrivateInput, TERMINAL_PRIVATE_INPUT_TIMEOUT_MS)
      : null;
  }
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
  const completionHistoryIndex = useMemo(
    () => indexTerminalCompletionHistory(completionHistory),
    [completionHistory],
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
  completionEnabledRef.current = completionSupported && completionPreferences.enabled;
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
        historyIndex: completionHistoryIndex,
        quickCommands: completionQuickCommands,
      }).slice(0, completionPreferences.listRows)
      : []
  ), [
    completionDismissedLine,
    completionHistoryIndex,
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
  const completionGeometryKey = completionSurfaceOpen
    ? [active?.profile.id ?? "", completionInput.line, completionCandidates.length, completionUsageHint?.label ?? "", completionPanelHeight, completionPreferences.previewMode].join("\u0000")
    : "";
  const completionSurfaceVisible = completionSurfaceOpen && completionReadyKey === completionGeometryKey;
  const completionShiftTransform = completionSurfaceVisible && completionAnchor.shift > 0
    ? `translateY(-${completionAnchor.shift}px)`
    : undefined;
  refreshCompletionAnchorRef.current = () => {
    if (!completionSurfaceOpenRef.current) return;
    const term = termRef.current;
    const host = hostRef.current;
    const terminalRegion = host?.closest<HTMLElement>(".terminal-terminal-region");
    const screen = host?.querySelector<HTMLElement>(".xterm-screen");
    if (!term || !host || !terminalRegion || !screen) return;
    const canvasRect = terminalRegion.getBoundingClientRect();
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
  onCommandSubmitRef.current = onCommandSubmit;
  focusedRef.current = focused;
  mouseReportingRef.current = mouseReporting;
  copyOnSelectRef.current = copyOnSelect;
  blockSelectionRef.current = blockSelection;
  keyModeRef.current = keyMode;
  onKeyModeChangeRef.current = onKeyModeChange;
  completionSuggestionsRef.current = completionCandidates;
  completionSelectionRef.current = activeCompletionIndex;
  const freeInputOpen = freeInputSource !== null;
  const focusTerminalSurface = () => {
    if (!focusedRef.current) return;
    const gotoLineInput = gotoLineInputRef.current;
    if (gotoLineInput?.isConnected) {
      gotoLineInput.focus({ preventScroll: true });
      return;
    }
    const freeInput = freeInputRef.current;
    if (freeInput?.isConnected) {
      freeInput.focus({ preventScroll: true });
      return;
    }
    const searchInput = searchInputRef.current;
    if (searchInput?.isConnected) {
      searchInput.focus({ preventScroll: true });
      return;
    }
    if (displayModeRef.current !== "hex") termRef.current?.focus();
  };
  const scheduleTerminalSurfaceFocus = () => {
    window.requestAnimationFrame(focusTerminalSurface);
  };
  openSearchRef.current = () => {
    if (!focusedRef.current || displayModeRef.current === "hex") return;
    closeTerminalGotoLine(true, false);
    setFreeInputSource(null);
    setFreeInputValue("");
    const selection = terminalSearchSeed(termRef.current?.getSelection() ?? "");
    if (!searchOpen && selection) setSearchQuery(selection);
    setSearchInvalid(false);
    setSearchOpen(true);
    window.requestAnimationFrame(() => {
      if (!focusedRef.current) return;
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    });
  };
  openFreeInputRef.current = (value = "") => {
    if (!focusedRef.current || displayModeRef.current === "hex") return;
    closeTerminalGotoLine(true, false);
    dismissOneKeyPrompt();
    searchRef.current?.clearDecorations();
    setSearchOpen(false);
    setSearchResult(null);
    setSearchInvalid(false);
    if (value || (!freeInputOpen && !gotoLineContext?.resumeFreeInputSource)) {
      setFreeInputValue(normalizeTerminalFreeInput(value));
    }
    setFreeInputSource("manual");
    scheduleTerminalSurfaceFocus();
  };
  openGotoLineRef.current = () => {
    if (!focusedRef.current || displayModeRef.current === "hex") return;
    const term = termRef.current;
    if (!term) return;
    if (gotoLineContext) {
      scheduleTerminalSurfaceFocus();
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
    scheduleTerminalSurfaceFocus();
  };
  runSearchRef.current = (direction) => runTerminalSearch(direction);

  function storeCompletionInput(next: TerminalCompletionInputState) {
    cancelScheduledCompletionInput();
    completionInputRef.current = next;
    setCompletionInput(next);
  }

  function cancelScheduledCompletionInput() {
    if (completionInputTimerRef.current !== null) {
      window.clearTimeout(completionInputTimerRef.current);
      completionInputTimerRef.current = null;
    }
    pendingCompletionInputRef.current = null;
  }

  function storeCompletionInputDeferred(next: TerminalCompletionInputState) {
    completionInputRef.current = next;
    pendingCompletionInputRef.current = next;
    if (completionInputTimerRef.current !== null) {
      window.clearTimeout(completionInputTimerRef.current);
    }
    completionInputTimerRef.current = window.setTimeout(() => {
      completionInputTimerRef.current = null;
      const pending = pendingCompletionInputRef.current;
      pendingCompletionInputRef.current = null;
      if (!pending) return;
      startTransition(() => setCompletionInput(pending));
    }, TERMINAL_COMPLETION_INPUT_DEBOUNCE_MS);
  }

  function resetCompletionInput(synchronized = true) {
    storeCompletionInput({ line: "", synchronized });
    setCompletionDismissedLine((current) => current ? "" : current);
    if (completionSelectionRef.current !== 0) {
      completionSelectionRef.current = 0;
      setCompletionSelection(0);
    }
  }

  function updateCompletionInput(text: string): string[] {
    const current = oneKeyPromptStateRef.current.prompt
      ? { line: "", synchronized: false }
      : completionInputRef.current;
    const reduction = reduceTerminalCompletionInputWithSubmissions(current, text);
    const next = reduction.state;
    if (!completionEnabledRef.current) {
      cancelScheduledCompletionInput();
      completionInputRef.current = next;
      return reduction.submittedCommands;
    }
    if (/[\u0000-\u001f\u007f]/.test(text)) storeCompletionInput(next);
    else storeCompletionInputDeferred(next);
    if (completionDismissedLineRef.current) setCompletionDismissedLine("");
    if (completionSelectionRef.current !== 0) {
      completionSelectionRef.current = 0;
      setCompletionSelection(0);
    }
    return reduction.submittedCommands;
  }

  acceptCompletionRef.current = (suggestion) => {
    if (!active || !suggestion.appendText) return;
    const next = reduceTerminalCompletionInput(completionInputRef.current, suggestion.appendText);
    storeCompletionInput(next);
    setCompletionDismissedLine(suggestion.source === "history" || suggestion.source === "quick" ? next.line : "");
    completionSelectionRef.current = 0;
    setCompletionSelection(0);
    void onInputRef.current(
      active.profile.id,
      suggestion.appendText,
      "interactive",
      privateInputActive ? { sensitive: true } : undefined,
    );
    scheduleTerminalSurfaceFocus();
  };
  dismissCompletionRef.current = () => {
    setCompletionDismissedLine(completionInputRef.current.line);
    completionSelectionRef.current = 0;
    setCompletionSelection(0);
  };

  function applyOneKeyPromptState(state: OneKeyPromptDetectionState) {
    const previousPrompt = oneKeyPromptStateRef.current.prompt;
    oneKeyPromptStateRef.current = state;
    const prompt = state.prompt && !dismissedOneKeyPromptEventsRef.current.has(state.prompt.eventId)
      ? state.prompt
      : null;
    if (!previousPrompt && !prompt) return;
    const previousSignature = previousPrompt
      ? `${previousPrompt.field}\u0000${previousPrompt.line}`
      : "";
    const nextSignature = prompt
      ? `${prompt.field}\u0000${prompt.line}`
      : "";
    const requestChanged = previousPrompt?.eventId !== prompt?.eventId;
    if (requestChanged) {
      setOneKeyCompletionBusy(false);
      setOneKeyCompletionError("");
    }
    if (!requestChanged && previousSignature === nextSignature) return;
    if (prompt) resetCompletionInput(false);
    setOneKeyPrompt(prompt);
  }

  function dismissOneKeyPrompt() {
    const state = oneKeyPromptStateRef.current;
    const prompt = state.prompt;
    if (!prompt) {
      if (state.raw) oneKeyPromptStateRef.current = emptyOneKeyPromptDetectionState();
      return;
    }
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
    // Detection can advance through multiple transport chunks without a
    // corresponding React render. Use the latest validated event id.
    const promptEventId = oneKeyPromptStateRef.current.prompt?.eventId ?? oneKeyPrompt.eventId;
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
        scheduleTerminalSurfaceFocus();
      }
    } else if (focusTerminal) {
      scheduleTerminalSurfaceFocus();
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
    scheduleTerminalSurfaceFocus();
  }

  function submitTerminalFreeInput() {
    if (!active) return;
    const payload = createTerminalFreeInputPayload(freeInputValue);
    if (!payload) return;
    void onInputRef.current(
      active.profile.id,
      payload,
      "atomic",
      privateInputActive ? { sensitive: true } : undefined,
    );
    if (privateInputActive) clearPrivateInput();
    if (termRef.current?.buffer.active.type === "normal"
      && !terminalInputLooksSensitive(termRef.current, oneKeyPromptStateRef.current.prompt)) {
      const submitted = reduceTerminalCompletionInputWithSubmissions(
        emptyTerminalCompletionInputState,
        payload,
      ).submittedCommands;
      for (const command of submitted) onCommandSubmitRef.current?.(active.profile.id, command);
    }
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
    const cachedState = terminalStateCache.get(stateCacheKey);
    const terminalSettings = normalizeTerminalProfileSettings(active.profile.terminal);
    // A new XTerm instance must replay the current polled snapshot even when
    // the same session was already mounted in another view instance.
    polledEventIdsRef.current.delete(active.profile.id);
    seenEventsRef.current = new Set(cachedState?.seenEventIds ?? []);
    lastSizeRef.current = "";
    lastCopiedSelectionRef.current = "";
    const term = new XTerm({
      allowProposedApi: true,
      allowTransparency: true,
      cols: cachedState?.cols ?? terminalSettings.cols,
      rows: cachedState?.rows ?? terminalSettings.rows,
      cursorBlink: !window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      cursorStyle: terminalKeyModeCursorStyle(keyModeRef.current),
      convertEol: false,
      drawBoldTextInBrightColors: true,
      fontFamily: terminalSettings.fontFamily,
      fontSize: terminalSettings.fontSize,
      minimumContrastRatio: 1,
      scrollOnUserInput: true,
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
      if (!focusedRef.current) {
        event.preventDefault();
        return false;
      }
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
          if (resolution.ok) term.write(resolution.sequence, () => {
            if (focusedRef.current) term.focus();
          });
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
    host.dataset.terminalCursorColor = term.options.theme?.cursor ?? "#5eead4";
    host.dataset.terminalMouseReporting = mouseReportingRef.current ? "enabled" : "disabled";
    host.dataset.terminalRestored = cachedState ? "true" : "false";
    let mouseEncoding: TerminalMouseEncoding = cachedState?.mouseEncoding ?? "default";
    host.dataset.terminalMouseEncoding = mouseEncoding;
    const mouseEncodingSetDisposable = term.parser.registerCsiHandler({ prefix: "?", final: "h" }, (params) => {
      mouseEncoding = reduceTerminalMouseEncoding(mouseEncoding, params, true);
      host.dataset.terminalMouseEncoding = mouseEncoding;
      return false;
    });
    const mouseEncodingResetDisposable = term.parser.registerCsiHandler({ prefix: "?", final: "l" }, (params) => {
      mouseEncoding = reduceTerminalMouseEncoding(mouseEncoding, params, false);
      host.dataset.terminalMouseEncoding = mouseEncoding;
      return false;
    });
    let terminalDisposed = false;
    let timestampFrame: number | null = null;
    let resizeReportTimer: number | null = null;
    let enterScrollFrame: number | null = null;
    let outputFollowDeadline = 0;
    const timestampMarkerLimit = Math.min(
      MAX_TERMINAL_TIMESTAMPS,
      terminalSettings.scrollback + Math.max(term.rows, terminalSettings.rows) + 1,
    );
    let timestampMarkers: TerminalTimestampMarker[] = [];
    let normalTimestampAnchor: string | null = null;
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
    let sensitiveProbeRow = -1;
    let sensitiveProbeLine = "";
    let sensitiveProbeResult = false;

    const detectSensitiveInput = () => {
      if (oneKeyPromptStateRef.current.prompt) return true;
      const buffer = term.buffer.active;
      const row = buffer.type === "normal" ? buffer.baseY + buffer.cursorY : buffer.cursorY;
      const line = buffer.getLine(row)?.translateToString(true) ?? "";
      if (row === sensitiveProbeRow && line === sensitiveProbeLine) return sensitiveProbeResult;
      sensitiveProbeRow = row;
      sensitiveProbeLine = line;
      sensitiveProbeResult = terminalInputLineLooksSensitive(line);
      return sensitiveProbeResult;
    };

    const followTerminalOutput = () => {
      if (terminalDisposed || termRef.current !== term) return;
      const buffer = term.buffer.active;
      if (buffer.viewportY !== buffer.baseY) {
        term.scrollToBottom();
        scheduleTimestampGutter();
      }
      recordTerminalViewport();
    };

    const releaseOutputFollow = () => {
      outputFollowDeadline = 0;
      if (enterScrollFrame !== null) {
        window.cancelAnimationFrame(enterScrollFrame);
        enterScrollFrame = null;
      }
    };

    const keepTerminalAtOutput = () => {
      outputFollowDeadline = performance.now() + TERMINAL_OUTPUT_FOLLOW_WINDOW_MS;
      followTerminalOutput();
      // React completion state and xterm parser/layout work can both run after
      // the key event. Re-apply the explicit Enter boundary after those jobs,
      // without taking control away from a later user scroll.
      queueMicrotask(() => {
        if (outputFollowDeadline > performance.now()) followTerminalOutput();
      });
      if (enterScrollFrame !== null) return;
      enterScrollFrame = window.requestAnimationFrame(() => {
        enterScrollFrame = null;
        if (outputFollowDeadline > performance.now()) followTerminalOutput();
      });
    };

    const compactTimestampMarkers = () => {
      const retained: TerminalTimestampMarker[] = [];
      const disposed: TerminalTimestampMarker[] = [];
      for (const entry of timestampMarkers) {
        const line = entry.marker.line;
        if (!entry.marker.isDisposed && line >= 0) {
          entry.lastLine = line;
          retained.push(entry);
        } else {
          disposed.push(entry);
        }
      }
      disposed.sort((left, right) => left.lastLine - right.lastLine || left.marker.id - right.marker.id);
      normalTimestampAnchor = disposed.at(-1)?.ts ?? normalTimestampAnchor;
      timestampMarkers = retained.sort((left, right) => left.marker.line - right.marker.line);
      while (timestampMarkers.length > timestampMarkerLimit) {
        const removed = timestampMarkers.shift();
        if (!removed) break;
        normalTimestampAnchor = removed.ts;
        removed.marker.dispose();
      }
    };
    const registerTimestampLine = (line: number, ts: string): boolean => {
      if (term.buffer.active.type !== "normal") return false;
      const normalized = normalizeTerminalTimestamps([{ line, ts }], 1)[0];
      if (!normalized) return false;
      const latest = timestampMarkers.at(-1);
      if (latest && !latest.marker.isDisposed && latest.marker.line === normalized.line) {
        latest.lastLine = normalized.line;
        return false;
      }
      compactTimestampMarkers();
      const existing = timestampMarkers.find((entry) => entry.marker.line === normalized.line);
      if (existing) {
        return false;
      }
      const normal = term.buffer.normal;
      const cursorLine = normal.baseY + normal.cursorY;
      const marker = term.registerMarker(normalized.line - cursorLine);
      if (!marker || marker.line < 0) return false;
      timestampMarkers.push({ marker, ts: normalized.ts, lastLine: marker.line });
      compactTimestampMarkers();
      return true;
    };
    const trackTimestampLine = (line: number, ts: string): TerminalTimestampMarker | null => {
      if (term.buffer.active.type !== "normal") return null;
      const normalized = normalizeTerminalTimestamps([{ line, ts }], 1)[0];
      if (!normalized) return null;
      const normal = term.buffer.normal;
      const cursorLine = normal.baseY + normal.cursorY;
      const marker = term.registerMarker(normalized.line - cursorLine);
      return marker ? { marker, ts: normalized.ts, lastLine: marker.line } : null;
    };
    const commitTrackedTimestamp = (tracked: TerminalTimestampMarker, cursorLine: number): boolean => {
      const trackedLine = tracked.marker.line;
      if (tracked.marker.isDisposed || trackedLine < 0) {
        compactTimestampMarkers();
        const changed = normalTimestampAnchor !== tracked.ts;
        normalTimestampAnchor = tracked.ts;
        return changed;
      }
      tracked.lastLine = trackedLine;
      const existing = timestampMarkers.find((entry) => entry.marker.line === trackedLine);
      let changed = false;
      if (existing) {
        tracked.marker.dispose();
      } else {
        timestampMarkers.push(tracked);
        changed = true;
      }
      if (cursorLine !== trackedLine) {
        const nextLine = cursorLine > trackedLine ? trackedLine + 1 : cursorLine;
        changed = registerTimestampLine(nextLine, tracked.ts) || changed;
      }
      compactTimestampMarkers();
      return changed;
    };
    const registerTimestampRange = (startLine: number, endLine: number, ts: string): boolean => {
      const lastLine = Math.max(0, Math.max(startLine, endLine));
      const firstLine = Math.max(0, Math.min(startLine, endLine), lastLine - timestampMarkerLimit + 1);
      return registerTimestampLine(firstLine, ts);
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
        ...(normalTimestampAnchor ? [{ line: 0, ts: normalTimestampAnchor }] : []),
      ], firstSerializedLine);
    };
    const exportTimestampSnapshot = (): TerminalTimestampEntry[] => (
      term.buffer.active.type === "alternate"
        ? normalizeTerminalTimestamps(alternateTimestamps, Math.max(1, alternateTimestamps.length))
        : timestampSnapshot(0)
    );
    exportTimestampSnapshotRef.current = exportTimestampSnapshot;
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
          [
            ...timestampMarkers.map((entry) => ({ line: entry.marker.line, ts: entry.ts })),
            ...(normalTimestampAnchor ? [{ line: 0, ts: normalTimestampAnchor }] : []),
          ],
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
    const recordTerminalViewport = () => {
      const buffer = term.buffer.active;
      host.dataset.terminalViewportY = String(buffer.viewportY);
      host.dataset.terminalBaseY = String(buffer.baseY);
    };
    recordTerminalViewport();
    host.dataset.terminalBuffer = terminalBufferType(term);
    host.dataset.terminalHasSelection = "false";
    const bufferChangeDisposable = term.buffer.onBufferChange((buffer) => {
      host.dataset.terminalBuffer = buffer.type === "alternate" ? "alternate" : "normal";
      if (buffer.type === "alternate" && !restorePending) {
        alternateTimestamps = [];
        alternateLatestTimestamp = null;
      }
      recordTerminalViewport();
      restoreTimestampMarkers();
      scheduleSemanticHighlighting();
      scheduleTimestampGutter();
    });
    const pendingEventIds = new Set<string>();
    const pendingEventWrites: PendingTerminalWrite[] = [];
    const pendingTextEvents = new Map<string, PendingTerminalWrite>();
    let fastPathTextDecoder = new TextDecoder();
    let pendingEventWriteHead = 0;
    let eventWriteActive = false;
    let eventWriteDrainQueued = false;
    let scheduleEventWriteDrain = () => {};

    const compactPendingEventWrites = () => {
      if (pendingEventWriteHead === 0) return;
      if (pendingEventWriteHead < 256 && pendingEventWriteHead * 2 < pendingEventWrites.length) return;
      pendingEventWrites.splice(0, pendingEventWriteHead);
      pendingEventWriteHead = 0;
    };
    const queueEventWrite = (pending: PendingTerminalWrite) => {
      pendingEventWrites.push(pending);
    };
    const nextEventWriteBatch = (): PendingTerminalWrite[] | null => {
      const first = pendingEventWrites[pendingEventWriteHead];
      if (!first || first.waitingForRaw) return null;
      pendingEventWriteHead += 1;
      if (!first.rawBytes) return [first];

      const batch = [first];
      let byteLength = first.rawBytes.byteLength;
      while (batch.length < TERMINAL_WRITE_BATCH_MAX_FRAMES
        && byteLength < TERMINAL_WRITE_BATCH_MAX_BYTES) {
        const next = pendingEventWrites[pendingEventWriteHead];
        if (!next || next.waitingForRaw || !next.rawBytes || next.event.direction !== "inbound") break;
        if (byteLength + next.rawBytes.byteLength > TERMINAL_WRITE_BATCH_MAX_BYTES) break;
        pendingEventWriteHead += 1;
        batch.push(next);
        byteLength += next.rawBytes.byteLength;
      }
      return batch;
    };
    drainEventWrites = () => {
      if (terminalDisposed || restorePending || eventWriteActive) return;
      const batch = nextEventWriteBatch();
      if (!batch) return;
      compactPendingEventWrites();
      const first = batch[0];
      const last = batch.at(-1) ?? first;
      const rawBytes = first.rawBytes
        ? concatTerminalWriteBytes(batch.map((pending) => pending.rawBytes!))
        : undefined;
      const eventText = batch.map((pending) => pending.event.text ?? "").join("");
      const event = { ...last.event, text: eventText };
      eventWriteActive = true;
      const beforeBuffer = term.buffer.active;
      const beforeBufferType = beforeBuffer.type === "alternate" ? "alternate" : "normal";
      const beforeLine = beforeBufferType === "normal"
        ? term.buffer.normal.baseY + term.buffer.normal.cursorY
        : beforeBuffer.cursorY;
      const inspectAlternateRows = beforeBufferType === "alternate"
        && eventText.length >= TERMINAL_ALTERNATE_SNAPSHOT_MIN_CHARACTERS;
      const beforeAlternateSnapshot = inspectAlternateRows
        ? alternateTerminalScreenSnapshot(term)
        : [];
      const trackedNormalStart = eventText
        && event.direction !== "outbound"
        && beforeBufferType === "normal"
        && terminalTextMayMoveRows(eventText)
        ? trackTimestampLine(beforeLine, first.event.ts)
        : null;
      let callbackCompleted = false;
      let writeReturned = false;
      writeTerminalEvent(term, event, rawBytes, () => {
        let timestampChanged = false;
        if (eventText && event.direction !== "outbound") {
          const afterBuffer = term.buffer.active;
          if (afterBuffer.type === "normal") {
            const afterLine = term.buffer.normal.baseY + term.buffer.normal.cursorY;
            if (trackedNormalStart) {
              timestampChanged = commitTrackedTimestamp(trackedNormalStart, afterLine);
            } else {
              const firstChangedLine = beforeBufferType !== "normal"
                ? afterLine
                : afterLine > beforeLine ? beforeLine + 1 : afterLine;
              timestampChanged = registerTimestampRange(firstChangedLine, afterLine, first.event.ts);
            }
          } else {
            trackedNormalStart?.marker.dispose();
            const normalizedTimestamp = normalizeTerminalTimestamps([
              { line: 0, ts: first.event.ts },
            ], 1)[0]?.ts ?? null;
            alternateLatestTimestamp = normalizedTimestamp ?? alternateLatestTimestamp;
            if (beforeBufferType === "alternate") {
              const changedRows = inspectAlternateRows
                ? changedAlternateTerminalRows(
                  beforeAlternateSnapshot,
                  alternateTerminalScreenSnapshot(term),
                  beforeLine,
                  afterBuffer.cursorY,
                )
                : beforeLine === afterBuffer.cursorY && !terminalTextMayMoveRows(eventText)
                  ? []
                  : [...new Set([beforeLine, afterBuffer.cursorY])];
              alternateTimestamps = updateAlternateTerminalTimestamps(
                alternateTimestamps,
                term.rows,
                changedRows,
                first.event.ts,
              );
              timestampChanged = changedRows.length > 0;
            } else {
              alternateTimestamps = resizeAlternateTerminalTimestamps([], term.rows, first.event.ts);
              timestampChanged = alternateTimestamps.length > 0;
            }
          }
        }
        for (const pending of batch) {
          settleTerminalEventId(seenEventsRef.current, pendingEventIds, pending.event.id);
        }
        eventWriteActive = false;
        callbackCompleted = true;
        if (timestampChanged) scheduleTimestampGutter();
        if (writeReturned) drainEventWrites();
      });
      writeReturned = true;
      if (callbackCompleted) drainEventWrites();
    };
    scheduleEventWriteDrain = () => {
      if (terminalDisposed || eventWriteDrainQueued) return;
      eventWriteDrainQueued = true;
      // A Tauri event burst often arrives as several callbacks in one task.
      // Move only queue admission to a microtask so those callbacks share one
      // xterm write, while parser callbacks still drain synchronously.
      queueMicrotask(() => {
        eventWriteDrainQueued = false;
        drainEventWrites();
      });
    };
    const enqueueFallback = (pending: PendingTerminalWrite, rawMatched = false) => {
      pending.waitingForRaw = false;
      pending.fallbackTimer = undefined;
      pendingTextEvents.delete(pending.event.id);
      if (!rawMatched) fastPathTextDecoder = new TextDecoder();
      scheduleEventWriteDrain();
    };
    const writeEvent = (event: SessionEvent, awaitRaw = false) => {
      if (!event.id) return false;
      if (seenEventsRef.current.has(event.id)) return false;
      const isLiveInboundText = event.direction === "inbound"
        && Boolean(event.text)
        && (event.stream === "stdout" || event.stream === "stderr");
      if (!rememberTerminalEventId(seenEventsRef.current, pendingEventIds, event.id)) return false;
      if (isLiveInboundText && awaitRaw) {
        const pending: PendingTerminalWrite = { event, waitingForRaw: true };
        pending.fallbackTimer = window.setTimeout(() => enqueueFallback(pending), TERMINAL_RAW_EVENT_CORRELATION_WAIT_MS);
        pendingTextEvents.set(event.id, pending);
        queueEventWrite(pending);
      } else {
        queueEventWrite({ event });
      }
      scheduleEventWriteDrain();
      return true;
    };
    writeEventRef.current = writeEvent;
    const writeTerminalBytes = (bytesEvent: TerminalBytesEvent) => {
      if (bytesEvent.canonical) return false;
      if (bytesEvent.sessionId !== active.profile.id || bytesEvent.direction !== "inbound") return false;
      if (!bytesEvent.id || !Array.isArray(bytesEvent.bytes) || !bytesEvent.bytes.length
        || bytesEvent.bytes.some((byte) => !Number.isInteger(byte) || byte < 0 || byte > 0xff)) return false;
      const rawBytes = Uint8Array.from(bytesEvent.bytes);
      if (bytesEvent.eventId) {
        if (bytesEvent.truncated) {
          fastPathTextDecoder = new TextDecoder();
          return false;
        }
        const pendingText = pendingTextEvents.get(bytesEvent.eventId);
        if (pendingText) {
          if (pendingText.fallbackTimer !== undefined) window.clearTimeout(pendingText.fallbackTimer);
          pendingText.rawBytes = rawBytes;
          pendingText.event = {
            ...pendingText.event,
            text: fastPathTextDecoder.decode(rawBytes, { stream: true }),
            annotations: { ...pendingText.event.annotations, "terminalBytesFastPath": "true" },
          };
          enqueueFallback(pendingText, true);
          return true;
        }
        if (seenEventsRef.current.has(bytesEvent.eventId)) {
          fastPathTextDecoder.decode(rawBytes, { stream: true });
          return false;
        }
        const syntheticEvent: SessionEvent = {
          id: bytesEvent.eventId,
          sessionId: bytesEvent.sessionId,
          paneId: bytesEvent.sessionId + ":main",
          ts: bytesEvent.ts,
          direction: "inbound",
          stream: bytesEvent.stream,
          bytesRef: null,
          text: fastPathTextDecoder.decode(rawBytes, { stream: true }),
          annotations: { "terminalBytesFastPath": "true" },
        };
        if (!rememberTerminalEventId(seenEventsRef.current, pendingEventIds, syntheticEvent.id)) return false;
        queueEventWrite({ event: syntheticEvent, rawBytes });
        scheduleEventWriteDrain();
        applyOneKeyPromptState(reduceOneKeyPromptDetection(oneKeyPromptStateRef.current, syntheticEvent));
        return true;
      }
      const eventId = "terminal-bytes:" + bytesEvent.id;
      if (!rememberTerminalEventId(seenEventsRef.current, pendingEventIds, eventId)) return false;
      if (bytesEvent.truncated) fastPathTextDecoder = new TextDecoder();
      const event: SessionEvent = {
        id: eventId,
        sessionId: bytesEvent.sessionId,
        paneId: bytesEvent.sessionId + ":main",
        ts: bytesEvent.ts,
        direction: "inbound",
        stream: bytesEvent.stream,
        bytesRef: null,
        text: bytesEvent.truncated
          ? "PortMate: terminal byte frame was truncated; omitted bytes were not rendered.\r\n"
          : fastPathTextDecoder.decode(rawBytes, { stream: true }),
        annotations: { "terminalBytesFastPath": "true" },
      };
      queueEventWrite(bytesEvent.truncated ? { event } : { event, rawBytes });
      scheduleEventWriteDrain();
      return true;
    };
    writeTerminalBytesRef.current = writeTerminalBytes;
    const deferredBytes = deferredTerminalBytesRef.current.splice(0);
    for (const bytesEvent of deferredBytes) writeTerminalBytes(bytesEvent);
    const writeTerminalLive = (packet: TerminalLiveEvent) => {
      if (packet.event.sessionId !== active.profile.id) return false;
      const pendingText = pendingTextEvents.get(packet.event.id);
      if (pendingText) {
        if (pendingText.fallbackTimer !== undefined) window.clearTimeout(pendingText.fallbackTimer);
        pendingText.rawBytes = packet.truncated ? undefined : Uint8Array.from(packet.bytes);
        pendingText.event = {
          ...packet.event,
          annotations: { ...packet.event.annotations, terminalBytesCanonical: "true" },
        };
        enqueueFallback(pendingText, !packet.truncated);
        return true;
      }
      if (seenEventsRef.current.has(packet.event.id)) return false;
      if (!rememberTerminalEventId(seenEventsRef.current, pendingEventIds, packet.event.id)) return false;
      const event = packet.truncated
        ? {
          ...packet.event,
          text: packet.event.text || "PortMate: terminal byte frame was truncated; omitted bytes were not rendered.\r\n",
          annotations: { ...packet.event.annotations, terminalBytesCanonical: "true" },
        }
        : { ...packet.event, annotations: { ...packet.event.annotations, terminalBytesCanonical: "true" } };
      const rawBytes = packet.event.direction === "inbound" && !packet.truncated
        ? Uint8Array.from(packet.bytes)
        : undefined;
      queueEventWrite(rawBytes ? { event, rawBytes } : { event });
      scheduleEventWriteDrain();
      if (packet.event.direction === "inbound") {
        applyOneKeyPromptState(reduceOneKeyPromptDetection(oneKeyPromptStateRef.current, event));
      }
      return true;
    };
    writeTerminalLiveRef.current = writeTerminalLive;
    const deferredLive = deferredTerminalLiveRef.current.splice(0);
    for (const packet of deferredLive) writeTerminalLive(packet);
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
      void loadTerminalWebglAddon().then(({ WebglAddon }) => {
        if (terminalDisposed || generation !== webglGeneration || termRef.current !== term) return;
        // Chromium automation can discard the WebGL back buffer before Playwright
        // captures it, which makes rendered cursor checks observe an empty canvas.
        const nextAddon = new WebglAddon(navigator.webdriver);
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
      if (resizeReportTimer !== null) {
        window.clearTimeout(resizeReportTimer);
        resizeReportTimer = null;
      }
      // Only the active pane may resize the shared PTY. ResizeObserver callbacks
      // can arrive after focus changes, before the inactive pane has been laid out.
      if (
        displayModeRef.current === "hex"
        || !focusedRef.current
        || host.dataset.terminalResizeOwner !== "active"
        || !host.closest(".terminal-pane.active")
        || lastSizeRef.current === size
      ) return;
      let candidateSize = size;
      let candidateViewport = `${window.innerWidth}x${window.innerHeight}`;
      const reportStableSize = () => {
        resizeReportTimer = null;
        if (terminalDisposed || !focusedRef.current || host.dataset.terminalResizeOwner !== "active"
          || !host.closest(".terminal-pane.active")) return;
        fit.fit();
        scheduleTimestampGutter();
        const settledSize = `${term.cols}x${term.rows}`;
        host.dataset.terminalSize = settledSize;
        const settledViewport = `${window.innerWidth}x${window.innerHeight}`;
        if (settledSize !== candidateSize || settledViewport !== candidateViewport) {
          candidateSize = settledSize;
          candidateViewport = settledViewport;
          resizeReportTimer = window.setTimeout(reportStableSize, TERMINAL_RESIZE_SETTLE_MS);
          return;
        }
        if (displayModeRef.current === "hex" || lastSizeRef.current === settledSize) return;
        lastSizeRef.current = settledSize;
        if (isBackendAvailable()) {
          void invokeBackend("resize_session", {
            sessionId: active.profile.id,
            cols: term.cols,
            rows: term.rows,
          }).catch(() => {});
        }
      };
      resizeReportTimer = window.setTimeout(reportStableSize, TERMINAL_RESIZE_SETTLE_MS);
    };
    fitAndReportRef.current = fitAndReport;
    queueMicrotask(fitAndReport);

    const resizeObserver = new ResizeObserver(fitAndReport);
    resizeObserver.observe(host);
    let windowResizeFrame: number | null = null;
    const handleWindowResize = () => {
      if (resizeReportTimer !== null) {
        window.clearTimeout(resizeReportTimer);
        resizeReportTimer = null;
      }
      if (windowResizeFrame !== null) window.cancelAnimationFrame(windowResizeFrame);
      windowResizeFrame = window.requestAnimationFrame(() => {
        windowResizeFrame = null;
        fitAndReport();
      });
    };
    window.addEventListener("resize", handleWindowResize);
    let semanticFrame: number | null = null;
    let semanticTimer: number | null = null;
    let completionAnchorFrame: number | null = null;
    let completionAnchorRow = "";
    let lastInteractiveInputAt = 0;
    let semanticLines: TerminalSemanticDecorationLine[] = [];
    let semanticPresentationFingerprint = "";
    const disposeSemanticLine = (line: TerminalSemanticDecorationLine) => {
      for (const decoration of line.decorations) decoration.dispose();
      for (const marker of line.markers) marker.dispose();
    };
    const clearSemanticHighlighting = () => {
      for (const line of semanticLines.splice(0)) disposeSemanticLine(line);
      host.dataset.terminalSemanticDecorationCount = "0";
    };
    const renderSemanticHighlighting = () => {
      semanticFrame = null;
      const presentationFingerprint = terminalSemanticPresentationFingerprint(
        term,
        semanticThemeRef.current,
        semanticHighlightingEnabledRef.current,
        semanticHighlightingSupportedRef.current,
      );
      if (presentationFingerprint !== semanticPresentationFingerprint) {
        clearSemanticHighlighting();
        semanticPresentationFingerprint = presentationFingerprint;
      }
      if (!semanticHighlightingEnabledRef.current) {
        host.dataset.terminalSemanticHighlighting = "disabled";
        return;
      }
      if (!semanticHighlightingSupportedRef.current) {
        host.dataset.terminalSemanticHighlighting = "unsupported";
        return;
      }
      const buffer = term.buffer.active;
      if (buffer.type !== "normal") {
        host.dataset.terminalSemanticHighlighting = "alternate";
        return;
      }

      const cursorRow = buffer.baseY + buffer.cursorY;
      const previousByRow = new Map<number, TerminalSemanticDecorationLine>();
      for (const line of semanticLines) {
        if (!line.marker.isDisposed && line.marker.line >= 0) previousByRow.set(line.marker.line, line);
      }
      const nextLines: TerminalSemanticDecorationLine[] = [];
      const retainedLines = new Set<TerminalSemanticDecorationLine>();
      let firstRow = buffer.viewportY;
      while (firstRow > 0 && buffer.getLine(firstRow)?.isWrapped) firstRow -= 1;
      const viewportEnd = Math.min(buffer.length, buffer.viewportY + term.rows);
      let row = firstRow;
      let decorationCount = 0;
      while (row < viewportEnd) {
        const lineStart = row;
        const logicalLine = readTerminalSemanticLogicalLine(term, row);
        row = logicalLine.nextRow;
        const lineFingerprint = terminalSemanticLineFingerprint(logicalLine);
        const previous = previousByRow.get(lineStart);
        if (previous?.fingerprint === lineFingerprint) {
          retainedLines.add(previous);
          nextLines.push(previous);
          decorationCount += previous.decorations.length;
          continue;
        }
        const tokens = terminalSemanticTokens(logicalLine.text);
        if (!tokens.length) continue;
        const startMarker = term.registerMarker(lineStart - cursorRow);
        if (!startMarker) continue;
        const markers = [startMarker];
        const decorations: Array<{ dispose: () => void }> = [];
        const markerByRow = new Map<number, IMarker>([[lineStart, startMarker]]);
        for (const token of tokens) {
          for (const segment of terminalSemanticCellSegments(logicalLine.cells, token.start, token.end)) {
            let marker = markerByRow.get(segment.row);
            if (!marker) {
              marker = term.registerMarker(segment.row - cursorRow);
              if (marker) {
                markerByRow.set(segment.row, marker);
                markers.push(marker);
              }
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
            decorations.push(decoration);
            decorationCount += 1;
          }
        }
        nextLines.push({
          marker: startMarker,
          fingerprint: lineFingerprint,
          decorations,
          markers,
        });
      }
      for (const previous of semanticLines) {
        if (!retainedLines.has(previous)) disposeSemanticLine(previous);
      }
      semanticLines = nextLines;
      host.dataset.terminalSemanticHighlighting = "active";
      host.dataset.terminalSemanticDecorationCount = String(decorationCount);
    };
    scheduleSemanticHighlighting = () => {
      if (terminalDisposed) return;
      const inputIdleRemaining = Math.max(
        0,
        TERMINAL_SEMANTIC_INPUT_IDLE_MS - (performance.now() - lastInteractiveInputAt),
      );
      if (inputIdleRemaining > 0) {
        if (semanticTimer !== null) window.clearTimeout(semanticTimer);
        semanticTimer = window.setTimeout(() => {
          semanticTimer = null;
          scheduleSemanticHighlighting();
        }, inputIdleRemaining);
        return;
      }
      if (semanticTimer !== null) {
        window.clearTimeout(semanticTimer);
        semanticTimer = null;
      }
      if (semanticFrame !== null) return;
      semanticFrame = window.requestAnimationFrame(renderSemanticHighlighting);
    };
    const settleSemanticHighlighting = () => {
      if (terminalDisposed) return;
      if (semanticTimer !== null) window.clearTimeout(semanticTimer);
      const inputIdleRemaining = Math.max(
        0,
        TERMINAL_SEMANTIC_INPUT_IDLE_MS - (performance.now() - lastInteractiveInputAt),
      );
      semanticTimer = window.setTimeout(() => {
        semanticTimer = null;
        scheduleSemanticHighlighting();
      }, Math.max(TERMINAL_SEMANTIC_SETTLE_MS, inputIdleRemaining));
    };
    const scheduleCompletionAnchorRefresh = (force = false) => {
      if (terminalDisposed || !completionSurfaceOpenRef.current) return;
      const buffer = term.buffer.active;
      const row = `${buffer.type}:${buffer.cursorY}:${term.rows}`;
      if (!force && completionAnchorRow === row) return;
      completionAnchorRow = row;
      if (completionAnchorFrame !== null) return;
      completionAnchorFrame = window.requestAnimationFrame(() => {
        completionAnchorFrame = null;
        refreshCompletionAnchorRef.current();
      });
    };
    refreshSemanticHighlightingRef.current = scheduleSemanticHighlighting;
    const semanticWriteDisposable = term.onWriteParsed(() => {
      settleSemanticHighlighting();
      scheduleCompletionAnchorRefresh();
      commitDetectedPrivateInput(detectSensitiveInput());
      // A prompt can arrive after the Enter key's frame (especially over a
      // serial line). Keep the active output visible while that response is
      // parsed instead of allowing xterm to restore the old viewport.
      if (outputFollowDeadline > performance.now()) followTerminalOutput();
    });
    const semanticScrollDisposable = term.onScroll(() => {
      recordTerminalViewport();
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
      scheduleCompletionAnchorRefresh(true);
    });
    scheduleSemanticHighlighting();
    scheduleTimestampGutter();
    const guardTerminalEnter = (event: KeyboardEvent) => {
      if (!focusedRef.current
        || keyModeRef.current !== "remote"
        || event.key !== "Enter"
        || event.isComposing) return;
      // XTerm still receives the event and emits CR; this only removes the
      // browser's fallback textarea/page scroll behavior.
      event.preventDefault();
      keepTerminalAtOutput();
    };
    const releaseFollowOnWheel = () => releaseOutputFollow();
    host.addEventListener("keydown", guardTerminalEnter, true);
    host.addEventListener("wheel", releaseFollowOnWheel, true);
    const dispatchBinaryInput = (text: string) => {
      if (!focusedRef.current || keyModeRef.current !== "remote" || !text) return;
      const mouseReport = isTerminalMouseReport(text);
      if (mouseReport
        && (!mouseReportingRef.current || host.querySelector(".xterm-cursor-pointer"))) return;
      // XTerm's onBinary callback is a Latin-1 byte container. Keep the
      // payload atomic and source-local so default (non-SGR) mouse reports
      // cannot be lost or broadcast to another synchronized pane.
      if (!terminalBinaryStringToBytes(text)) return;
      lastInteractiveInputAt = performance.now();
      void onInputRef.current(active.profile.id, text, "atomic", { binary: true });
      if (mouseReport) dismissOneKeyPrompt();
      else {
        resetCompletionInput(false);
        dismissOneKeyPrompt();
      }
    };
    const inputDisposable = term.onData((text) => {
      if (!focusedRef.current || keyModeRef.current !== "remote") return;
      const mouseReport = isTerminalMouseReport(text);
      if (mouseReport) {
        dispatchBinaryInput(text);
        return;
      }
      lastInteractiveInputAt = performance.now();
      const detectedSensitive = detectSensitiveInput();
      commitDetectedPrivateInput(detectedSensitive);
      const sensitive = manualPrivateInputRef.current || detectedSensitive;
      const isEnter = /\r|\n/.test(text);
      const inputOrigin: SyncInputOrigin = /[\u0000-\u001f\u007f]/.test(text)
        ? "atomic"
        : "interactive";
      // The App-level registry is the single ordering queue for this session.
      // Bypassing the view-local pump removes a second IPC batching window.
      void onInputRef.current(
        active.profile.id,
        text,
        inputOrigin,
        sensitive ? { sensitive: true } : undefined,
      );
      const submittedCommands = updateCompletionInput(text);
      if (term.buffer.active.type === "normal"
        && submittedCommands.length
        && !sensitive) {
        for (const command of submittedCommands) {
          onCommandSubmitRef.current?.(active.profile.id, command);
        }
      }
      if (sensitive && /\r|\n|\u0003|\u0004/.test(text)) clearPrivateInput();
      dismissOneKeyPrompt();
      if (isEnter) keepTerminalAtOutput();
    });
    const binaryInputDisposable = term.onBinary(dispatchBinaryInput);
    const selectionDisposable = term.onSelectionChange(() => {
      host.dataset.terminalHasSelection = term.hasSelection() ? "true" : "false";
      if (!focusedRef.current) return;
      if (keyModeRef.current !== "remote") return;
      if (!copyOnSelectRef.current) return;
      const selected = term.getSelection();
      if (!selected || selected === lastCopiedSelectionRef.current) return;
      lastCopiedSelectionRef.current = selected;
      void navigator.clipboard?.writeText(selected).catch(() => {});
    });
    const pasteFromClipboard = (event: MouseEvent) => {
      event.preventDefault();
      if (!focusedRef.current || keyModeRef.current !== "remote") return;
      void navigator.clipboard?.readText().then((text) => {
        if (text) {
          resetCompletionInput(false);
          dismissOneKeyPrompt();
          const sensitive = manualPrivateInputRef.current || terminalInputLooksSensitive(
            term,
            oneKeyPromptStateRef.current.prompt,
          );
          void onInputRef.current(
            active.profile.id,
            text,
            "atomic",
            sensitive ? { sensitive: true } : undefined,
          );
          if (sensitive && /\r|\n|\u0003|\u0004/.test(text)) clearPrivateInput();
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
      writeTerminalLiveRef.current = null;
      window.cancelAnimationFrame(readyFrame);
      window.removeEventListener("resize", handleWindowResize);
      if (windowResizeFrame !== null) window.cancelAnimationFrame(windowResizeFrame);
      if (host.dataset.terminalInstanceId === terminalInstanceId) host.dataset.terminalReady = "false";
      searchResultDisposable.dispose();
      inputDisposable.dispose();
      binaryInputDisposable.dispose();
      selectionDisposable.dispose();
      bufferChangeDisposable.dispose();
      host.removeEventListener("paste", pauseCompletionOnPaste, true);
      host.removeEventListener("mousedown", forceBlockSelection, true);
      host.removeEventListener("auxclick", pasteOnMiddleClick);
      host.removeEventListener("keydown", guardTerminalEnter, true);
      host.removeEventListener("wheel", releaseFollowOnWheel, true);
      cancelScheduledCompletionInput();
      if (resizeReportTimer !== null) {
        window.clearTimeout(resizeReportTimer);
        resizeReportTimer = null;
      }
      pendingEventWrites.length = 0;
      resizeObserver.disconnect();
      semanticWriteDisposable.dispose();
      semanticScrollDisposable.dispose();
      semanticResizeDisposable.dispose();
      if (semanticFrame !== null) window.cancelAnimationFrame(semanticFrame);
      if (semanticTimer !== null) window.clearTimeout(semanticTimer);
      if (completionAnchorFrame !== null) window.cancelAnimationFrame(completionAnchorFrame);
      if (timestampFrame !== null) window.cancelAnimationFrame(timestampFrame);
      if (enterScrollFrame !== null) window.cancelAnimationFrame(enterScrollFrame);
      outputFollowDeadline = 0;
      clearSemanticHighlighting();
      if (refreshSemanticHighlightingRef.current === scheduleSemanticHighlighting) {
        refreshSemanticHighlightingRef.current = () => {};
      }
      if (refreshTimestampGutterRef.current === scheduleTimestampGutter) {
        refreshTimestampGutterRef.current = () => {};
      }
      if (exportTimestampSnapshotRef.current === exportTimestampSnapshot) {
        exportTimestampSnapshotRef.current = () => [];
      }
      if (fitAndReportRef.current === fitAndReport) fitAndReportRef.current = () => {};
      if (writeEventRef.current === writeEvent) writeEventRef.current = () => false;
      if (writeTerminalBytesRef.current === writeTerminalBytes) writeTerminalBytesRef.current = null;
      if (writeTerminalLiveRef.current === writeTerminalLive) writeTerminalLiveRef.current = null;
      deferredTerminalBytesRef.current = [];
      deferredTerminalLiveRef.current = [];
      for (const pending of pendingTextEvents.values()) {
        if (pending.fallbackTimer !== undefined) window.clearTimeout(pending.fallbackTimer);
      }
      pendingTextEvents.clear();
      fastPathTextDecoder.decode();
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
          terminalStateCache.save(stateCacheKey, {
            serialized: serialize.serialize({
              scrollback: serializedScrollback,
            }),
            cols: term.cols,
            rows: term.rows,
            seenEventIds: terminalEventSnapshotIds(
              seenEventsRef.current,
              polledEventIdsRef.current.get(active.profile.id) ?? [],
            ),
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
  }, [active?.profile.id, stateCacheKey, viewId]);

  useEffect(() => {
    setTimestampViewport(emptyTerminalTimestampViewport);
  }, [stateCacheKey]);

  useEffect(() => {
    refreshSemanticHighlightingRef.current();
  }, [
    completionSettings,
    semanticHighlightingSupported,
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
    host.dataset.terminalCursorColor = term.options.theme?.cursor ?? "#5eead4";
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
      if (active && focused && !modalBlocksTerminalCommand(hostRef.current)) openSearchRef.current();
    };
    window.addEventListener(TERMINAL_SEARCH_REQUEST_EVENT, requestSearch);
    return () => window.removeEventListener(TERMINAL_SEARCH_REQUEST_EVENT, requestSearch);
  }, [active?.profile.id, focused]);

  useEffect(() => {
    const clearSelectionBehindModal = (event: Event) => {
      const layer = (event as CustomEvent<ModalLayerActivatedDetail>).detail?.layer;
      const host = hostRef.current;
      if (!layer || !host || layer.contains(host)) return;
      termRef.current?.clearSelection();
      localNavigationRef.current = null;
      clearDocumentSelectionWithin(host.closest(".terminal-canvas"));
    };
    window.addEventListener(MODAL_LAYER_ACTIVATED_EVENT, clearSelectionBehindModal);
    return () => window.removeEventListener(MODAL_LAYER_ACTIVATED_EVENT, clearSelectionBehindModal);
  }, []);

  useEffect(() => {
    const requestGotoLine = () => {
      if (active && focused && !modalBlocksTerminalCommand(hostRef.current)) openGotoLineRef.current();
    };
    window.addEventListener(TERMINAL_GOTO_LINE_REQUEST_EVENT, requestGotoLine);
    return () => window.removeEventListener(TERMINAL_GOTO_LINE_REQUEST_EVENT, requestGotoLine);
  }, [active?.profile.id, focused]);

  useEffect(() => {
    const requestFreeInput = (event: Event) => {
      if (active && focused && !modalBlocksTerminalCommand(hostRef.current)) {
        const value = (event as CustomEvent<{ value?: unknown }>).detail?.value;
        openFreeInputRef.current(typeof value === "string" ? value : "");
      }
    };
    window.addEventListener(TERMINAL_FREE_INPUT_REQUEST_EVENT, requestFreeInput);
    return () => window.removeEventListener(TERMINAL_FREE_INPUT_REQUEST_EVENT, requestFreeInput);
  }, [active?.profile.id, focused]);

  useEffect(() => {
    const requestExport = (event: Event) => {
      const detail = (event as CustomEvent<TerminalTextExportRequestDetail>).detail;
      if (!detail || typeof detail.respond !== "function" || !active || !focused || !viewId
        || detail.sessionId !== active.profile.id || detail.viewId !== viewId) return;
      if (modalBlocksTerminalCommand(hostRef.current)) {
        detail.respond({ ok: false, error: "顶层对话框打开时不能导出终端文本。" });
        return;
      }
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
        : extractTerminalBufferText(term.buffer.active, exportTimestampSnapshotRef.current());
      if (!extracted.ok) {
        detail.respond({
          ok: false,
          error: extracted.reason === "empty"
            ? source === "selection" ? "当前终端没有选中文本。" : "当前终端缓冲为空。"
            : extracted.reason === "missing-timestamp"
              ? "终端时间戳尚未就绪，请稍后重试。"
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
          lineCount: extracted.lineCount,
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
      if (modalBlocksTerminalCommand(hostRef.current)) {
        detail.respond({ ok: false, error: "顶层对话框打开时不能修改终端缓冲。" });
        return;
      }
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
        if (focusedRef.current) term.focus();
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
      if (modalBlocksTerminalCommand(hostRef.current)) {
        detail.respond({ ok: false, error: "顶层对话框打开时不能访问终端选区。" });
        return;
      }
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
      if (focusedRef.current) term.focus();
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
    clearPrivateInput();
    setFreeInputSource(null);
    setFreeInputValue("");
    setGotoLineContext(null);
    setGotoLineQuery("");
  }, [active?.profile.id, viewId]);

  useEffect(() => () => {
    if (privateInputTimerRef.current !== null) {
      window.clearTimeout(privateInputTimerRef.current);
    }
  }, []);

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
        scheduleTerminalSurfaceFocus();
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
      scheduleTerminalSurfaceFocus();
      return;
    }

    setFreeInputSource(null);
    setFreeInputValue("");
    scheduleTerminalSurfaceFocus();
  }, [active?.profile.id, keyMode, mouseReporting, viewId]);

  useLayoutEffect(() => {
    if (!completionSurfaceOpen) {
      if (completionReadyKey !== "") setCompletionReadyKey("");
      return;
    }
    setCompletionReadyKey("");
    refreshCompletionAnchorRef.current();
    setCompletionReadyKey(completionGeometryKey);
  }, [completionGeometryKey, completionSurfaceOpen]);

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
      scheduleTerminalSurfaceFocus();
      fitAndReportRef.current();
    } else if (!focused) {
      termRef.current?.blur();
      termRef.current?.clearSelection();
      localNavigationRef.current = null;
      clearDocumentSelectionWithin(host?.closest(".terminal-canvas") ?? null);
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
      if (!term) return;
      writeEventRef.current(event.payload, true);
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
    if (!active || !isBackendAvailable()) return;
    return subscribeTerminalLiveEvents(active.profile.id, (packet) => {
      const writeTerminalLive = writeTerminalLiveRef.current;
      if (writeTerminalLive) writeTerminalLive(packet);
      else {
        deferredTerminalLiveRef.current.push(packet);
        if (deferredTerminalLiveRef.current.length > 256) {
          deferredTerminalLiveRef.current.splice(0, deferredTerminalLiveRef.current.length - 256);
        }
      }
    });
  }, [active?.profile.id]);

  useEffect(() => {
    if (!active || !isBackendAvailable()) return;
    return subscribeTerminalByteEvents(active.profile.id, (event) => {
      const writeTerminalBytes = writeTerminalBytesRef.current;
      if (writeTerminalBytes) {
        writeTerminalBytes(event);
      } else {
        deferredTerminalBytesRef.current.push(event);
        if (deferredTerminalBytesRef.current.length > 256) {
          deferredTerminalBytesRef.current.splice(0, deferredTerminalBytesRef.current.length - 256);
        }
      }
    });
  }, [active?.profile.id]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    const sessionId = active?.profile.id;
    const previousIds = sessionId ? polledEventIdsRef.current.get(sessionId) ?? [] : [];
    let firstNewIndex = 0;
    while (firstNewIndex < previousIds.length
      && firstNewIndex < events.length
      && previousIds[firstNewIndex] === events[firstNewIndex]?.id) {
      firstNewIndex += 1;
    }
    for (let index = firstNewIndex; index < events.length; index += 1) {
      writeEventRef.current(events[index]);
    }
    if (sessionId) {
      polledEventIdsRef.current.set(sessionId, events.map((event) => event.id));
    }
  }, [events, active?.profile.id]);

  return (
    <div
      className={`terminal-canvas${active ? " has-terminal-view" : ""}${completionSurfaceVisible ? " completion-open" : ""}`}
      data-terminal-focused={focused ? "true" : "false"}
      data-terminal-session-id={sessionId || undefined}
      data-terminal-view-id={viewId || undefined}
      inert={!focused}
      data-terminal-display-mode={active ? displayMode : undefined}
      data-terminal-private-input={privateInputActive ? "true" : "false"}
      data-completion-placement={completionSurfaceVisible ? "below" : undefined}
      data-completion-cursor-bottom={completionSurfaceVisible ? completionAnchor.cursorBottom : undefined}
      data-completion-shift={completionSurfaceVisible ? completionAnchor.shift : undefined}
      style={{
        "--terminal-background": canvasBackground ?? "#0d1117",
        "--terminal-completion-height": `${completionPanelHeight}px`,
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
            <button
              type="button"
              className={`terminal-private-input${privateInputActive ? " active" : ""}`}
              aria-label={detectedPrivateInput
                ? "已自动开启私密输入"
                : privateInputActive ? "关闭私密输入" : "开启私密输入"}
              aria-pressed={privateInputActive}
              disabled={detectedPrivateInput}
              title={detectedPrivateInput
                ? "检测到凭据提示，已自动保护：仅发送到当前会话且不写入日志"
                : privateInputActive
                  ? "私密输入已开启：仅发送到当前会话且不写入日志"
                : "私密输入：仅发送到当前会话且不写入日志"}
              onClick={toggleManualPrivateInput}
            >
              <Lock size={13} />
              <span>{privateInputActive ? "私密" : "私密输入"}</span>
            </button>
            <TerminalByteToolbar
              sessionId={active.profile.id}
              follow={byteFollow}
              onFollowChange={setByteFollow}
              onClear={() => { setByteSelection(null); setByteFollow(true); }}
            />
          </div>
          <div className={`terminal-workspace mode-${displayMode}`}>
            <div className={`terminal-terminal-region${focused && freeInputOpen ? " free-input-open" : ""}`} aria-hidden={displayMode === "hex"} inert={displayMode === "hex"}>
              <div
                className="terminal-timestamp-gutter"
                role="list"
                aria-label="终端行时间戳"
                data-buffer-type={timestampViewport.bufferType}
                data-timestamp-count={timestampViewport.entries.length}
                style={{
                  "--terminal-timestamp-cell-height": `${timestampViewport.cellHeight}px`,
                  transform: completionShiftTransform,
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
              <div
                ref={hostRef}
                className="terminal-host"
                inert={displayMode === "hex" || freeInputOpen || gotoLineOpen}
                style={{ transform: completionShiftTransform }}
              />
          {focused && freeInputOpen ? (
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
          {completionSurfaceVisible ? (
            <div className="terminal-completion-layer">
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
            </div>
          ) : null}
          {focused && gotoLineContext ? (
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
          {focused && searchOpen ? (
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
              <TerminalByteInspectorPane
                sessionId={active.profile.id}
                bytesPerRow={displayMode === "split" ? 8 : 16}
                follow={byteFollow}
                selection={byteSelection}
                onFollowChange={setByteFollow}
                onSelectionChange={setByteSelection}
              />
            ) : null}
          </div>
        </>
      ) : (
        <div className="terminal-empty">未打开会话</div>
      )}
    </div>
  );
}

function useTerminalByteSnapshot(sessionId: string) {
  const subscribe = useCallback(
    (listener: () => void) => subscribeTerminalByteCache(sessionId, listener),
    [sessionId],
  );
  const getSnapshot = useCallback(() => terminalByteCacheSnapshot(sessionId), [sessionId]);
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

function TerminalByteToolbar({
  sessionId,
  follow,
  onFollowChange,
  onClear,
}: {
  sessionId: string;
  follow: boolean;
  onFollowChange: (follow: boolean) => void;
  onClear: () => void;
}) {
  const snapshot = useTerminalByteSnapshot(sessionId);
  const stats = useMemo(() => terminalByteBufferStats(snapshot), [snapshot]);
  return (
    <>
      <span className="terminal-byte-summary" title={`实时窗口 ${formatBytes(snapshot.capturedBytes)} · ${snapshot.frames.length} 帧${snapshot.droppedFrames ? ` · 已淘汰 ${snapshot.droppedFrames} 帧` : ""}${stats.omittedBytes ? ` · 帧内截断 ${formatBytes(stats.omittedBytes)}` : ""}`}>
        <span className="rx">RX {formatBytes(stats.rxBytes)}</span>
        <span className="tx">TX {formatBytes(stats.txBytes)}</span>
      </span>
      <button type="button" className={follow ? "terminal-byte-tool active" : "terminal-byte-tool"} aria-label="跟随最新字节" aria-pressed={follow} title="跟随最新字节" disabled={!snapshot.frames.length} onClick={() => onFollowChange(!follow)}><ArrowDownToLine size={13} /></button>
      <button type="button" className="terminal-byte-tool" aria-label="清空实时字节" title="清空实时字节" disabled={!snapshot.frames.length} onClick={() => { flushTerminalByteEvents(); clearTerminalByteCache(sessionId); onClear(); }}><Trash2 size={13} /></button>
    </>
  );
}

function TerminalByteInspectorPane({
  sessionId,
  bytesPerRow,
  follow,
  selection,
  onFollowChange,
  onSelectionChange,
}: {
  sessionId: string;
  bytesPerRow: number;
  follow: boolean;
  selection: TerminalByteSelection | null;
  onFollowChange: (follow: boolean) => void;
  onSelectionChange: (selection: TerminalByteSelection | null) => void;
}) {
  const snapshot = useTerminalByteSnapshot(sessionId);
  return (
    <Suspense fallback={<section className="terminal-byte-inspector" aria-label="终端字节检查器" aria-busy="true" />}>
      <LazyTerminalByteInspector
        snapshot={snapshot}
        bytesPerRow={bytesPerRow}
        follow={follow}
        selection={selection}
        onFollowChange={onFollowChange}
        onSelectionChange={onSelectionChange}
      />
    </Suspense>
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

function writeTerminalEvent(
  term: XTerm,
  event: SessionEvent,
  rawBytes: Uint8Array | undefined,
  onParsed: () => void,
) {
  if (rawBytes) {
    term.write(rawBytes, onParsed);
    return;
  }
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

function terminalInputLooksSensitive(
  term: XTerm,
  prompt: OneKeyTerminalPrompt | null,
): boolean {
  if (prompt) return true;
  const buffer = term.buffer.active;
  const row = buffer.type === "normal" ? buffer.baseY + buffer.cursorY : buffer.cursorY;
  const line = buffer.getLine(row)?.translateToString(true) ?? "";
  return terminalInputLineLooksSensitive(line);
}

function terminalInputLineLooksSensitive(line: string): boolean {
  return TERMINAL_SENSITIVE_INPUT_PATTERN.test(line);
}

function concatTerminalWriteBytes(frames: readonly Uint8Array[]): Uint8Array {
  if (frames.length === 1) return frames[0];
  const length = frames.reduce((total, frame) => total + frame.byteLength, 0);
  const merged = new Uint8Array(length);
  let offset = 0;
  for (const frame of frames) {
    merged.set(frame, offset);
    offset += frame.byteLength;
  }
  return merged;
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

function modalBlocksTerminalCommand(host: HTMLElement | null): boolean {
  const layer = activeModalLayer();
  return Boolean(layer && (!host || !layer.contains(host)));
}

function formatTerminalTimestampTitle(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : `${date.toLocaleString()} · ${value}`;
}

function clearDocumentSelectionWithin(container: Element | null) {
  if (!container) return;
  const selection = document.getSelection();
  if (!selection?.rangeCount) return;
  if ((selection.anchorNode && container.contains(selection.anchorNode))
    || (selection.focusNode && container.contains(selection.focusNode))) {
    selection.removeAllRanges();
  }
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

export default memo(TerminalCanvas);
