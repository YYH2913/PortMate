import {
  emptyTerminalCommandLineTracker,
  trackTerminalCommandInput,
} from "./terminal-command-line-tracker";
import type {
  TerminalCommandLineTrackerState,
} from "./terminal-command-line-tracker";

/** Owns semantic terminal input state independently from the renderer. */
export class TerminalInputController {
  private state: TerminalCommandLineTrackerState = emptyTerminalCommandLineTracker();

  reset(synchronized = true): void {
    this.state = { ...emptyTerminalCommandLineTracker(), synchronized };
  }

  invalidate(): void {
    this.reset(false);
  }

  track(text: string, sensitive = false): string[] {
    const result = trackTerminalCommandInput(this.state, text, sensitive);
    this.state = result.state;
    return result.submitted;
  }

  snapshot(): TerminalCommandLineTrackerState {
    return this.state;
  }
}
