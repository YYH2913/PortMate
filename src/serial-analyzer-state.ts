import { serialCaptureAscii } from "./serial-capture-state";
import type { SerialCaptureDirectionFilter } from "./serial-capture-state";
import type { SerialCaptureFrame } from "./types";

export const SERIAL_ANALYZER_STORAGE_KEY = "portmate.serialAnalyzer.v1";
export const MAX_SERIAL_ANALYZED_FRAMES = 4096;
export const MAX_SERIAL_ANALYZER_BOOKMARKS = 128;
export const MAX_SERIAL_DELIMITER_BYTES = 32;
export const MIN_SERIAL_FIXED_LENGTH = 1;
export const MAX_SERIAL_FIXED_LENGTH = 4096;
export const MIN_SERIAL_GAP_MS = 1;
export const MAX_SERIAL_GAP_MS = 60_000;

export type SerialFrameParserMode = "capture" | "delimiter" | "fixed" | "gap";

export interface SerialFrameParserConfig {
  mode: SerialFrameParserMode;
  delimiterHex: string;
  includeDelimiter: boolean;
  fixedLength: number;
  gapMs: number;
}

export interface SerialAnalyzedFrame {
  id: string;
  ts: string;
  endTs: string;
  direction: SerialCaptureFrame["direction"];
  bytes: number[];
  complete: boolean;
  truncated: boolean;
  sourceFrameIds: string[];
  bookmarkId: string;
}

export interface SerialAnalysisResult {
  frames: SerialAnalyzedFrame[];
  totalFrames: number;
  droppedFrames: number;
  capturedBytes: number;
}

export interface SerialAnalyzerStoredState {
  version: 1;
  parser: SerialFrameParserConfig;
  direction: SerialCaptureDirectionFilter;
  pageSize: 100 | 250 | 500;
  follow: boolean;
  bookmarksOnly: boolean;
  bookmarks: Record<string, string[]>;
}

export const defaultSerialFrameParserConfig: SerialFrameParserConfig = {
  mode: "capture",
  delimiterHex: "0D 0A",
  includeDelimiter: true,
  fixedLength: 8,
  gapMs: 20,
};

export const defaultSerialAnalyzerStoredState: SerialAnalyzerStoredState = {
  version: 1,
  parser: defaultSerialFrameParserConfig,
  direction: "all",
  pageSize: 250,
  follow: true,
  bookmarksOnly: false,
  bookmarks: {},
};

export function normalizeSerialFrameParserConfig(value: unknown): SerialFrameParserConfig {
  const source = objectValue(value);
  const mode = source.mode === "delimiter" || source.mode === "fixed" || source.mode === "gap"
    ? source.mode
    : "capture";
  const delimiter = serialAnalyzerDelimiterBytes(source.delimiterHex);
  return {
    mode,
    delimiterHex: delimiter ? formatSerialHexBytes(delimiter) : defaultSerialFrameParserConfig.delimiterHex,
    includeDelimiter: source.includeDelimiter !== false,
    fixedLength: boundedInteger(source.fixedLength, MIN_SERIAL_FIXED_LENGTH, MAX_SERIAL_FIXED_LENGTH, defaultSerialFrameParserConfig.fixedLength),
    gapMs: boundedInteger(source.gapMs, MIN_SERIAL_GAP_MS, MAX_SERIAL_GAP_MS, defaultSerialFrameParserConfig.gapMs),
  };
}

export function normalizeSerialAnalyzerStoredState(value: unknown): SerialAnalyzerStoredState {
  const source = objectValue(value);
  const pageSize = source.pageSize === 100 || source.pageSize === 500 ? source.pageSize : 250;
  const direction = source.direction === "inbound" || source.direction === "outbound" ? source.direction : "all";
  return {
    version: 1,
    parser: normalizeSerialFrameParserConfig(source.parser),
    direction,
    pageSize,
    follow: source.follow !== false,
    bookmarksOnly: source.bookmarksOnly === true,
    bookmarks: normalizeSerialAnalyzerBookmarks(source.bookmarks),
  };
}

export function serialAnalyzerDelimiterBytes(value: unknown): number[] | null {
  if (typeof value !== "string" || !value.trim() || /[^0-9a-fx\s,;:_-]/i.test(value)) return null;
  const compact = value.replace(/0x/gi, "").replace(/[^0-9a-f]/gi, "");
  if (!compact.length || compact.length % 2 !== 0 || compact.length > MAX_SERIAL_DELIMITER_BYTES * 2) return null;
  return compact.match(/../g)?.map((byte) => Number.parseInt(byte, 16)) ?? null;
}

export function analyzeSerialCaptureFrames(
  sourceFrames: SerialCaptureFrame[],
  requestedConfig: SerialFrameParserConfig,
): SerialAnalysisResult {
  const config = normalizeSerialFrameParserConfig(requestedConfig);
  const collector = new SerialAnalyzedFrameCollector();
  const frames = sourceFrames.filter(validSourceFrame);
  const capturedBytes = frames.reduce((total, frame) => total + frame.bytes.length, 0);
  if (config.mode === "capture") analyzeCaptureFrames(frames, collector);
  else if (config.mode === "delimiter") analyzeDelimitedFrames(frames, config, collector);
  else if (config.mode === "fixed") analyzeFixedFrames(frames, config.fixedLength, collector);
  else analyzeGapFrames(frames, config.gapMs, collector);
  return collector.result(capturedBytes);
}

export function filterSerialAnalyzedFrames(
  frames: SerialAnalyzedFrame[],
  direction: SerialCaptureDirectionFilter,
  query: string,
  bookmarkIds: ReadonlySet<string> = new Set(),
  bookmarksOnly = false,
): SerialAnalyzedFrame[] {
  const normalizedQuery = query.trim().toLowerCase();
  const hexQuery = serialAnalyzerSearchHex(normalizedQuery);
  return frames.filter((frame) => {
    if (direction !== "all" && frame.direction !== direction) return false;
    if (bookmarksOnly && !bookmarkIds.has(frame.bookmarkId)) return false;
    if (!normalizedQuery) return true;
    const ascii = serialCaptureAscii(frame.bytes).toLowerCase();
    return ascii.includes(normalizedQuery) || Boolean(hexQuery && compactHex(frame.bytes).includes(hexQuery));
  });
}

export function toggleSerialAnalyzerBookmark(
  state: SerialAnalyzerStoredState,
  sessionId: string,
  sourceFrameId: string,
): SerialAnalyzerStoredState {
  const cleanSessionId = cleanIdentifier(sessionId, 256);
  const cleanFrameId = cleanIdentifier(sourceFrameId, 128);
  if (!cleanSessionId || !cleanFrameId) return state;
  const current = state.bookmarks[cleanSessionId] ?? [];
  const exists = current.includes(cleanFrameId);
  const next = exists
    ? current.filter((id) => id !== cleanFrameId)
    : [...current.filter((id) => id !== cleanFrameId), cleanFrameId].slice(-MAX_SERIAL_ANALYZER_BOOKMARKS);
  const bookmarks = { ...state.bookmarks };
  if (next.length) bookmarks[cleanSessionId] = next;
  else delete bookmarks[cleanSessionId];
  return { ...state, bookmarks };
}

export function serialAnalyzerHexDump(bytes: number[], limit = 4096): string {
  const boundedLimit = Math.max(16, Math.min(64 * 1024, Math.trunc(limit) || 4096));
  const visible = bytes.slice(0, boundedLimit);
  const lines: string[] = [];
  for (let offset = 0; offset < visible.length; offset += 16) {
    const chunk = visible.slice(offset, offset + 16);
    const hex = chunk.map((byte) => byte.toString(16).padStart(2, "0").toUpperCase()).join(" ").padEnd(47, " ");
    lines.push(`${offset.toString(16).padStart(8, "0").toUpperCase()}  ${hex}  ${serialCaptureAscii(chunk)}`);
  }
  if (visible.length < bytes.length) lines.push(`... ${bytes.length - visible.length} more bytes`);
  return lines.join("\n");
}

function analyzeCaptureFrames(frames: SerialCaptureFrame[], collector: SerialAnalyzedFrameCollector) {
  for (const frame of frames) {
    collector.push({
      id: `capture:${frame.id}`,
      ts: frame.ts,
      endTs: frame.ts,
      direction: frame.direction,
      bytes: [...frame.bytes],
      complete: true,
      truncated: frame.truncated,
      sourceFrameIds: [frame.id],
      bookmarkId: frame.id,
    });
  }
}

function analyzeDelimitedFrames(
  frames: SerialCaptureFrame[],
  config: SerialFrameParserConfig,
  collector: SerialAnalyzedFrameCollector,
) {
  const delimiter = serialAnalyzerDelimiterBytes(config.delimiterHex) ?? [13, 10];
  let pending: PendingSerialFrame | null = null;
  const flush = (complete: boolean) => {
    if (!pending) return;
    const bytes = complete && !config.includeDelimiter
      ? pending.bytes.slice(0, Math.max(0, pending.bytes.length - delimiter.length))
      : [...pending.bytes];
    collector.push(pendingSerialFrame(pending, bytes, complete, "delimiter", collector.totalFrames));
    pending = null;
  };
  for (const frame of frames) {
    if (pending && pending.direction !== frame.direction) flush(false);
    for (const byte of frame.bytes) {
      pending = appendPendingSerialByte(pending, frame, byte);
      if (endsWithBytes(pending.bytes, delimiter)) flush(true);
    }
  }
  flush(false);
}

function analyzeFixedFrames(
  frames: SerialCaptureFrame[],
  fixedLength: number,
  collector: SerialAnalyzedFrameCollector,
) {
  let pending: PendingSerialFrame | null = null;
  const flush = (complete: boolean) => {
    if (!pending) return;
    collector.push(pendingSerialFrame(pending, [...pending.bytes], complete, "fixed", collector.totalFrames));
    pending = null;
  };
  for (const frame of frames) {
    if (pending && pending.direction !== frame.direction) flush(false);
    for (const byte of frame.bytes) {
      pending = appendPendingSerialByte(pending, frame, byte);
      if (pending.bytes.length === fixedLength) flush(true);
    }
  }
  flush(false);
}

function analyzeGapFrames(
  frames: SerialCaptureFrame[],
  gapMs: number,
  collector: SerialAnalyzedFrameCollector,
) {
  let pending: PendingSerialFrame | null = null;
  const flush = (complete: boolean) => {
    if (!pending) return;
    collector.push(pendingSerialFrame(pending, [...pending.bytes], complete, "gap", collector.totalFrames));
    pending = null;
  };
  for (const frame of frames) {
    const currentTime = Date.parse(frame.ts);
    const previousTime = pending ? Date.parse(pending.endTs) : Number.NaN;
    const boundary = pending && (
      pending.direction !== frame.direction
      || !Number.isFinite(currentTime)
      || !Number.isFinite(previousTime)
      || currentTime - previousTime > gapMs
    );
    if (boundary) flush(true);
    for (const byte of frame.bytes) pending = appendPendingSerialByte(pending, frame, byte);
  }
  flush(false);
}

type PendingSerialFrame = {
  ts: string;
  endTs: string;
  direction: SerialCaptureFrame["direction"];
  bytes: number[];
  truncated: boolean;
  sourceFrameIds: string[];
};

function appendPendingSerialByte(
  pending: PendingSerialFrame | null,
  frame: SerialCaptureFrame,
  byte: number,
): PendingSerialFrame {
  const next = pending ?? {
    ts: frame.ts,
    endTs: frame.ts,
    direction: frame.direction,
    bytes: [],
    truncated: false,
    sourceFrameIds: [],
  };
  next.bytes.push(byte);
  next.endTs = frame.ts;
  next.truncated ||= frame.truncated;
  if (next.sourceFrameIds.at(-1) !== frame.id) next.sourceFrameIds.push(frame.id);
  return next;
}

function pendingSerialFrame(
  pending: PendingSerialFrame,
  bytes: number[],
  complete: boolean,
  mode: SerialFrameParserMode,
  sequence: number,
): SerialAnalyzedFrame {
  return {
    id: `${mode}:${pending.sourceFrameIds[0]}:${pending.sourceFrameIds.at(-1)}:${sequence}`,
    ts: pending.ts,
    endTs: pending.endTs,
    direction: pending.direction,
    bytes,
    complete,
    truncated: pending.truncated,
    sourceFrameIds: [...pending.sourceFrameIds],
    bookmarkId: pending.sourceFrameIds[0],
  };
}

class SerialAnalyzedFrameCollector {
  private readonly ring: Array<SerialAnalyzedFrame | undefined> = Array(MAX_SERIAL_ANALYZED_FRAMES);
  totalFrames = 0;

  push(frame: SerialAnalyzedFrame) {
    this.ring[this.totalFrames % MAX_SERIAL_ANALYZED_FRAMES] = frame;
    this.totalFrames += 1;
  }

  result(capturedBytes: number): SerialAnalysisResult {
    const visibleCount = Math.min(this.totalFrames, MAX_SERIAL_ANALYZED_FRAMES);
    const start = this.totalFrames - visibleCount;
    const frames: SerialAnalyzedFrame[] = [];
    for (let index = start; index < this.totalFrames; index += 1) {
      const frame = this.ring[index % MAX_SERIAL_ANALYZED_FRAMES];
      if (frame) frames.push(frame);
    }
    return {
      frames,
      totalFrames: this.totalFrames,
      droppedFrames: this.totalFrames - frames.length,
      capturedBytes,
    };
  }
}

function normalizeSerialAnalyzerBookmarks(value: unknown): Record<string, string[]> {
  const source = objectValue(value);
  const result: Record<string, string[]> = {};
  for (const [rawSessionId, rawIds] of Object.entries(source).slice(0, 32)) {
    const sessionId = cleanIdentifier(rawSessionId, 256);
    if (!sessionId || !Array.isArray(rawIds)) continue;
    const ids = rawIds
      .map((id) => cleanIdentifier(id, 128))
      .filter((id, index, all) => Boolean(id) && all.indexOf(id) === index)
      .slice(-MAX_SERIAL_ANALYZER_BOOKMARKS);
    if (ids.length) result[sessionId] = ids;
  }
  return result;
}

function serialAnalyzerSearchHex(value: string): string {
  if (!value || !/^(?:0x|[0-9a-f]|[\s,;:_-])+$/i.test(value)) return "";
  const compact = value.replace(/0x/gi, "").replace(/[^0-9a-f]/gi, "");
  return compact.length % 2 === 0 ? compact : "";
}

function formatSerialHexBytes(bytes: number[]): string {
  return bytes.map((byte) => byte.toString(16).padStart(2, "0").toUpperCase()).join(" ");
}

function compactHex(bytes: number[]): string {
  return bytes.map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function endsWithBytes(bytes: number[], suffix: number[]): boolean {
  if (bytes.length < suffix.length) return false;
  const offset = bytes.length - suffix.length;
  return suffix.every((byte, index) => bytes[offset + index] === byte);
}

function validSourceFrame(frame: SerialCaptureFrame): boolean {
  return Boolean(
    frame
    && cleanIdentifier(frame.id, 128)
    && (frame.direction === "inbound" || frame.direction === "outbound")
    && Array.isArray(frame.bytes)
    && frame.bytes.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255),
  );
}

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function boundedInteger(value: unknown, minimum: number, maximum: number, fallback: number): number {
  const number = typeof value === "number" && Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.min(maximum, Math.max(minimum, number));
}

function cleanIdentifier(value: unknown, maximum: number): string {
  if (typeof value !== "string" || /[\u0000-\u001f\u007f]/.test(value)) return "";
  const clean = value.trim();
  return clean && clean.length <= maximum ? clean : "";
}
