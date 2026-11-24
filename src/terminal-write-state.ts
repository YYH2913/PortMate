import type { SessionEvent } from "./types";

/** Outbound capture/empty acknowledgements never change the terminal screen. */
export function terminalWriteHasOutput(event: Pick<SessionEvent, "direction" | "text">, rawBytes?: Uint8Array): boolean {
  return event.direction !== "outbound" && (rawBytes !== undefined ? rawBytes.byteLength > 0 : Boolean(event.text));
}

/** A complete readline erase echo (EL, optionally preceded by BS) cannot
 * redraw other rows after the parser reaches EL. Never infer this from a
 * decoded preview when xterm actually consumes the raw transport bytes. */
export function isStandaloneLineErase(text: string, bytes?: Uint8Array): boolean {
  if (!bytes) return text === "\x1b[K" || text === "\b\x1b[K";
  const start = bytes.length - 3;
  return (start === 0 || (start === 1 && bytes[0] === 8))
    && bytes[start] === 27 && bytes[start + 1] === 91 && bytes[start + 2] === 75;
}

/** True when the payload may move the cursor across rows or emit escape controls. */
export function terminalPayloadMayMoveRows(text: string, bytes?: Uint8Array): boolean {
  if (bytes) {
    for (let index = 0; index < bytes.length; index += 1) {
      const byte = bytes[index];
      if (byte === 0x0a || byte === 0x0b || byte === 0x0c || byte === 0x0d || byte === 0x1b) return true;
    }
    return false;
  }
  return /[\r\n\f\v\x1b]/u.test(text);
}

/** True when the payload can jump backward on the current row and overwrite it in place. */
export function terminalPayloadMayOverwriteCursorLine(text: string, bytes?: Uint8Array): boolean {
  if (bytes) {
    for (let index = 0; index < bytes.length; index += 1) {
      const byte = bytes[index];
      if (byte === 0x0d || byte === 0x1b) return true;
    }
    return false;
  }
  return /[\r\x1b]/u.test(text);
}

/** Cursor movement via C0 controls leaves the timestamps of existing text intact. */
export function isTerminalCursorMotionOnly(text: string, bytes?: Uint8Array): boolean {
  if (!bytes) return /^[\b\t\n\v\f\r]+$/u.test(text);
  for (const byte of bytes) {
    if (byte < 0x08 || byte > 0x0d) return false;
  }
  return bytes.length > 0;
}
