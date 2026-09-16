import { t, useLocale } from "./i18n";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDownToLine,
  Bookmark,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  Copy,
  Download,
  RefreshCw,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { callBackend, invokeBackend, isBackendAvailable } from "./api";
import ChildWindowScreenLockOverlay from "./ChildWindowScreenLockOverlay";
import { KeyedRequestGate } from "./keyed-request-gate";
import { decodeStoredScreenLockMarker, SCREEN_LOCK_STORAGE_KEY } from "./screen-lock-state";
import type { ScreenLockMarker } from "./screen-lock-state";
import {
  analyzeSerialCaptureFrames,
  defaultSerialAnalyzerStoredState,
  filterSerialAnalyzedFrames,
  normalizeSerialAnalyzerStoredState,
  normalizeSerialFrameParserConfig,
  SERIAL_ANALYZER_STORAGE_KEY,
  serialAnalyzerDelimiterBytes,
  serialAnalyzerHasDistinctWire,
  serialAnalyzerHexDump,
  serialModbusSilenceMs,
  toggleSerialAnalyzerBookmark,
} from "./serial-analyzer-state";
import type {
  SerialAnalyzedFrame,
  SerialAnalyzerStoredState,
  SerialFrameParserConfig,
  SerialFrameParserMode,
} from "./serial-analyzer-state";
import type { SerialAnalyzerRequest } from "./serial-analyzer-route";
import { mergeSerialCaptureSnapshot, serialCaptureAscii, serialCaptureHex } from "./serial-capture-state";
import { readSessionSummaryCache } from "./session-summary-cache";
import {
  sessionRuntimeDisconnectDescription,
  sessionRuntimeHealthDescription,
  sessionRuntimeStatusLabel,
} from "./session-runtime-state";
import type {
  ExportSerialCaptureResult,
  SerialCaptureFrame,
  SerialCaptureHistorySnapshot,
  SerialCaptureSnapshot,
  SessionSummary,
} from "./types";

type SerialCaptureSource = "live" | "history";
type SerialCaptureOperation = "refresh" | "clear" | "export";

const parserModes: Array<{ value: SerialFrameParserMode; label: string }> = [
  { value: "capture", label: "capture" },
  { value: "delimiter", label: "delimiter" },
  { value: "fixed", label: "fixed-length" },
  { value: "gap", label: "interval" },
  { value: "slip", label: "SLIP" },
  { value: "cobs", label: "COBS" },
  { value: "modbus", label: "Modbus" },
];

export default function SerialAnalyzerApp({ request }: { request: SerialAnalyzerRequest }) {
  useLocale();
  const [sessions, setSessions] = useState<SessionSummary[]>(loadLocalSessions);
  const [frames, setFrames] = useState<SerialCaptureFrame[]>([]);
  const [source, setSource] = useState<SerialCaptureSource>("live");
  const [history, setHistory] = useState<SerialCaptureHistorySnapshot | null>(null);
  const [captureOperation, setCaptureOperation] = useState<SerialCaptureOperation | null>(null);
  const [message, setMessage] = useState("");
  const [screenLock, setScreenLock] = useState<ScreenLockMarker | null>(readScreenLockMarker);
  const sessionsRef = useRef<SessionSummary[]>(sessions);
  const sessionRefreshGateRef = useRef(new KeyedRequestGate<"sessions">());
  const framesRef = useRef<SerialCaptureFrame[]>([]);
  const captureOperationGateRef = useRef(new KeyedRequestGate<"capture">());
  const captureEpochRef = useRef(0);
  const sessionSignatureRef = useRef("");
  const session = sessions.find((item) => item.profile.id === request.sessionId);
  const isSerial = session?.profile.connection.kind === "serial";
  const captureBusy = captureOperation !== null;
  const refreshing = captureOperation === "refresh";

  useEffect(() => {
    document.title = t("portmate-serial-analyzer", [session?.profile.name ?? t("serial")]);
  }, [session?.profile.name]);

  useEffect(() => {
    const refreshLock = () => setScreenLock((current) => {
      const next = readScreenLockMarker(current?.lockedAt);
      return screenLockMarkersEqual(current, next) ? current : next;
    });
    const handleStorage = (event: StorageEvent) => {
      if (event.key === SCREEN_LOCK_STORAGE_KEY || event.key === null) refreshLock();
    };
    const timer = window.setInterval(refreshLock, 500);
    window.addEventListener("storage", handleStorage);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("storage", handleStorage);
    };
  }, []);

  useEffect(() => {
    void refreshSessions();
    const timer = window.setInterval(() => void refreshSessions(), 1500);
    return () => {
      window.clearInterval(timer);
      sessionRefreshGateRef.current.invalidate("sessions");
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const epoch = captureEpochRef.current + 1;
    captureEpochRef.current = epoch;
    if (source === "live") void refreshCapture();
    else void refreshHistory();
    const captureTimer = source === "live" ? window.setInterval(() => void refreshCapture(), 750) : null;
    return () => {
      disposed = true;
      if (captureTimer !== null) window.clearInterval(captureTimer);
      captureOperationGateRef.current.invalidate("capture");
    };

    async function refreshCapture(force = false) {
      if (!isBackendAvailable()) return;
      const gate = captureOperationGateRef.current;
      const token = gate.begin("capture");
      if (token === null) return;
      setCaptureOperation("refresh");
      try {
        const current = force ? [] : framesRef.current;
        const snapshot = await invokeBackend<SerialCaptureSnapshot>("list_serial_capture", {
          sessionId: request.sessionId,
          afterId: current.at(-1)?.id ?? null,
        });
        if (disposed || captureEpochRef.current !== epoch || !gate.isCurrent("capture", token)) return;
        storeFrames(mergeSerialCaptureSnapshot(current, snapshot));
        setHistory(null);
        setMessage("");
      } catch (error) {
        if (!disposed && captureEpochRef.current === epoch && gate.isCurrent("capture", token)) {
          setMessage(formatAnalyzerError(error));
        }
      } finally {
        if (gate.finish("capture", token) && !disposed && captureEpochRef.current === epoch) {
          setCaptureOperation(null);
        }
      }
    }

    async function refreshHistory() {
      if (!isBackendAvailable()) return;
      const gate = captureOperationGateRef.current;
      const token = gate.begin("capture");
      if (token === null) return;
      setCaptureOperation("refresh");
      try {
        const snapshot = await invokeBackend<SerialCaptureHistorySnapshot>("list_serial_capture_history", {
          sessionId: request.sessionId,
        });
        if (disposed || captureEpochRef.current !== epoch || !gate.isCurrent("capture", token)) return;
        storeFrames(snapshot.frames);
        setHistory(snapshot);
        setMessage(snapshot.enabled ? "" : t("raw-logging-disabled"));
      } catch (error) {
        if (!disposed && captureEpochRef.current === epoch && gate.isCurrent("capture", token)) {
          setMessage(formatAnalyzerError(error));
        }
      } finally {
        if (gate.finish("capture", token) && !disposed && captureEpochRef.current === epoch) {
          setCaptureOperation(null);
        }
      }
    }
  }, [request.sessionId, source]);

  function storeFrames(next: SerialCaptureFrame[]) {
    framesRef.current = next;
    setFrames(next);
  }

  function storeSessions(next: SessionSummary[]) {
    const signature = serialAnalyzerSessionSignature(next);
    if (sessionSignatureRef.current === signature) return;
    const snapshot = cloneSessionSummaries(next);
    sessionSignatureRef.current = signature;
    sessionsRef.current = snapshot;
    setSessions(snapshot);
  }

  async function refreshSessions() {
    const gate = sessionRefreshGateRef.current;
    const token = gate.begin("sessions");
    if (token === null) return;
    try {
      const fallback = isBackendAvailable() ? sessionsRef.current : loadLocalSessions();
      const next = await callBackend<SessionSummary[]>("list_sessions", {}, fallback);
      if (gate.isCurrent("sessions", token)) storeSessions(next);
    } finally {
      gate.finish("sessions", token);
    }
  }

  async function refreshNow() {
    const epoch = captureEpochRef.current;
    if (!isBackendAvailable()) return;
    const gate = captureOperationGateRef.current;
    const token = gate.begin("capture");
    if (token === null) return;
    setCaptureOperation("refresh");
    try {
      if (source === "live") {
        const snapshot = await invokeBackend<SerialCaptureSnapshot>("list_serial_capture", {
          sessionId: request.sessionId,
          afterId: null,
        });
        if (captureEpochRef.current !== epoch || !gate.isCurrent("capture", token)) return;
        storeFrames(mergeSerialCaptureSnapshot([], snapshot));
        setHistory(null);
        setMessage("");
      } else {
        const snapshot = await invokeBackend<SerialCaptureHistorySnapshot>("list_serial_capture_history", {
          sessionId: request.sessionId,
        });
        if (captureEpochRef.current !== epoch || !gate.isCurrent("capture", token)) return;
        storeFrames(snapshot.frames);
        setHistory(snapshot);
        setMessage(snapshot.enabled ? "" : t("raw-logging-disabled"));
      }
      await refreshSessions();
    } catch (error) {
      if (captureEpochRef.current === epoch && gate.isCurrent("capture", token)) {
        setMessage(formatAnalyzerError(error));
      }
    } finally {
      if (gate.finish("capture", token) && captureEpochRef.current === epoch) {
        setCaptureOperation(null);
      }
    }
  }

  async function clearCapture() {
    if (source !== "live") return;
    if (!frames.length) return;
    const gate = captureOperationGateRef.current;
    const token = gate.begin("capture");
    if (token === null) return;
    if (!window.confirm(t("clear-all-captured-frames-from-memory-for-the-current"))) {
      gate.finish("capture", token);
      return;
    }
    const epoch = captureEpochRef.current;
    setCaptureOperation("clear");
    try {
      if (isBackendAvailable()) {
        const snapshot = await invokeBackend<SerialCaptureSnapshot>("clear_serial_capture", { sessionId: request.sessionId });
        if (captureEpochRef.current !== epoch || !gate.isCurrent("capture", token)) return;
        storeFrames(mergeSerialCaptureSnapshot([], snapshot));
      } else {
        storeFrames([]);
      }
      setMessage(t("capture-cleared"));
    } catch (error) {
      if (captureEpochRef.current === epoch && gate.isCurrent("capture", token)) {
        setMessage(formatAnalyzerError(error));
      }
    } finally {
      if (gate.finish("capture", token) && captureEpochRef.current === epoch) {
        setCaptureOperation(null);
      }
    }
  }

  async function exportFrames(frameIds: string[]) {
    if (!frameIds.length) return;
    const gate = captureOperationGateRef.current;
    const token = gate.begin("capture");
    if (token === null) return;
    const epoch = captureEpochRef.current;
    const exportSource = source;
    setCaptureOperation("export");
    try {
      const command = exportSource === "live" ? "export_serial_capture" : "export_serial_capture_history";
      const result = await invokeBackend<ExportSerialCaptureResult>(command, {
        request: { sessionId: request.sessionId, frameIds },
      });
      if (captureEpochRef.current !== epoch || !gate.isCurrent("capture", token)) return;
      setMessage(t("frames", [result.frames, formatAnalyzerBytes(result.capturedBytes), result.path]));
    } catch (error) {
      if (captureEpochRef.current === epoch && gate.isCurrent("capture", token)) {
        setMessage(formatAnalyzerError(error));
      }
    } finally {
      if (gate.finish("capture", token) && captureEpochRef.current === epoch) {
        setCaptureOperation(null);
      }
    }
  }

  function changeSource(next: SerialCaptureSource) {
    if (next === source || captureBusy) return;
    captureEpochRef.current += 1;
    captureOperationGateRef.current.invalidate("capture");
    storeFrames([]);
    setHistory(null);
    setMessage("");
    setSource(next);
  }

  async function closeWindow() {
    try {
      if (isBackendAvailable()) await getCurrentWebviewWindow().close();
      else window.close();
    } catch {
      window.close();
    }
  }

  return (
    <main className="serial-analyzer-root" data-window-id={request.windowId} data-session-id={request.sessionId}>
      {session && isSerial ? (
        <SerialAnalyzerWorkspace
          session={session}
          frames={frames}
          source={source}
          history={history}
          captureBusy={captureBusy}
          refreshing={refreshing}
          message={message}
          canExport={isBackendAvailable()}
          onRefresh={() => void refreshNow()}
          onClear={() => void clearCapture()}
          onExport={(frameIds) => void exportFrames(frameIds)}
          onSourceChange={changeSource}
          onClose={() => void closeWindow()}
        />
      ) : (
        <section className="serial-analyzer-missing">
          <strong>{session ? t("the-session-is-not-serial") : t("serial-session-unavailable")}</strong>
          <span>{request.sessionId}</span>
          <button type="button" onClick={() => void closeWindow()}>{t("close-window")}</button>
        </section>
      )}
      {screenLock ? <ChildWindowScreenLockOverlay marker={screenLock} ownerWindowId={request.ownerWindowId} /> : null}
    </main>
  );
}

function SerialAnalyzerWorkspace({
  session,
  frames,
  source,
  history,
  captureBusy,
  refreshing,
  message,
  canExport,
  onRefresh,
  onClear,
  onExport,
  onSourceChange,
  onClose,
}: {
  session: SessionSummary;
  frames: SerialCaptureFrame[];
  source: SerialCaptureSource;
  history: SerialCaptureHistorySnapshot | null;
  captureBusy: boolean;
  refreshing: boolean;
  message: string;
  canExport: boolean;
  onRefresh: () => void;
  onClear: () => void;
  onExport: (frameIds: string[]) => void;
  onSourceChange: (source: SerialCaptureSource) => void;
  onClose: () => void;
}) {
  useLocale();
  const [stored, setStored] = useState<SerialAnalyzerStoredState>(loadStoredAnalyzerState);
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState("");
  const [page, setPage] = useState(0);
  const [inspectorView, setInspectorView] = useState<"decoded" | "wire">("decoded");
  const [delimiterDraft, setDelimiterDraft] = useState(stored.parser.delimiterHex);
  const serial = session.profile.connection.kind === "serial" ? session.profile.connection : null;
  const analysis = useMemo(
    () => analyzeSerialCaptureFrames(frames, stored.parser, serial?.baudRate),
    [frames, serial?.baudRate, stored.parser],
  );
  const bookmarkIds = useMemo(() => new Set(stored.bookmarks[session.profile.id] ?? []), [session.profile.id, stored.bookmarks]);
  const filtered = useMemo(
    () => filterSerialAnalyzedFrames(analysis.frames, stored.direction, query, bookmarkIds, stored.bookmarksOnly),
    [analysis.frames, bookmarkIds, query, stored.bookmarksOnly, stored.direction],
  );
  const pageCount = Math.max(1, Math.ceil(filtered.length / stored.pageSize));
  const activePage = stored.follow ? pageCount - 1 : Math.min(page, pageCount - 1);
  const visible = filtered.slice(activePage * stored.pageSize, (activePage + 1) * stored.pageSize);
  const selected = (stored.follow ? filtered.at(-1) : filtered.find((frame) => frame.id === selectedId))
    ?? visible.at(-1)
    ?? null;
  const delimiterValid = Boolean(serialAnalyzerDelimiterBytes(delimiterDraft));
  const rxCount = analysis.frames.filter((frame) => frame.direction === "inbound").length;
  const txCount = analysis.frames.length - rxCount;
  const errorCount = analysis.frames.filter((frame) => frame.decodeError).length;
  const hasDistinctWire = selected ? serialAnalyzerHasDistinctWire(selected) : false;
  const inspectedBytes = selected && inspectorView === "wire" && hasDistinctWire ? selected.wireBytes : selected?.bytes ?? [];

  useEffect(() => {
    try {
      window.localStorage.setItem(SERIAL_ANALYZER_STORAGE_KEY, JSON.stringify(stored));
    } catch {
      // Analyzer remains usable when browser storage is unavailable.
    }
  }, [stored]);

  function updateParser(patch: Partial<SerialFrameParserConfig>) {
    setStored((current) => ({
      ...current,
      parser: normalizeSerialFrameParserConfig({ ...current.parser, ...patch }),
    }));
    setPage(0);
    setSelectedId("");
  }

  function commitDelimiter() {
    const bytes = serialAnalyzerDelimiterBytes(delimiterDraft);
    if (!bytes) return;
    updateParser({ delimiterHex: delimiterDraft });
    setDelimiterDraft(bytes.map((byte) => byte.toString(16).padStart(2, "0").toUpperCase()).join(" "));
  }

  function selectFrame(frame: SerialAnalyzedFrame) {
    setStored((current) => ({ ...current, follow: false }));
    setSelectedId(frame.id);
  }

  function moveSelection(offset: number) {
    if (!filtered.length) return;
    const currentIndex = selected ? filtered.findIndex((frame) => frame.id === selected.id) : filtered.length - 1;
    const nextIndex = Math.min(filtered.length - 1, Math.max(0, currentIndex + offset));
    const next = filtered[nextIndex];
    setStored((current) => ({ ...current, follow: false }));
    setSelectedId(next.id);
    setPage(Math.floor(nextIndex / stored.pageSize));
  }

  function toggleBookmark(frame: SerialAnalyzedFrame) {
    setStored((current) => toggleSerialAnalyzerBookmark(current, session.profile.id, frame.bookmarkId));
  }

  function exportVisible() {
    const ids = [...new Set(filtered.flatMap((frame) => frame.sourceFrameIds))];
    onExport(ids);
  }

  const connectionLabel = serial
    ? `${serial.port || t("no-port-selected")} · ${serial.baudRate} baud · ${serial.dataBits}${serial.parity.slice(0, 1).toUpperCase()}${serial.stopBits} · ${serial.flowControl}`
    : "Serial";
  const runtimeStatus = sessionRuntimeStatusLabel(session.runtime.status);
  const runtimeHealth = sessionRuntimeHealthDescription(session.runtime);
  const disconnectHealth = sessionRuntimeDisconnectDescription(session.runtime);

  return (
    <>
      <header className="serial-analyzer-titlebar">
        <span className="serial-analyzer-brand">PortMate</span>
        <strong>{t("serial-analyzer")}</strong>
        <span className="serial-analyzer-session" title={session.profile.name}>{session.profile.name}</span>
        <span
          className={`serial-analyzer-connection ${session.runtime.status}`}
          title={runtimeHealth}
          aria-description={runtimeHealth}
        >{runtimeStatus}</span>
        <button type="button" title={t("close-serial-analyzer")} aria-label={t("close-serial-analyzer")} onClick={onClose}><X size={17} /></button>
      </header>

      <section className="serial-analyzer-toolbar" aria-label={t("serial-analysis-settings")} aria-busy={captureBusy}>
        <div className="serial-analyzer-segmented" aria-label={t("frame-parser")}>
          {parserModes.map((mode) => (
            <button key={mode.value} type="button" aria-pressed={stored.parser.mode === mode.value} onClick={() => updateParser({ mode: mode.value })}>{t(mode.label)}</button>
          ))}
        </div>
        <div className="serial-analyzer-parser-option">
          {stored.parser.mode === "delimiter" ? (
            <>
              <label><span>Hex</span><input className={delimiterValid ? "" : "invalid"} aria-label={t("hex-frame-delimiter")} aria-invalid={!delimiterValid} value={delimiterDraft} onChange={(event) => setDelimiterDraft(event.target.value.slice(0, 128))} onBlur={commitDelimiter} onKeyDown={(event) => event.key === "Enter" && commitDelimiter()} /></label>
              <label className="serial-analyzer-check"><input type="checkbox" checked={stored.parser.includeDelimiter} onChange={(event) => updateParser({ includeDelimiter: event.target.checked })} /><span>{t("keep")}</span></label>
            </>
          ) : stored.parser.mode === "fixed" ? (
            <label><span>{t("bytes")}</span><input type="number" min={1} max={4096} value={stored.parser.fixedLength} onChange={(event) => updateParser({ fixedLength: Number(event.target.value) })} /></label>
          ) : stored.parser.mode === "gap" ? (
            <label><span>ms</span><input type="number" min={1} max={60000} value={stored.parser.gapMs} onChange={(event) => updateParser({ gapMs: Number(event.target.value) })} /></label>
          ) : stored.parser.mode === "modbus" ? (
            <>
              <label className="serial-analyzer-check"><input type="checkbox" checked={stored.parser.modbusAutoGap !== false} onChange={(event) => updateParser({ modbusAutoGap: event.target.checked })} /><span>{t("automatic")}</span></label>
              {stored.parser.modbusAutoGap !== false
                ? <span className="serial-analyzer-parser-value">{serialModbusSilenceMs(serial?.baudRate ?? 115_200)} ms</span>
                : <label><span>ms</span><input type="number" min={1} max={60000} value={stored.parser.modbusGapMs ?? 2} onChange={(event) => updateParser({ modbusGapMs: Number(event.target.value) })} /></label>}
            </>
          ) : <span className="serial-analyzer-parser-value">{
            stored.parser.mode === "slip" ? "RFC 1055" : stored.parser.mode === "cobs" ? t("0x00-frame-boundary") : t("read-chunks")
          }</span>}
        </div>
        <div className="serial-analyzer-segmented source" aria-label={t("capture-source")}>
          {(["live", "history"] as const).map((value) => (
            <button key={value} type="button" disabled={captureBusy} aria-pressed={source === value} onClick={() => onSourceChange(value)}>{value === "live" ? t("live") : t("logs")}</button>
          ))}
        </div>
        <div className="serial-analyzer-segmented direction" aria-label={t("frame-direction")}>
          {(["all", "inbound", "outbound"] as const).map((direction) => (
            <button key={direction} type="button" aria-pressed={stored.direction === direction} onClick={() => setStored((current) => ({ ...current, direction }))}>{direction === "all" ? t("all") : direction === "inbound" ? "RX" : "TX"}</button>
          ))}
        </div>
        <label className="serial-analyzer-search"><Search size={13} /><input aria-label={t("filter-analysis-frames")} placeholder="Hex / ASCII" value={query} onChange={(event) => { setQuery(event.target.value); setPage(0); }} /></label>
        <button type="button" className={stored.bookmarksOnly ? "active" : ""} aria-pressed={stored.bookmarksOnly} title={t("bookmarks-only")} aria-label={t("bookmarks-only")} onClick={() => setStored((current) => ({ ...current, bookmarksOnly: !current.bookmarksOnly }))}><Bookmark size={14} fill={stored.bookmarksOnly ? "currentColor" : "none"} /></button>
        <button type="button" className={stored.follow ? "active" : ""} aria-pressed={stored.follow} title={t("follow-latest-frame")} aria-label={t("follow-latest-frame")} onClick={() => setStored((current) => ({ ...current, follow: !current.follow }))}><ArrowDownToLine size={14} /></button>
        <button type="button" title={t("refresh-capture")} aria-label={t("refresh-serial-capture")} onClick={onRefresh} disabled={captureBusy}><RefreshCw size={14} className={refreshing ? "spin" : ""} /></button>
        <button type="button" title={t("export-filtered-frames")} aria-label={t("export-filtered-serial-frames")} disabled={captureBusy || !canExport || !filtered.length || (source === "history" && !history?.enabled)} onClick={exportVisible}><Download size={14} /></button>
        <button type="button" title={source === "live" ? t("clear-capture") : t("historical-logs-can-only-be-deleted-in-the-log")} aria-label={t("clear-serial-capture")} disabled={captureBusy || source !== "live" || !frames.length} onClick={onClear}><Trash2 size={14} /></button>
      </section>

      <section className="serial-analyzer-status-strip">
        <span title={connectionLabel}>{connectionLabel}</span>
        <span>{source === "live" ? t("live") : t("logs")}</span>
        <span>{t("captured", [frames.length])}</span>
        <span>{t("parsed", [analysis.totalFrames])}</span>
        <span>RX {rxCount}</span>
        <span>TX {txCount}</span>
        <span>{formatAnalyzerBytes(analysis.capturedBytes)}</span>
        {errorCount ? <span className="error">{t("errors", [errorCount])}</span> : null}
        {analysis.droppedFrames ? <span className="warning">{t("outside-window", [analysis.droppedFrames])}</span> : null}
        {history?.droppedFrames ? <span className="warning">{t("outside-logs", [history.droppedFrames])}</span> : null}
        {history?.unavailableFrames ? <span className="error">{t("unavailable", [history.unavailableFrames])}</span> : null}
        {disconnectHealth
          ? <span className="serial-analyzer-last-disconnect" title={disconnectHealth}>{disconnectHealth}</span>
          : null}
      </section>

      <section
        className="serial-analyzer-table"
        role="grid"
        aria-label={t("serial-analysis-frames")}
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.key === "ArrowUp" || event.key === "ArrowDown") {
            event.preventDefault();
            moveSelection(event.key === "ArrowUp" ? -1 : 1);
          } else if (event.key === "Home" || event.key === "End") {
            event.preventDefault();
            moveSelection(event.key === "Home" ? -filtered.length : filtered.length);
          }
        }}
      >
        <div className="serial-analyzer-table-head" role="row">
          <span role="columnheader" aria-label={t("bookmark")} />
          <span role="columnheader">{t("time")}</span>
          <span role="columnheader">{t("direction")}</span>
          <span role="columnheader">{t("length")}</span>
          <span role="columnheader">{t("boundary")}</span>
          <span role="columnheader">Hex</span>
          <span role="columnheader">ASCII</span>
        </div>
        <div className="serial-analyzer-table-body">
          {!visible.length ? <div className="serial-analyzer-empty">{t("no-matching-analysis-frames")}</div> : null}
          {visible.map((frame) => {
            const bookmarked = bookmarkIds.has(frame.bookmarkId);
            return (
              <div
                key={frame.id}
                role="row"
                aria-selected={selected?.id === frame.id}
                className={`serial-analyzer-row ${frame.direction}${selected?.id === frame.id ? " selected" : ""}`}
                tabIndex={-1}
                onClick={() => selectFrame(frame)}
              >
                <button type="button" role="gridcell" title={bookmarked ? t("remove-bookmark") : t("add-bookmark")} aria-label={bookmarked ? t("remove-frame-bookmark") : t("add-frame-bookmark")} onClick={(event) => { event.stopPropagation(); toggleBookmark(frame); }}><Bookmark size={13} fill={bookmarked ? "currentColor" : "none"} /></button>
                <span role="gridcell" title={new Date(frame.ts).toLocaleString()}>{formatAnalyzerTime(frame.ts)}</span>
                <strong role="gridcell">{frame.direction === "inbound" ? "RX" : "TX"}</strong>
                <span role="gridcell" title={serialAnalyzerLengthTitle(frame)}>{serialAnalyzerLengthLabel(frame)}</span>
                <span role="gridcell" className={frame.decodeError ? "error" : !frame.complete || frame.truncated ? "warning" : ""}>{serialAnalyzerFrameStatus(frame)}</span>
                <code role="gridcell">{serialCaptureHex(frame.bytes, 48) || "--"}</code>
                <span role="gridcell" className="ascii">{serialCaptureAscii(frame.bytes.slice(0, 128)) || "--"}</span>
              </div>
            );
          })}
        </div>
      </section>

      <section className="serial-analyzer-inspector">
        {selected ? (
          <>
            <header>
              <strong>{selected.direction === "inbound" ? "RX" : "TX"} · {inspectedBytes.length} B</strong>
              <span className="serial-analyzer-inspector-time">{new Date(selected.ts).toLocaleString()}</span>
              <span className="serial-analyzer-inspector-sources">{t("capture-chunks", [selected.sourceFrameIds.length])}</span>
              {selected.protocol ? <span className="serial-analyzer-protocol">{serialAnalyzerProtocolLabel(selected)}</span> : null}
              {hasDistinctWire ? (
                <select className="serial-analyzer-byte-view" aria-label={t("detailed-byte-view")} value={inspectorView} onChange={(event) => setInspectorView(event.target.value === "wire" ? "wire" : "decoded")}>
                  <option value="decoded">{t("decoded-b", [selected.bytes.length])}</option>
                  <option value="wire">{t("wire-b", [selected.wireBytes.length])}</option>
                </select>
              ) : null}
              {selected.decodeError ? <span className="serial-analyzer-decode-error">{serialAnalyzerDecodeErrorLabel(selected.decodeError)}</span> : null}
              <button type="button" title={inspectorView === "wire" && hasDistinctWire ? t("copy-wire-hex") : t("copy-decoded-hex")} aria-label={t("copy-full-frame-hex")} onClick={() => void navigator.clipboard?.writeText(serialCaptureHex(inspectedBytes, inspectedBytes.length)).catch(() => {})}><Copy size={13} /></button>
              <button type="button" className={bookmarkIds.has(selected.bookmarkId) ? "active" : ""} title={t("toggle-bookmark")} aria-label={t("toggle-frame-bookmark")} onClick={() => toggleBookmark(selected)}><Bookmark size={13} fill={bookmarkIds.has(selected.bookmarkId) ? "currentColor" : "none"} /></button>
            </header>
            <div className="serial-analyzer-dump">
              <pre>{serialAnalyzerHexDump(inspectedBytes) || "--"}</pre>
              <pre className="ascii">{serialCaptureAscii(inspectedBytes.slice(0, 4096)) || "--"}</pre>
            </div>
          </>
        ) : <div className="serial-analyzer-empty">{t("no-frame-selected")}</div>}
      </section>

      <footer className="serial-analyzer-footer">
        <span className={message ? "message" : ""}>{message || t("frames-2", [filtered.length, analysis.frames.length])}</span>
        <label>{t("per-page")}<select value={stored.pageSize} onChange={(event) => { const pageSize = Number(event.target.value) as 100 | 250 | 500; setStored((current) => ({ ...current, pageSize, follow: false })); setPage(0); }}><option value={100}>100</option><option value={250}>250</option><option value={500}>500</option></select></label>
        <button type="button" title={t("first-page")} aria-label={t("first-page")} disabled={activePage <= 0} onClick={() => { setStored((current) => ({ ...current, follow: false })); setPage(0); }}><ChevronsLeft size={14} /></button>
        <button type="button" title={t("previous-page")} aria-label={t("previous-page")} disabled={activePage <= 0} onClick={() => { setStored((current) => ({ ...current, follow: false })); setPage(Math.max(0, activePage - 1)); }}><ChevronLeft size={14} /></button>
        <span>{activePage + 1}/{pageCount}</span>
        <button type="button" title={t("next-page")} aria-label={t("next-page")} disabled={activePage >= pageCount - 1} onClick={() => { setStored((current) => ({ ...current, follow: false })); setPage(Math.min(pageCount - 1, activePage + 1)); }}><ChevronRight size={14} /></button>
        <button type="button" title={t("last-page")} aria-label={t("last-page")} disabled={activePage >= pageCount - 1} onClick={() => { setStored((current) => ({ ...current, follow: false })); setPage(pageCount - 1); }}><ChevronsRight size={14} /></button>
      </footer>
    </>
  );
}

function loadStoredAnalyzerState(): SerialAnalyzerStoredState {
  try {
    const raw = window.localStorage.getItem(SERIAL_ANALYZER_STORAGE_KEY);
    return normalizeSerialAnalyzerStoredState(raw ? JSON.parse(raw) : defaultSerialAnalyzerStoredState);
  } catch {
    return normalizeSerialAnalyzerStoredState(defaultSerialAnalyzerStoredState);
  }
}

function loadLocalSessions(): SessionSummary[] {
  try {
    return readSessionSummaryCache(window.localStorage);
  } catch {
    return [];
  }
}

function serialAnalyzerSessionSignature(sessions: readonly SessionSummary[]): string {
  return sessions
    .map((session) => [
      session.profile.id,
      session.profile.name,
      session.profile.connection.kind,
      session.profile.connection.kind === "serial"
        ? [
            session.profile.connection.port,
            session.profile.connection.baudRate,
            session.profile.connection.dataBits,
            session.profile.connection.stopBits,
            session.profile.connection.parity,
            session.profile.connection.flowControl,
          ].join("\u0002")
        : "",
      session.runtime.status,
      session.runtime.connectedSince ?? "",
      session.runtime.lastDisconnect ?? "",
      session.runtime.lastDisconnectReason ?? "",
      session.runtime.lastActivity,
    ].join("\u0000"))
    .join("\u0001");
}

function cloneSessionSummaries(sessions: readonly SessionSummary[]): SessionSummary[] {
  if (typeof structuredClone === "function") return structuredClone(sessions) as SessionSummary[];
  return JSON.parse(JSON.stringify(sessions)) as SessionSummary[];
}

function readScreenLockMarker(fallbackLockedAt = Date.now()): ScreenLockMarker | null {
  try {
    const raw = window.localStorage.getItem(SCREEN_LOCK_STORAGE_KEY);
    return decodeStoredScreenLockMarker(raw, fallbackLockedAt)?.marker ?? null;
  } catch {
    return null;
  }
}

function screenLockMarkersEqual(left: ScreenLockMarker | null, right: ScreenLockMarker | null) {
  return left?.lockedAt === right?.lockedAt && left?.reason === right?.reason;
}

function formatAnalyzerTime(value: string): string {
  const date = new Date(value);
  return Number.isFinite(date.getTime())
    ? date.toLocaleTimeString([], { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit", fractionalSecondDigits: 3 })
    : "--";
}

function formatAnalyzerBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(2)} MiB`;
}

function serialAnalyzerLengthLabel(frame: SerialAnalyzedFrame): string {
  return serialAnalyzerHasDistinctWire(frame) ? `${frame.bytes.length}/${frame.wireBytes.length} B` : `${frame.bytes.length} B`;
}

function serialAnalyzerLengthTitle(frame: SerialAnalyzedFrame): string {
  return serialAnalyzerHasDistinctWire(frame)
    ? t("decoded-b-wire-b", [frame.bytes.length, frame.wireBytes.length])
    : `${frame.bytes.length} B`;
}

function serialAnalyzerFrameStatus(frame: SerialAnalyzedFrame): string {
  if (frame.truncated) return t("truncated");
  if (frame.decodeError === "invalidEscape") return t("escape-error");
  if (frame.decodeError === "truncatedCobs") return t("length-error");
  if (frame.decodeError === "invalidCobs") return t("encoding-error");
  if (frame.decodeError === "modbusTooShort") return t("frame-too-short");
  if (frame.decodeError === "modbusAddress") return t("address-error");
  if (frame.decodeError === "modbusCrc") return t("crc-error");
  if (frame.protocol?.kind === "modbusRtu") return "CRC OK";
  return frame.complete ? t("complete") : t("trailing-frame");
}

function serialAnalyzerDecodeErrorLabel(error: SerialAnalyzedFrame["decodeError"]): string {
  if (error === "invalidEscape") return t("invalid-slip-escape");
  if (error === "truncatedCobs") return t("truncated-cobs-length");
  if (error === "invalidCobs") return t("invalid-cobs-encoding");
  if (error === "modbusTooShort") return t("modbus-rtu-frame-too-short");
  if (error === "modbusAddress") return t("invalid-modbus-rtu-address");
  if (error === "modbusCrc") return t("modbus-rtu-crc-mismatch");
  return "";
}

function serialAnalyzerProtocolLabel(frame: SerialAnalyzedFrame): string {
  const protocol = frame.protocol;
  if (!protocol || protocol.kind !== "modbusRtu") return "";
  const functionCode = protocol.functionCode.toString(16).padStart(2, "0").toUpperCase();
  const exception = protocol.exceptionCode === null
    ? ""
    : t("exception", [protocol.exceptionCode.toString(16).padStart(2, "0").toUpperCase()]);
  return t("station-fc", [protocol.address, functionCode, exception]);
}

function formatAnalyzerError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
