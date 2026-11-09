import type { SessionEvent } from "./types";

/** Outbound capture/empty acknowledgements never change the terminal screen. */
export function terminalWriteHasOutput(event: Pick<SessionEvent, "direction" | "text">, rawBytes?: Uint8Array): boolean {
  return event.direction !== "outbound" && (rawBytes !== undefined ? rawBytes.byteLength > 0 : Boolean(event.text));
}
