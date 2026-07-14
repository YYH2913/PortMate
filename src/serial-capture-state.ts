import type { SerialCaptureFrame, SerialCaptureSnapshot } from "./types";

export type SerialCaptureDirectionFilter = "all" | SerialCaptureFrame["direction"];

function compactHex(bytes: number[]): string {
  return bytes.map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function serialCaptureAscii(bytes: number[]): string {
  return bytes.map((byte) => {
    if (byte === 13) return "\\r";
    if (byte === 10) return "\\n";
    if (byte === 9) return "\\t";
    return byte >= 0x20 && byte <= 0x7e ? String.fromCharCode(byte) : ".";
  }).join("");
}

export function serialCaptureHex(bytes: number[], limit = 192): string {
  const visible = bytes.slice(0, limit);
  const hex = visible.map((byte) => byte.toString(16).padStart(2, "0").toUpperCase()).join(" ");
  const remaining = bytes.length - visible.length;
  return remaining > 0 ? `${hex} ... (+${remaining} B)` : hex;
}

export function filterSerialCaptureFrames(
  frames: SerialCaptureFrame[],
  direction: SerialCaptureDirectionFilter,
  query: string,
): SerialCaptureFrame[] {
  const normalizedQuery = query.trim().toLowerCase();
  const isHexQuery = normalizedQuery.length > 0
    && /^(?:0x|[0-9a-f]|[\s,;:_-])+$/i.test(normalizedQuery);
  const hexQuery = isHexQuery ? normalizedQuery.replace(/0x/gi, "").replace(/[^0-9a-f]/gi, "") : "";
  return frames.filter((frame) => {
    if (direction !== "all" && frame.direction !== direction) return false;
    if (!normalizedQuery) return true;
    const ascii = serialCaptureAscii(frame.bytes).toLowerCase();
    return ascii.includes(normalizedQuery)
      || Boolean(hexQuery && compactHex(frame.bytes).includes(hexQuery));
  });
}

export function mergeSerialCaptureSnapshot(
  current: SerialCaptureFrame[],
  snapshot: SerialCaptureSnapshot,
): SerialCaptureFrame[] {
  if (snapshot.totalFrames === 0) return [];
  const source = snapshot.reset ? snapshot.frames : [...current, ...snapshot.frames];
  const seen = new Set<string>();
  const unique = source.filter((frame) => {
    if (seen.has(frame.id)) return false;
    seen.add(frame.id);
    return true;
  });
  return unique.slice(-snapshot.totalFrames);
}
