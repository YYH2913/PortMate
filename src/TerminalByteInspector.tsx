import { t, useLocale } from "./i18n";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent } from "react";
import {
  moveTerminalByteSelection,
  sameTerminalByteSelection,
  terminalByteCellCharacter,
  terminalByteCellLabel,
  terminalByteFollowForScroll,
  terminalByteHex,
  indexTerminalByteRows,
  terminalByteRowRange,
  terminalByteSelectionAt,
  terminalByteSelectionPosition,
  terminalByteSelectionRowIndex,
  TERMINAL_BYTE_BYTES_PER_ROW,
} from "./terminal-byte-state";
import type { TerminalByteBuffer, TerminalByteRow, TerminalByteSelection } from "./terminal-byte-state";

type TerminalByteInspectorProps = {
  snapshot: TerminalByteBuffer;
  bytesPerRow?: number;
  follow: boolean;
  selection: TerminalByteSelection | null;
  onFollowChange: (follow: boolean) => void;
  onSelectionChange: (selection: TerminalByteSelection | null) => void;
};

const TERMINAL_BYTE_HEADER_HEIGHT = 25;
const TERMINAL_BYTE_ROW_HEIGHT = 24;
const TERMINAL_BYTE_ROW_OVERSCAN = 4;

export default function TerminalByteInspector({
  snapshot,
  bytesPerRow = TERMINAL_BYTE_BYTES_PER_ROW,
  follow,
  selection,
  onFollowChange,
  onSelectionChange,
}: TerminalByteInspectorProps) {
  const { locale } = useLocale();
  const byteInfo = useMemo(() => Array.from({ length: 256 }, (_, byte) => ({ hex: terminalByteHex(byte), label: terminalByteCellLabel(byte), char: terminalByteCellCharacter(byte) })), [locale]);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const rowWidth = Math.max(1, Math.trunc(bytesPerRow));
  const rowIndex = useMemo(() => indexTerminalByteRows(snapshot.frames, rowWidth), [rowWidth, snapshot.frames]);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const [hovered, setHovered] = useState<TerminalByteSelection | null>(null);
  const highlighted = hovered ?? selection;
  const contentHeight = TERMINAL_BYTE_HEADER_HEIGHT + rowIndex.rowCount * TERMINAL_BYTE_ROW_HEIGHT;
  const viewportTop = follow ? Math.max(0, contentHeight - viewportHeight)
    : Math.min(scrollTop, Math.max(0, contentHeight - viewportHeight));
  const firstVisibleRow = Math.max(
    0,
    Math.floor((viewportTop - TERMINAL_BYTE_HEADER_HEIGHT) / TERMINAL_BYTE_ROW_HEIGHT)
      - TERMINAL_BYTE_ROW_OVERSCAN,
  );
  const visibleRowCount = Math.ceil(viewportHeight / TERMINAL_BYTE_ROW_HEIGHT)
    + TERMINAL_BYTE_ROW_OVERSCAN * 2;
  const visibleRows = useMemo(() => terminalByteRowRange(rowIndex, firstVisibleRow, Math.max(1, visibleRowCount)), [rowIndex, firstVisibleRow, visibleRowCount]);

  useEffect(() => {
    const scroll = scrollRef.current;
    if (!scroll) return;
    const measure = () => setViewportHeight(scroll.clientHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(scroll);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const currentPosition = terminalByteSelectionPosition(snapshot.frames, selection);
    if (!snapshot.frames.length) {
      if (selection) onSelectionChange(null);
      return;
    }
    if (currentPosition !== null) return;
    onSelectionChange(moveTerminalByteSelection(snapshot.frames, null, -1));
  }, [snapshot.frames, selection?.byteIndex, selection?.frameId]);

  useEffect(() => {
    if (!follow) return;
    const frame = window.requestAnimationFrame(() => {
      const scroll = scrollRef.current;
      if (!scroll) return;
      scroll.scrollTop = scroll.scrollHeight;
      setScrollTop(scroll.scrollTop);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [follow, rowWidth, snapshot.revision]);

  function revealSelection(next: TerminalByteSelection | null) {
    const scroll = scrollRef.current;
    const rowIndex = terminalByteSelectionRowIndex(snapshot.frames, next, rowWidth);
    if (!scroll || rowIndex === null) return;
    const rowTop = TERMINAL_BYTE_HEADER_HEIGHT + rowIndex * TERMINAL_BYTE_ROW_HEIGHT;
    const visibleTop = scroll.scrollTop + TERMINAL_BYTE_HEADER_HEIGHT;
    const visibleBottom = scroll.scrollTop + scroll.clientHeight;
    if (rowTop < visibleTop) scroll.scrollTop = Math.max(0, rowTop - TERMINAL_BYTE_HEADER_HEIGHT);
    else if (rowTop + TERMINAL_BYTE_ROW_HEIGHT > visibleBottom) {
      scroll.scrollTop = rowTop + TERMINAL_BYTE_ROW_HEIGHT - scroll.clientHeight;
    }
  }

  function selectByte(next: TerminalByteSelection) {
    onSelectionChange(next);
    scrollRef.current?.focus({ preventScroll: true });
  }

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    let next: TerminalByteSelection | null = null;
    if (event.key === "ArrowLeft") next = moveTerminalByteSelection(snapshot.frames, selection, -1);
    else if (event.key === "ArrowRight") next = moveTerminalByteSelection(snapshot.frames, selection, 1);
    else if (event.key === "ArrowUp") next = moveTerminalByteSelection(snapshot.frames, selection, -rowWidth);
    else if (event.key === "ArrowDown") next = moveTerminalByteSelection(snapshot.frames, selection, rowWidth);
    else if (event.key === "Home") next = terminalByteSelectionAt(snapshot.frames, 0);
    else if (event.key === "End") next = terminalByteSelectionAt(snapshot.frames, Number.MAX_SAFE_INTEGER);
    else return;
    event.preventDefault();
    setHovered(null);
    onFollowChange(false);
    onSelectionChange(next);
    revealSelection(next);
  }

  return (
    <section className="terminal-byte-inspector" aria-label={t("terminal-byte-inspector")} data-bytes-per-row={rowWidth}>
      <div
        ref={scrollRef}
        className="terminal-byte-scroll"
        role="grid"
        aria-label={t("live-terminal-hex-and-ascii")}
        aria-colcount={6}
        aria-rowcount={rowIndex.rowCount + 1}
        tabIndex={0}
        onKeyDown={handleKeyDown}
        onScroll={(event) => {
          const scroll = event.currentTarget;
          setScrollTop(scroll.scrollTop);
          onFollowChange(terminalByteFollowForScroll(
            scroll.scrollTop,
            scroll.clientHeight,
            scroll.scrollHeight,
          ));
        }}
      >
        <div
          className="terminal-byte-content"
          onClick={event => { const cell = byteSelectionFromTarget(event.target); if (cell) selectByte(cell); }}
          onMouseOver={event => {
            const cell = byteSelectionFromTarget(event.target);
            setHovered(current => sameTerminalByteSelection(current, cell) ? current : cell);
          }}
          onMouseLeave={() => setHovered(null)}
          style={{ height: `${contentHeight}px` } as CSSProperties}
        >
          <div className="terminal-byte-header" role="row" aria-rowindex={1}>
            <span role="columnheader">{t("time")}</span>
            <span role="columnheader">{t("direction")}</span>
            <span role="columnheader">{t("offset")}</span>
            <span role="columnheader">Hex</span>
            <span role="columnheader">ASCII</span>
            <span role="columnheader">{t("status")}</span>
          </div>
          {!rowIndex.rowCount ? <div className="terminal-byte-empty">{t("waiting-for-live-bytes")}</div> : null}
          {/* Rows have no local state; reuse viewport slots while capture IDs
              and every interaction target follow the current row props. */}
          {visibleRows.map((row, visibleIndex) => <ByteRowView
            key={visibleIndex} row={row} rowIndex={firstVisibleRow + visibleIndex} rowWidth={rowWidth} byteInfo={byteInfo}
            selectionIndex={selectionIndexForRow(row, selection)} highlightedIndex={selectionIndexForRow(row, highlighted)}
          />)}
        </div>
      </div>
    </section>
  );
}

type ByteInfo = { hex: string; label: string; char: string };
type ByteRowViewProps = { row: TerminalByteRow; rowIndex: number; rowWidth: number; byteInfo: readonly ByteInfo[]; selectionIndex: number; highlightedIndex: number };

const ByteRowView = memo(function ByteRowView({ row, rowIndex, rowWidth, byteInfo, selectionIndex, highlightedIndex }: ByteRowViewProps) {
  const { locale } = useLocale();
  const timestamp = formatByteTimestamp(row.ts, locale);
  return (
              <div
                className={`terminal-byte-row ${row.direction}`}
                role="row"
                aria-rowindex={rowIndex + 2}
                style={{ top: `${TERMINAL_BYTE_HEADER_HEIGHT + rowIndex * TERMINAL_BYTE_ROW_HEIGHT}px` }}
                data-frame-id={row.frameId}
                data-row-offset={row.offset}
                data-frame-byte-offset={row.frameByteOffset}
              >
                <span role="gridcell" title={timestamp.title}>{timestamp.time}</span>
                <strong role="gridcell" title={`${row.direction === "inbound" ? t("receive") : t("send")} · ${row.stream}`}>
                  {row.direction === "inbound" ? "RX" : "TX"}
                </strong>
                <code role="gridcell" title={t("decimal-offset", [row.offset])}>{row.offset.toString(16).padStart(8, "0").toUpperCase()}</code>
                <div className="terminal-byte-hex-cells" role="gridcell">
                  {Array.from({ length: rowWidth }, (_, index) => {
                    const byte = row.bytes[index];
                    if (byte === undefined) return <span key={index} className="terminal-byte-cell-placeholder" aria-hidden="true" />;
                    const selected = selectionIndex === index;
                    const linked = highlightedIndex === index;
                    return (
                      <button
                        key={index}
                        type="button"
                        className={`${selected ? "selected " : ""}${linked ? "linked" : ""}`.trim()}
                        aria-label={t("offset-hex", [row.direction === "inbound" ? t("receive") : t("send"), row.offset + index, byteInfo[byte].hex, byteInfo[byte].label])}
                        aria-pressed={selected}
                        title={`0x${(row.offset + index).toString(16).padStart(8, "0").toUpperCase()} · ${byteInfo[byte].label}`}
                        tabIndex={-1}
                        data-byte-column="hex"
                        data-byte-cell={index}
                      >{byteInfo[byte].hex}</button>
                    );
                  })}
                </div>
                <div className="terminal-byte-ascii-cells" role="gridcell">
                  {Array.from({ length: rowWidth }, (_, index) => {
                    const byte = row.bytes[index];
                    if (byte === undefined) return <span key={index} className="terminal-byte-cell-placeholder" aria-hidden="true" />;
                    const selected = selectionIndex === index;
                    const linked = highlightedIndex === index;
                    return (
                      <button
                        key={index}
                        type="button"
                        className={`${selected ? "selected " : ""}${linked ? "linked" : ""}`.trim()}
                        aria-label={t("offset-ascii-hex", [row.direction === "inbound" ? t("receive") : t("send"), row.offset + index, byteInfo[byte].label, byteInfo[byte].hex])}
                        aria-pressed={selected}
                        title={`0x${(row.offset + index).toString(16).padStart(8, "0").toUpperCase()} · ${byteInfo[byte].label}`}
                        tabIndex={-1}
                        data-byte-column="ascii"
                        data-byte-cell={index}
                      >{byteInfo[byte].char}</button>
                    );
                  })}
                </div>
                <span className={row.omittedBytes ? "terminal-byte-status truncated" : "terminal-byte-status"} role="gridcell" title={row.omittedBytes ? t("another-b-from-this-transport-frame-are-outside-the", [row.omittedBytes]) : row.stream}>
                  {row.omittedBytes ? `+${row.omittedBytes} B` : row.stream === "stderr" ? "ERR" : ""}
                </span>
              </div>
  );
}, (before, after) => before.row.id === after.row.id && before.row.offset === after.row.offset
  && before.row.ts === after.row.ts && before.row.direction === after.row.direction && before.row.stream === after.row.stream
  && before.row.omittedBytes === after.row.omittedBytes && before.row.bytes.length === after.row.bytes.length
  && before.rowIndex === after.rowIndex && before.rowWidth === after.rowWidth && before.byteInfo === after.byteInfo
  && before.selectionIndex === after.selectionIndex && before.highlightedIndex === after.highlightedIndex);

function selectionIndexForRow(row: TerminalByteRow, selection: TerminalByteSelection | null): number {
  if (selection?.frameId !== row.frameId) return -1;
  const index = selection.byteIndex - row.frameByteOffset;
  return index >= 0 && index < row.bytes.length ? index : -1;
}

function byteSelectionFromTarget(target: EventTarget | null): TerminalByteSelection | null {
  if (!(target instanceof Element)) return null;
  const button = target.closest<HTMLElement>("button[data-byte-cell]");
  const row = button?.closest<HTMLElement>(".terminal-byte-row");
  const frameId = row?.dataset.frameId;
  const byteIndex = Number(row?.dataset.frameByteOffset) + Number(button?.dataset.byteCell);
  return frameId && Number.isSafeInteger(byteIndex) && byteIndex >= 0 ? { frameId, byteIndex } : null;
}

const byteTimestampCache = new Map<string, { title: string; time: string }>();
function formatByteTimestamp(value: string, locale: string): { title: string; time: string } {
  const key = `${locale}\0${value}`;
  const cached = byteTimestampCache.get(key);
  if (cached) return cached;
  const date = new Date(value);
  const result = Number.isNaN(date.getTime()) ? { title: value, time: "--:--:--" } : {
    title: date.toLocaleString(),
    time: `${twoDigits(date.getHours())}:${twoDigits(date.getMinutes())}:${twoDigits(date.getSeconds())}`,
  };
  if (value.length <= 128) {
    if (byteTimestampCache.size >= 512) byteTimestampCache.delete(byteTimestampCache.keys().next().value!);
    byteTimestampCache.set(key, result);
  }
  return result;
}

function twoDigits(value: number): string {
  return String(value).padStart(2, "0");
}
