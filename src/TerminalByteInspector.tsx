import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent } from "react";
import {
  moveTerminalByteSelection,
  sameTerminalByteSelection,
  terminalByteCellCharacter,
  terminalByteCellLabel,
  terminalByteFollowForScroll,
  terminalByteHex,
  terminalByteRows,
  terminalByteSelectionAt,
  terminalByteSelectionKey,
  terminalByteSelectionPosition,
  terminalByteSelectionRowIndex,
  TERMINAL_BYTE_BYTES_PER_ROW,
} from "./terminal-byte-state";
import type { TerminalByteBuffer, TerminalByteSelection } from "./terminal-byte-state";

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
const TERMINAL_BYTE_ROW_OVERSCAN = 10;

export default function TerminalByteInspector({
  snapshot,
  bytesPerRow = TERMINAL_BYTE_BYTES_PER_ROW,
  follow,
  selection,
  onFollowChange,
  onSelectionChange,
}: TerminalByteInspectorProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const rowWidth = Math.max(1, Math.trunc(bytesPerRow));
  const rows = useMemo(() => terminalByteRows(snapshot.frames, rowWidth), [rowWidth, snapshot.frames]);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const [hovered, setHovered] = useState<TerminalByteSelection | null>(null);
  const highlighted = hovered ?? selection;
  const firstVisibleRow = Math.max(
    0,
    Math.floor((scrollTop - TERMINAL_BYTE_HEADER_HEIGHT) / TERMINAL_BYTE_ROW_HEIGHT)
      - TERMINAL_BYTE_ROW_OVERSCAN,
  );
  const visibleRowCount = Math.ceil(viewportHeight / TERMINAL_BYTE_ROW_HEIGHT)
    + TERMINAL_BYTE_ROW_OVERSCAN * 2;
  const lastVisibleRow = Math.min(rows.length, firstVisibleRow + Math.max(1, visibleRowCount));
  const visibleRows = rows.slice(firstVisibleRow, lastVisibleRow);
  const contentHeight = TERMINAL_BYTE_HEADER_HEIGHT + rows.length * TERMINAL_BYTE_ROW_HEIGHT;

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
    <section className="terminal-byte-inspector" aria-label="终端字节检查器" data-bytes-per-row={rowWidth}>
      <div
        ref={scrollRef}
        className="terminal-byte-scroll"
        role="grid"
        aria-label="实时终端 Hex 与 ASCII"
        aria-colcount={6}
        aria-rowcount={rows.length + 1}
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
          style={{ height: `${contentHeight}px` } as CSSProperties}
        >
          <div className="terminal-byte-header" role="row" aria-rowindex={1}>
            <span role="columnheader">时间</span>
            <span role="columnheader">方向</span>
            <span role="columnheader">偏移</span>
            <span role="columnheader">Hex</span>
            <span role="columnheader">ASCII</span>
            <span role="columnheader">状态</span>
          </div>
          {!rows.length ? <div className="terminal-byte-empty">等待实时字节</div> : null}
          {visibleRows.map((row, visibleIndex) => {
            const rowIndex = firstVisibleRow + visibleIndex;
            return (
              <div
                key={row.id}
                className={`terminal-byte-row ${row.direction}`}
                role="row"
                aria-rowindex={rowIndex + 2}
                style={{ top: `${TERMINAL_BYTE_HEADER_HEIGHT + rowIndex * TERMINAL_BYTE_ROW_HEIGHT}px` }}
                data-frame-id={row.frameId}
                data-row-offset={row.offset}
              >
                <span role="gridcell" title={formatTerminalByteTimestamp(row.ts)}>{formatTerminalByteTime(row.ts)}</span>
                <strong role="gridcell" title={`${row.direction === "inbound" ? "接收" : "发送"} · ${row.stream}`}>
                  {row.direction === "inbound" ? "RX" : "TX"}
                </strong>
                <code role="gridcell" title={`十进制偏移 ${row.offset}`}>{row.offset.toString(16).padStart(8, "0").toUpperCase()}</code>
                <div className="terminal-byte-hex-cells" role="gridcell">
                  {Array.from({ length: rowWidth }, (_, index) => {
                    const byte = row.bytes[index];
                    if (byte === undefined) return <span key={index} className="terminal-byte-cell-placeholder" aria-hidden="true" />;
                    const cell = { frameId: row.frameId, byteIndex: row.frameByteOffset + index };
                    const selected = sameTerminalByteSelection(selection, cell);
                    const linked = sameTerminalByteSelection(highlighted, cell);
                    const key = terminalByteSelectionKey(cell);
                    return (
                      <button
                        key={index}
                        type="button"
                        className={`${selected ? "selected " : ""}${linked ? "linked" : ""}`.trim()}
                        aria-label={`${row.direction === "inbound" ? "接收" : "发送"}偏移 ${row.offset + index}，Hex ${terminalByteHex(byte)}，${terminalByteCellLabel(byte)}`}
                        aria-pressed={selected}
                        title={`0x${(row.offset + index).toString(16).padStart(8, "0").toUpperCase()} · ${terminalByteCellLabel(byte)}`}
                        tabIndex={-1}
                        data-byte-key={key}
                        data-byte-column="hex"
                        onMouseEnter={() => setHovered(cell)}
                        onMouseLeave={() => setHovered(null)}
                        onClick={() => selectByte(cell)}
                      >{terminalByteHex(byte)}</button>
                    );
                  })}
                </div>
                <div className="terminal-byte-ascii-cells" role="gridcell">
                  {Array.from({ length: rowWidth }, (_, index) => {
                    const byte = row.bytes[index];
                    if (byte === undefined) return <span key={index} className="terminal-byte-cell-placeholder" aria-hidden="true" />;
                    const cell = { frameId: row.frameId, byteIndex: row.frameByteOffset + index };
                    const selected = sameTerminalByteSelection(selection, cell);
                    const linked = sameTerminalByteSelection(highlighted, cell);
                    const key = terminalByteSelectionKey(cell);
                    return (
                      <button
                        key={index}
                        type="button"
                        className={`${selected ? "selected " : ""}${linked ? "linked" : ""}`.trim()}
                        aria-label={`${row.direction === "inbound" ? "接收" : "发送"}偏移 ${row.offset + index}，ASCII ${terminalByteCellLabel(byte)}，Hex ${terminalByteHex(byte)}`}
                        aria-pressed={selected}
                        title={`0x${(row.offset + index).toString(16).padStart(8, "0").toUpperCase()} · ${terminalByteCellLabel(byte)}`}
                        tabIndex={-1}
                        data-byte-key={key}
                        data-byte-column="ascii"
                        onMouseEnter={() => setHovered(cell)}
                        onMouseLeave={() => setHovered(null)}
                        onClick={() => selectByte(cell)}
                      >{terminalByteCellCharacter(byte)}</button>
                    );
                  })}
                </div>
                <span className={row.omittedBytes ? "terminal-byte-status truncated" : "terminal-byte-status"} role="gridcell" title={row.omittedBytes ? `该传输帧还有 ${row.omittedBytes} B 未进入实时窗口` : row.stream}>
                  {row.omittedBytes ? `+${row.omittedBytes} B` : row.stream === "stderr" ? "ERR" : ""}
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}

function formatTerminalByteTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "--:--:--";
  return `${twoDigits(date.getHours())}:${twoDigits(date.getMinutes())}:${twoDigits(date.getSeconds())}`;
}

function formatTerminalByteTimestamp(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function twoDigits(value: number): string {
  return String(value).padStart(2, "0");
}
