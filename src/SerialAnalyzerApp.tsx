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
import { KeyedRequestGate } from "./keyed-request-gate";
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
import type {
  ExportSerialCaptureResult,
  SerialCaptureFrame,
  SerialCaptureHistorySnapshot,
  SerialCaptureSnapshot,
  SessionSummary,
} from "./types";

type SerialCaptureSource = "live" | "history";

const parserModes: Array<{ value: SerialFrameParserMode; label: string }> = [
  { value: "capture", label: "捕获" },
  { value: "delimiter", label: "分隔符" },
  { value: "fixed", label: "定长" },
  { value: "gap", label: "间隔" },
  { value: "slip", label: "SLIP" },
  { value: "cobs", label: "COBS" },
  { value: "modbus", label: "Modbus" },
];

export default function SerialAnalyzerApp({ request }: { request: SerialAnalyzerRequest }) {
  const [sessions, setSessions] = useState<SessionSummary[]>(loadLocalSessions);
  const [frames, setFrames] = useState<SerialCaptureFrame[]>([]);
  const [source, setSource] = useState<SerialCaptureSource>("live");
  const [history, setHistory] = useState<SerialCaptureHistorySnapshot | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [message, setMessage] = useState("");
  const sessionsRef = useRef<SessionSummary[]>(sessions);
  const sessionRefreshGateRef = useRef(new KeyedRequestGate<"sessions">());
  const framesRef = useRef<SerialCaptureFrame[]>([]);
  const captureRefreshRef = useRef<number | null>(null);
  const captureEpochRef = useRef(0);
  const session = sessions.find((item) => item.profile.id === request.sessionId);
  const isSerial = session?.profile.connection.kind === "serial";

  useEffect(() => {
    document.title = `${session?.profile.name ?? "串口"} - PortMate 串口分析器`;
  }, [session?.profile.name]);

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
    };

    async function refreshCapture(force = false) {
      if (!isBackendAvailable() || captureRefreshRef.current === epoch) return;
      captureRefreshRef.current = epoch;
      setRefreshing(true);
      try {
        const current = force ? [] : framesRef.current;
        const snapshot = await invokeBackend<SerialCaptureSnapshot>("list_serial_capture", {
          sessionId: request.sessionId,
          afterId: current.at(-1)?.id ?? null,
        });
        if (disposed || captureEpochRef.current !== epoch) return;
        storeFrames(mergeSerialCaptureSnapshot(current, snapshot));
        setHistory(null);
        setMessage("");
      } catch (error) {
        if (!disposed && captureEpochRef.current === epoch) setMessage(formatAnalyzerError(error));
      } finally {
        if (captureRefreshRef.current === epoch) captureRefreshRef.current = null;
        if (!disposed && captureEpochRef.current === epoch) setRefreshing(false);
      }
    }

    async function refreshHistory() {
      if (!isBackendAvailable() || captureRefreshRef.current === epoch) return;
      captureRefreshRef.current = epoch;
      setRefreshing(true);
      try {
        const snapshot = await invokeBackend<SerialCaptureHistorySnapshot>("list_serial_capture_history", {
          sessionId: request.sessionId,
        });
        if (disposed || captureEpochRef.current !== epoch) return;
        storeFrames(snapshot.frames);
        setHistory(snapshot);
        setMessage(snapshot.enabled ? "" : "Raw 日志未启用");
      } catch (error) {
        if (!disposed && captureEpochRef.current === epoch) setMessage(formatAnalyzerError(error));
      } finally {
        if (captureRefreshRef.current === epoch) captureRefreshRef.current = null;
        if (!disposed && captureEpochRef.current === epoch) setRefreshing(false);
      }
    }
  }, [request.sessionId, source]);

  function storeFrames(next: SerialCaptureFrame[]) {
    framesRef.current = next;
    setFrames(next);
  }

  function storeSessions(next: SessionSummary[]) {
    sessionsRef.current = next;
    setSessions(next);
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
    if (!isBackendAvailable() || captureRefreshRef.current === epoch) return;
    captureRefreshRef.current = epoch;
    setRefreshing(true);
    try {
      if (source === "live") {
        const snapshot = await invokeBackend<SerialCaptureSnapshot>("list_serial_capture", {
          sessionId: request.sessionId,
          afterId: null,
        });
        if (captureEpochRef.current !== epoch) return;
        storeFrames(mergeSerialCaptureSnapshot([], snapshot));
        setHistory(null);
        setMessage("");
      } else {
        const snapshot = await invokeBackend<SerialCaptureHistorySnapshot>("list_serial_capture_history", {
          sessionId: request.sessionId,
        });
        if (captureEpochRef.current !== epoch) return;
        storeFrames(snapshot.frames);
        setHistory(snapshot);
        setMessage(snapshot.enabled ? "" : "Raw 日志未启用");
      }
      await refreshSessions();
    } catch (error) {
      if (captureEpochRef.current === epoch) setMessage(formatAnalyzerError(error));
    } finally {
      if (captureRefreshRef.current === epoch) captureRefreshRef.current = null;
      if (captureEpochRef.current === epoch) setRefreshing(false);
    }
  }

  async function clearCapture() {
    if (source !== "live") return;
    if (!frames.length || !window.confirm("清空当前串口会话的全部内存捕获帧？")) return;
    captureEpochRef.current += 1;
    try {
      if (isBackendAvailable()) {
        const snapshot = await invokeBackend<SerialCaptureSnapshot>("clear_serial_capture", { sessionId: request.sessionId });
        storeFrames(mergeSerialCaptureSnapshot([], snapshot));
      } else {
        storeFrames([]);
      }
      setMessage("捕获已清空");
    } catch (error) {
      setMessage(formatAnalyzerError(error));
    }
  }

  async function exportFrames(frameIds: string[]) {
    if (!frameIds.length) return;
    try {
      const command = source === "live" ? "export_serial_capture" : "export_serial_capture_history";
      const result = await invokeBackend<ExportSerialCaptureResult>(command, {
        request: { sessionId: request.sessionId, frameIds },
      });
      setMessage(`${result.frames} 帧 · ${formatAnalyzerBytes(result.capturedBytes)} · ${result.path}`);
    } catch (error) {
      setMessage(formatAnalyzerError(error));
    }
  }

  function changeSource(next: SerialCaptureSource) {
    if (next === source || refreshing) return;
    captureEpochRef.current += 1;
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
          <strong>{session ? "会话不是串口类型" : "串口会话不可用"}</strong>
          <span>{request.sessionId}</span>
          <button type="button" onClick={() => void closeWindow()}>关闭窗口</button>
        </section>
      )}
    </main>
  );
}

function SerialAnalyzerWorkspace({
  session,
  frames,
  source,
  history,
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
  refreshing: boolean;
  message: string;
  canExport: boolean;
  onRefresh: () => void;
  onClear: () => void;
  onExport: (frameIds: string[]) => void;
  onSourceChange: (source: SerialCaptureSource) => void;
  onClose: () => void;
}) {
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
    ? `${serial.port || "未选择端口"} · ${serial.baudRate} baud · ${serial.dataBits}${serial.parity.slice(0, 1).toUpperCase()}${serial.stopBits} · ${serial.flowControl}`
    : "Serial";
  const lastDisconnect = session.runtime.lastDisconnect
    ? `${new Date(session.runtime.lastDisconnect).toLocaleString()}${session.runtime.lastDisconnectReason ? ` · ${session.runtime.lastDisconnectReason}` : ""}`
    : "--";

  return (
    <>
      <header className="serial-analyzer-titlebar">
        <span className="serial-analyzer-brand">PortMate</span>
        <strong>串口分析器</strong>
        <span className="serial-analyzer-session" title={session.profile.name}>{session.profile.name}</span>
        <span className={`serial-analyzer-connection ${session.runtime.status}`}>{session.runtime.status}</span>
        <button type="button" title="关闭串口分析器" aria-label="关闭串口分析器" onClick={onClose}><X size={17} /></button>
      </header>

      <section className="serial-analyzer-toolbar" aria-label="串口分析设置">
        <div className="serial-analyzer-segmented" aria-label="帧解析方式">
          {parserModes.map((mode) => (
            <button key={mode.value} type="button" aria-pressed={stored.parser.mode === mode.value} onClick={() => updateParser({ mode: mode.value })}>{mode.label}</button>
          ))}
        </div>
        <div className="serial-analyzer-parser-option">
          {stored.parser.mode === "delimiter" ? (
            <>
              <label><span>Hex</span><input className={delimiterValid ? "" : "invalid"} aria-label="帧分隔符 Hex" aria-invalid={!delimiterValid} value={delimiterDraft} onChange={(event) => setDelimiterDraft(event.target.value.slice(0, 128))} onBlur={commitDelimiter} onKeyDown={(event) => event.key === "Enter" && commitDelimiter()} /></label>
              <label className="serial-analyzer-check"><input type="checkbox" checked={stored.parser.includeDelimiter} onChange={(event) => updateParser({ includeDelimiter: event.target.checked })} /><span>保留</span></label>
            </>
          ) : stored.parser.mode === "fixed" ? (
            <label><span>字节</span><input type="number" min={1} max={4096} value={stored.parser.fixedLength} onChange={(event) => updateParser({ fixedLength: Number(event.target.value) })} /></label>
          ) : stored.parser.mode === "gap" ? (
            <label><span>ms</span><input type="number" min={1} max={60000} value={stored.parser.gapMs} onChange={(event) => updateParser({ gapMs: Number(event.target.value) })} /></label>
          ) : stored.parser.mode === "modbus" ? (
            <>
              <label className="serial-analyzer-check"><input type="checkbox" checked={stored.parser.modbusAutoGap !== false} onChange={(event) => updateParser({ modbusAutoGap: event.target.checked })} /><span>自动</span></label>
              {stored.parser.modbusAutoGap !== false
                ? <span className="serial-analyzer-parser-value">{serialModbusSilenceMs(serial?.baudRate ?? 115_200)} ms</span>
                : <label><span>ms</span><input type="number" min={1} max={60000} value={stored.parser.modbusGapMs ?? 2} onChange={(event) => updateParser({ modbusGapMs: Number(event.target.value) })} /></label>}
            </>
          ) : <span className="serial-analyzer-parser-value">{
            stored.parser.mode === "slip" ? "RFC 1055" : stored.parser.mode === "cobs" ? "0x00 帧界" : "读取分片"
          }</span>}
        </div>
        <div className="serial-analyzer-segmented source" aria-label="捕获数据源">
          {(["live", "history"] as const).map((value) => (
            <button key={value} type="button" disabled={refreshing} aria-pressed={source === value} onClick={() => onSourceChange(value)}>{value === "live" ? "实时" : "日志"}</button>
          ))}
        </div>
        <div className="serial-analyzer-segmented direction" aria-label="帧方向">
          {(["all", "inbound", "outbound"] as const).map((direction) => (
            <button key={direction} type="button" aria-pressed={stored.direction === direction} onClick={() => setStored((current) => ({ ...current, direction }))}>{direction === "all" ? "全部" : direction === "inbound" ? "RX" : "TX"}</button>
          ))}
        </div>
        <label className="serial-analyzer-search"><Search size={13} /><input aria-label="筛选分析帧" placeholder="Hex / ASCII" value={query} onChange={(event) => { setQuery(event.target.value); setPage(0); }} /></label>
        <button type="button" className={stored.bookmarksOnly ? "active" : ""} aria-pressed={stored.bookmarksOnly} title="只显示书签" aria-label="只显示书签" onClick={() => setStored((current) => ({ ...current, bookmarksOnly: !current.bookmarksOnly }))}><Bookmark size={14} fill={stored.bookmarksOnly ? "currentColor" : "none"} /></button>
        <button type="button" className={stored.follow ? "active" : ""} aria-pressed={stored.follow} title="跟随最新帧" aria-label="跟随最新帧" onClick={() => setStored((current) => ({ ...current, follow: !current.follow }))}><ArrowDownToLine size={14} /></button>
        <button type="button" title="刷新捕获" aria-label="刷新串口捕获" onClick={onRefresh} disabled={refreshing}><RefreshCw size={14} className={refreshing ? "spin" : ""} /></button>
        <button type="button" title="导出筛选帧" aria-label="导出筛选串口帧" disabled={!canExport || !filtered.length || (source === "history" && !history?.enabled)} onClick={exportVisible}><Download size={14} /></button>
        <button type="button" title={source === "live" ? "清空捕获" : "日志历史只可在日志管理器中清理"} aria-label="清空串口捕获" disabled={refreshing || source !== "live" || !frames.length} onClick={onClear}><Trash2 size={14} /></button>
      </section>

      <section className="serial-analyzer-status-strip">
        <span title={connectionLabel}>{connectionLabel}</span>
        <span>{source === "live" ? "实时" : "日志"}</span>
        <span>捕获 {frames.length}</span>
        <span>解析 {analysis.totalFrames}</span>
        <span>RX {rxCount}</span>
        <span>TX {txCount}</span>
        <span>{formatAnalyzerBytes(analysis.capturedBytes)}</span>
        {errorCount ? <span className="error">错误 {errorCount}</span> : null}
        {analysis.droppedFrames ? <span className="warning">窗口外 {analysis.droppedFrames}</span> : null}
        {history?.droppedFrames ? <span className="warning">日志外 {history.droppedFrames}</span> : null}
        {history?.unavailableFrames ? <span className="error">不可用 {history.unavailableFrames}</span> : null}
        <span className="serial-analyzer-last-disconnect" title={lastDisconnect}>上次断开 {lastDisconnect}</span>
      </section>

      <section
        className="serial-analyzer-table"
        role="grid"
        aria-label="串口分析帧"
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
          <span role="columnheader" aria-label="书签" />
          <span role="columnheader">时间</span>
          <span role="columnheader">方向</span>
          <span role="columnheader">长度</span>
          <span role="columnheader">边界</span>
          <span role="columnheader">Hex</span>
          <span role="columnheader">ASCII</span>
        </div>
        <div className="serial-analyzer-table-body">
          {!visible.length ? <div className="serial-analyzer-empty">没有匹配的分析帧</div> : null}
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
                <button type="button" role="gridcell" title={bookmarked ? "移除书签" : "添加书签"} aria-label={bookmarked ? "移除帧书签" : "添加帧书签"} onClick={(event) => { event.stopPropagation(); toggleBookmark(frame); }}><Bookmark size={13} fill={bookmarked ? "currentColor" : "none"} /></button>
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
              <span className="serial-analyzer-inspector-sources">{selected.sourceFrameIds.length} 个捕获分片</span>
              {selected.protocol ? <span className="serial-analyzer-protocol">{serialAnalyzerProtocolLabel(selected)}</span> : null}
              {hasDistinctWire ? (
                <select className="serial-analyzer-byte-view" aria-label="详情字节视图" value={inspectorView} onChange={(event) => setInspectorView(event.target.value === "wire" ? "wire" : "decoded")}>
                  <option value="decoded">解码 {selected.bytes.length} B</option>
                  <option value="wire">线上 {selected.wireBytes.length} B</option>
                </select>
              ) : null}
              {selected.decodeError ? <span className="serial-analyzer-decode-error">{serialAnalyzerDecodeErrorLabel(selected.decodeError)}</span> : null}
              <button type="button" title={inspectorView === "wire" && hasDistinctWire ? "复制线上 Hex" : "复制解码 Hex"} aria-label="复制完整帧 Hex" onClick={() => void navigator.clipboard?.writeText(serialCaptureHex(inspectedBytes, inspectedBytes.length)).catch(() => {})}><Copy size={13} /></button>
              <button type="button" className={bookmarkIds.has(selected.bookmarkId) ? "active" : ""} title="切换书签" aria-label="切换帧书签" onClick={() => toggleBookmark(selected)}><Bookmark size={13} fill={bookmarkIds.has(selected.bookmarkId) ? "currentColor" : "none"} /></button>
            </header>
            <div className="serial-analyzer-dump">
              <pre>{serialAnalyzerHexDump(inspectedBytes) || "--"}</pre>
              <pre className="ascii">{serialCaptureAscii(inspectedBytes.slice(0, 4096)) || "--"}</pre>
            </div>
          </>
        ) : <div className="serial-analyzer-empty">没有选中的帧</div>}
      </section>

      <footer className="serial-analyzer-footer">
        <span className={message ? "message" : ""}>{message || `${filtered.length}/${analysis.frames.length} 帧`}</span>
        <label>每页<select value={stored.pageSize} onChange={(event) => { const pageSize = Number(event.target.value) as 100 | 250 | 500; setStored((current) => ({ ...current, pageSize, follow: false })); setPage(0); }}><option value={100}>100</option><option value={250}>250</option><option value={500}>500</option></select></label>
        <button type="button" title="第一页" aria-label="第一页" disabled={activePage <= 0} onClick={() => { setStored((current) => ({ ...current, follow: false })); setPage(0); }}><ChevronsLeft size={14} /></button>
        <button type="button" title="上一页" aria-label="上一页" disabled={activePage <= 0} onClick={() => { setStored((current) => ({ ...current, follow: false })); setPage(Math.max(0, activePage - 1)); }}><ChevronLeft size={14} /></button>
        <span>{activePage + 1}/{pageCount}</span>
        <button type="button" title="下一页" aria-label="下一页" disabled={activePage >= pageCount - 1} onClick={() => { setStored((current) => ({ ...current, follow: false })); setPage(Math.min(pageCount - 1, activePage + 1)); }}><ChevronRight size={14} /></button>
        <button type="button" title="最后一页" aria-label="最后一页" disabled={activePage >= pageCount - 1} onClick={() => { setStored((current) => ({ ...current, follow: false })); setPage(pageCount - 1); }}><ChevronsRight size={14} /></button>
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
    const raw = window.localStorage.getItem("portmate.sessions");
    return raw ? JSON.parse(raw) as SessionSummary[] : [];
  } catch {
    return [];
  }
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
    ? `解码 ${frame.bytes.length} B · 线上 ${frame.wireBytes.length} B`
    : `${frame.bytes.length} B`;
}

function serialAnalyzerFrameStatus(frame: SerialAnalyzedFrame): string {
  if (frame.truncated) return "截断";
  if (frame.decodeError === "invalidEscape") return "转义错";
  if (frame.decodeError === "truncatedCobs") return "长度错";
  if (frame.decodeError === "invalidCobs") return "编码错";
  if (frame.decodeError === "modbusTooShort") return "帧太短";
  if (frame.decodeError === "modbusAddress") return "地址错";
  if (frame.decodeError === "modbusCrc") return "CRC 错";
  if (frame.protocol?.kind === "modbusRtu") return "CRC OK";
  return frame.complete ? "完整" : "尾帧";
}

function serialAnalyzerDecodeErrorLabel(error: SerialAnalyzedFrame["decodeError"]): string {
  if (error === "invalidEscape") return "无效 SLIP 转义";
  if (error === "truncatedCobs") return "COBS 长度截断";
  if (error === "invalidCobs") return "无效 COBS 编码";
  if (error === "modbusTooShort") return "Modbus RTU 帧太短";
  if (error === "modbusAddress") return "Modbus RTU 地址无效";
  if (error === "modbusCrc") return "Modbus RTU CRC 不匹配";
  return "";
}

function serialAnalyzerProtocolLabel(frame: SerialAnalyzedFrame): string {
  const protocol = frame.protocol;
  if (!protocol || protocol.kind !== "modbusRtu") return "";
  const functionCode = protocol.functionCode.toString(16).padStart(2, "0").toUpperCase();
  const exception = protocol.exceptionCode === null
    ? ""
    : ` · 异常 ${protocol.exceptionCode.toString(16).padStart(2, "0").toUpperCase()}`;
  return `站 ${protocol.address} · FC ${functionCode}${exception}`;
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
