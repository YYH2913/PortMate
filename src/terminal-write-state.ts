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
