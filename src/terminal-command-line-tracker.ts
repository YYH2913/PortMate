import { MAX_COMMAND_HISTORY_COMMAND_CHARACTERS } from "./command-history-state";

export type TerminalCommandLineTrackerState = {
  line: string;
  cursor: number;
  synchronized: boolean;
  pendingEscape: string;
  bracketedPaste: boolean;
};

export const emptyTerminalCommandLineTracker = (): TerminalCommandLineTrackerState => ({
  line: "", cursor: 0, synchronized: true, pendingEscape: "", bracketedPaste: false,
});

/** Track only edits whose effect is known; unknown remote edits suppress that line. */
export function trackTerminalCommandInput(
  current: TerminalCommandLineTrackerState,
  input: string,
  sensitive = false,
): { state: TerminalCommandLineTrackerState; submitted: string[] } {
  if (sensitive) return {
    state: { ...emptyTerminalCommandLineTracker(), synchronized: /[\r\n\u0003\u0015]$/.test(input) },
    submitted: [],
  };
  let state = { ...current, pendingEscape: "" };
  const submitted: string[] = [];
  const inputChars = Array.from(current.pendingEscape + input);
  let chars = Array.from(state.line);
  for (let index = 0; index < inputChars.length; index += 1) {
    const char = inputChars[index];
    if (char === "\u001b") {
      const tail = inputChars.slice(index + 1).join("");
      const sequence = tail.match(/^\[([0-9;]*)([@-~])/)
        ?? tail.match(/^O()([A-DHF])/);
      if (!sequence) {
        if ((tail === "" || tail === "O" || /^\[[0-9;]*$/.test(tail)) && tail.length < 32) {
          state.pendingEscape = "\u001b" + tail;
          break;
        }
        state.synchronized = false;
        continue;
      }
      index += sequence[0].length;
      const [, parameters, final] = sequence;
      if (final === "~" && parameters === "200") { state.bracketedPaste = true; continue; }
      if (final === "~" && parameters === "201") { state.bracketedPaste = false; continue; }
      if (!state.synchronized) continue;
      const amount = Math.max(1, Number(parameters) || 1);
      if (state.bracketedPaste || parameters.includes(";")) state.synchronized = false;
      else if (final === "D") state.cursor = Math.max(0, state.cursor - amount);
      else if (final === "C") state.cursor = Math.min(chars.length, state.cursor + amount);
      else if (final === "H" || (final === "~" && ["1", "7"].includes(parameters))) state.cursor = 0;
      else if (final === "F" || (final === "~" && ["4", "8"].includes(parameters))) state.cursor = chars.length;
      else if (final === "~" && parameters === "3") chars.splice(state.cursor, 1);
      else state.synchronized = false;
      continue;
    }
    if (!state.bracketedPaste && (char === "\r" || char === "\n")) {
      if (state.synchronized && chars.join("").trim()) submitted.push(chars.join(""));
      state = emptyTerminalCommandLineTracker();
      chars = [];
      continue;
    }
    if (!state.bracketedPaste && char === "\u0003") {
      state = emptyTerminalCommandLineTracker();
      chars = [];
      continue;
    }
    if (!state.synchronized) continue;
    if (!state.bracketedPaste) {
      if (char === "\b" || char === "\u007f") {
        if (state.cursor > 0) chars.splice(--state.cursor, 1);
        continue;
      }
      if (char === "\u0001") { state.cursor = 0; continue; }
      if (char === "\u0005") { state.cursor = chars.length; continue; }
      if (char === "\u0002") { state.cursor = Math.max(0, state.cursor - 1); continue; }
      if (char === "\u0006") { state.cursor = Math.min(chars.length, state.cursor + 1); continue; }
      if (char === "\u0004") { chars.splice(state.cursor, 1); continue; }
      if (char === "\u000b") { chars.splice(state.cursor); continue; }
      if (char === "\u0015") { chars.splice(0, state.cursor); state.cursor = 0; continue; }
      if (char === "\u0017") {
        const prefix = chars.slice(0, state.cursor).join("");
        const remaining = Array.from(prefix.replace(/\S+\s*$/, "")).length;
        chars.splice(remaining, state.cursor - remaining);
        state.cursor = remaining;
        continue;
      }
    }
    if ((char >= " " && !/[\u007f-\u009f]/.test(char)) || (state.bracketedPaste && /[\r\n\t]/.test(char))) {
      if (chars.length >= MAX_COMMAND_HISTORY_COMMAND_CHARACTERS) state.synchronized = false;
      else chars.splice(state.cursor++, 0, char === "\r" ? "\n" : char);
    } else state.synchronized = false;
  }
  state.line = state.synchronized ? chars.join("") : "";
  if (!state.synchronized) state.cursor = 0;
  return { state, submitted };
}
