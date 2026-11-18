import type { TerminalSemanticTokenKind } from "./terminal-semantic-highlighting";
import type { TerminalThemeId } from "./terminal-theme";

// Application-owned text only. Keep ANSI 0–255 and truecolor entirely in xterm's theme.
// Operators and times recede; red/amber/green signal errors, warnings and success.
type SemanticPalette = Readonly<Record<TerminalSemanticTokenKind, string>>;
export const terminalSemanticPalettes: Readonly<Record<TerminalThemeId, SemanticPalette>> = {
  "portmate-dark": {
    command: "#91dccc", option: "#a3baff", string: "#e3c598", path: "#9cccf3", address: "#74d5e0",
    number: "#dba8cb", variable: "#c3a7ec", operator: "#a2afbf", timestamp: "#8b9aae", permission: "#abbba6",
    success: "#94d88b", warning: "#ffb75e", error: "#ff8884", info: "#85aef2",
  },
  graphite: {
    command: "#aed4bc", option: "#acb9da", string: "#d7c397", path: "#a0c3d5", address: "#86c8c5",
    number: "#dbb3a8", variable: "#cbb0d9", operator: "#afb1b6", timestamp: "#90959d", permission: "#abb7a3",
    success: "#acd890", warning: "#eabe73", error: "#f29798", info: "#98b7df",
  },
  "solarized-dark": {
    command: "#b3c678", option: "#97b6e9", string: "#d5bd86", path: "#8bc7e4", address: "#79c8c0",
    number: "#dfad9e", variable: "#c0aad9", operator: "#a1b2b3", timestamp: "#8ba3a5", permission: "#aab999",
    success: "#a7cf8c", warning: "#e6b75c", error: "#ff9183", info: "#85b9de",
  },
  "portmate-light": {
    command: "#116c60", option: "#425ba5", string: "#775720", path: "#2e608a", address: "#096779",
    number: "#90477a", variable: "#734f99", operator: "#4f5e70", timestamp: "#5a6879", permission: "#53674d",
    success: "#377037", warning: "#905509", error: "#b13942", info: "#2d63a0",
  },
};
