export const terminalKeyModes = ["remote", "local", "normal", "command"] as const;

export type TerminalKeyMode = typeof terminalKeyModes[number];

export type TerminalLocalCommand =
  | "move-left"
  | "move-right"
  | "move-up"
  | "move-down"
  | "line-start"
  | "line-end"
  | "document-start"
  | "document-end"
  | "word-forward"
  | "word-backward"
  | "word-end"
  | "page-up"
  | "page-down"
  | "half-page-up"
  | "half-page-down"
  | "scroll-line-up"
  | "scroll-line-down"
  | "open-search"
  | "find-next"
  | "find-previous"
  | "toggle-selection"
  | "toggle-line-selection"
  | "yank"
  | "clear-selection";

export type TerminalKeySequenceState = {
  count: string;
  prefix: "" | "g";
};

export type TerminalKeyModeResolution = {
  handled: boolean;
  state: TerminalKeySequenceState;
  nextMode?: TerminalKeyMode;
  command?: TerminalLocalCommand;
  count: number;
};

export type TerminalModeKeyEvent = Pick<
  KeyboardEvent,
  "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "isComposing"
>;

export const emptyTerminalKeySequenceState = (): TerminalKeySequenceState => ({ count: "", prefix: "" });

const modeLabels: Record<TerminalKeyMode, string> = {
  remote: "Insert 模式",
  local: "本地模式",
  normal: "本地编辑",
  command: "Normal 模式",
};

export function normalizeTerminalKeyMode(value: unknown): TerminalKeyMode {
  return typeof value === "string" && terminalKeyModes.includes(value as TerminalKeyMode)
    ? value as TerminalKeyMode
    : "remote";
}

export function terminalKeyModeLabel(mode: TerminalKeyMode): string {
  return modeLabels[mode];
}

export function toggleTerminalRemoteLocalMode(mode: TerminalKeyMode): TerminalKeyMode {
  return mode === "remote" ? "local" : "remote";
}

export function toggleTerminalInsertNormalMode(mode: TerminalKeyMode): TerminalKeyMode {
  return mode === "remote" ? "command" : "remote";
}

export function terminalKeyModeCursorStyle(mode: TerminalKeyMode): "bar" | "block" | "underline" {
  if (mode === "remote") return "bar";
  if (mode === "normal") return "underline";
  return "block";
}

export function resolveTerminalKeyModeEvent(
  mode: TerminalKeyMode,
  event: TerminalModeKeyEvent,
  state = emptyTerminalKeySequenceState(),
): TerminalKeyModeResolution {
  const empty = emptyTerminalKeySequenceState();
  if (event.isComposing) return { handled: mode !== "remote", state: empty, count: 1 };

  if (mode === "remote" && event.key === "Escape" && !hasCommandModifiers(event) && !event.shiftKey) {
    return { handled: true, state: empty, nextMode: "command", count: 1 };
  }

  if (isCtrlEnter(event)) {
    const nextMode = mode === "remote"
      ? "local"
      : mode === "local"
        ? "remote"
        : mode === "normal"
          ? "command"
          : "remote";
    return { handled: true, state: empty, nextMode, count: 1 };
  }

  if (mode === "remote") return { handled: false, state: empty, count: 1 };
  if (mode === "normal") {
    const nextMode = event.key === "Escape" ? "command" : undefined;
    return { handled: true, state: empty, nextMode, count: 1 };
  }

  if (!hasCommandModifiers(event) && event.key === "i") {
    return {
      handled: true,
      state: empty,
      nextMode: "remote",
      count: 1,
    };
  }

  if (event.key === "Escape") {
    return { handled: true, state: empty, command: "clear-selection", count: 1 };
  }

  if (state.prefix === "g") {
    const count = parsedCount(state.count);
    const command = resolveGCommand(event);
    return { handled: true, state: empty, command, count };
  }

  if (!hasCommandModifiers(event) && event.key === "g") {
    return { handled: true, state: { ...state, prefix: "g" }, count: parsedCount(state.count) };
  }

  if (!hasCommandModifiers(event) && /^\d$/.test(event.key)) {
    if (event.key !== "0" || state.count) {
      const countText = `${state.count}${event.key}`.slice(0, 6);
      return {
        handled: true,
        state: { count: countText, prefix: "" },
        count: parsedCount(countText),
      };
    }
  }

  const count = parsedCount(state.count);
  const command = resolveLocalCommand(mode, event);
  return { handled: true, state: empty, command, count };
}

function resolveGCommand(event: TerminalModeKeyEvent): TerminalLocalCommand | undefined {
  if (hasCommandModifiers(event)) return undefined;
  if (event.key === "g") return "document-start";
  if (event.key === "e") return "word-backward";
  if (event.key === "0" || event.key === "^") return "line-start";
  if (event.key === "$" || event.key === "_") return "line-end";
  return undefined;
}

function resolveLocalCommand(
  mode: "local" | "command",
  event: TerminalModeKeyEvent,
): TerminalLocalCommand | undefined {
  if (event.ctrlKey && !event.metaKey && !event.altKey) {
    const key = event.key.toLowerCase();
    if (key === "d") return "half-page-down";
    if (key === "u") return "half-page-up";
    if (key === "e") return "scroll-line-down";
    if (key === "y") return "scroll-line-up";
    if (key === "b") return "page-up";
    if (key === "f" && mode === "command") return "page-down";
  }
  if (event.ctrlKey || event.metaKey || event.altKey) return undefined;

  if (event.key === "h" || event.key === "ArrowLeft" || event.key === "Backspace") return "move-left";
  if (event.key === "l" || event.key === "ArrowRight") return "move-right";
  if (event.key === "k" || event.key === "ArrowUp") return "move-up";
  if (event.key === "j" || event.key === "ArrowDown" || event.key === "Enter") return "move-down";
  if (event.key === "0" || event.key === "^" || event.key === "Home") return "line-start";
  if (event.key === "$" || event.key === "End") return "line-end";
  if (event.key === "G") return "document-end";
  if (event.key === "w") return "word-forward";
  if (event.key === "b") return "word-backward";
  if (event.key === "e") return "word-end";
  if (event.key === "PageUp") return "page-up";
  if (event.key === "PageDown" || event.key === " ") return "page-down";
  if (event.key === "/" || event.key === "?") return "open-search";
  if (event.key === "n") return "find-next";
  if (event.key === "N") return "find-previous";
  if (event.key === "v") return "toggle-selection";
  if (event.key === "V") return "toggle-line-selection";
  if (event.key === "y") return "yank";
  return undefined;
}

function parsedCount(value: string): number {
  const count = Number.parseInt(value, 10);
  return Number.isFinite(count) && count > 0 ? Math.min(count, 100_000) : 1;
}

function isCtrlEnter(event: TerminalModeKeyEvent): boolean {
  return event.key === "Enter" && (event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey;
}

function hasCommandModifiers(event: TerminalModeKeyEvent): boolean {
  return event.ctrlKey || event.metaKey || event.altKey;
}
