export type SerialColumnObservation = { columns: number; observations: number };

type ParserState = "ground" | "escape" | "csi" | "string" | "string-escape" | "escape-intermediate";
type RedrawState = "idle" | "line-start" | "up" | "right" | "erase";
const EVIDENCE_WINDOW_MS = 120_000;
const SEQUENCE_IDLE_MS = 2_000;
const MAX_COLUMNS = 1024;

/**
 * A heuristic, NOT a terminal-size protocol. Recognizes Readline-style
 * CR [EL] CUU(1) CUF(width-1) [BS] EL redraws, only in bracketed-paste
 * shell mode on the normal screen. Two matching observations are required.
 * No text is retained, no probe is sent and no terminal state is modified.
 */
export class SerialColumnDetector {
  private parser: ParserState = "ground";
  private csi = "";
  private stringIsOsc = false;
  private bracketedPaste = false;
  private alternateModes = new Set<number>();
  private redraw: RedrawState = "idle";
  private right = 0;
  private lastAt: number | null = null;
  private candidate: (SerialColumnObservation & { at: number }) | null = null;

  reset(): void {
    this.parser = "ground";
    this.csi = "";
    this.stringIsOsc = false;
    this.bracketedPaste = false;
    this.alternateModes.clear();
    this.breakRedraw();
    this.candidate = null;
    this.lastAt = null;
  }

  observe(data: Uint8Array | string, now: number): SerialColumnObservation | null {
    if (this.lastAt !== null && (now < this.lastAt || now - this.lastAt > SEQUENCE_IDLE_MS)) {
      this.breakRedraw();
    }
    if (this.candidate && (now < this.candidate.at || now - this.candidate.at > EVIDENCE_WINDOW_MS)) {
      this.candidate = null;
    }
    this.lastAt = now;
    for (let index = 0; index < data.length; index += 1) {
      this.consume(typeof data === "string" ? data.charCodeAt(index) : data[index], now);
    }
    return this.bracketedPaste && !this.alternateModes.size && this.candidate && this.candidate.observations >= 2
      ? { columns: this.candidate.columns, observations: this.candidate.observations }
      : null;
  }

  private breakRedraw(): void {
    this.redraw = "idle";
    this.right = 0;
  }

  private consume(byte: number, now: number): void {
    // Never inspect escape-looking data inside control strings. Retain no
    // payload and bound memory even for unterminated strings/oversized CSI.
    if (this.parser === "string" || this.parser === "string-escape") {
      if (byte === 0x18 || byte === 0x1a || (this.stringIsOsc && byte === 7)
        || (this.parser === "string-escape" && byte === 0x5c)) this.parser = "ground";
      else this.parser = byte === 0x1b ? "string-escape" : "string";
      return;
    }
    if (byte === 0x18 || byte === 0x1a) {
      this.parser = "ground";
      this.csi = "";
      this.breakRedraw();
      return;
    }
    if (byte === 0x1b) {
      this.parser = "escape";
      this.csi = "";
      return;
    }
    if (this.parser === "escape") {
      if (byte === 0x5b) this.parser = "csi";
      else if ([0x5d, 0x50, 0x5f, 0x5e, 0x58].includes(byte)) {
        this.stringIsOsc = byte === 0x5d;
        this.parser = "string";
        this.breakRedraw();
      } else if (byte >= 0x20 && byte <= 0x2f) {
        this.parser = "escape-intermediate";
        this.breakRedraw();
      } else {
        this.parser = "ground";
        if (byte === 0x63) this.reset();
        else this.breakRedraw();
      }
      return;
    }
    if (this.parser === "escape-intermediate") {
      if (byte >= 0x30) this.parser = "ground";
      return;
    }
    if (this.parser === "csi") {
      if (byte >= 0x40 && byte <= 0x7e) {
        this.control(this.csi, String.fromCharCode(byte), now);
        this.parser = "ground";
        this.csi = "";
      } else if (byte >= 0x20 && byte <= 0x3f && this.csi.length < 48) {
        this.csi += String.fromCharCode(byte);
      } else {
        this.parser = "ground";
        this.csi = "";
        this.breakRedraw();
      }
      return;
    }
    if (this.bracketedPaste && !this.alternateModes.size && byte === 13) {
      this.redraw = "line-start";
      this.right = 0;
    } else if (byte === 8 && this.redraw === "right") {
      this.redraw = "erase";
    } else this.breakRedraw();
  }

  private control(params: string, final: string, now: number): void {
    if ((final === "h" || final === "l") && /^\?\d+(;\d+)*$/.test(params)) {
      for (const mode of params.slice(1).split(";").map(Number)) {
        if (mode === 2004) {
          const nextBracketedPaste = final === "h";
          if (nextBracketedPaste !== this.bracketedPaste) {
            this.candidate = null;
            this.breakRedraw();
          }
          this.bracketedPaste = nextBracketedPaste;
        }
        if ([47, 1047, 1049].includes(mode)) {
          if (final === "h") this.alternateModes.add(mode);
          else this.alternateModes.delete(mode);
          this.candidate = null;
        }
      }
      this.breakRedraw();
      return;
    }
    if (params === "!" && final === "p") { this.reset(); return; }
    if (!this.bracketedPaste || this.alternateModes.size || !/^\d*$/.test(params)) {
      this.breakRedraw();
      return;
    }
    const value = Number(params);
    if (final === "A" && value <= 1 && this.redraw === "line-start") {
      this.redraw = "up";
    } else if (final === "C" && (this.redraw === "up" || this.redraw === "right")) {
      this.right += value || 1;
      this.redraw = "right";
      if (this.right >= MAX_COLUMNS) this.breakRedraw();
    } else if (final === "K" && value === 0) {
      if (this.redraw === "line-start") return;
      if ((this.redraw === "right" || this.redraw === "erase") && this.right >= 19) {
        const columns = this.right + 1;
        this.candidate = {
          columns,
          observations: this.candidate?.columns === columns ? Math.min(3, this.candidate.observations + 1) : 1,
          at: now,
        };
      }
      this.breakRedraw();
    } else this.breakRedraw();
  }
}
