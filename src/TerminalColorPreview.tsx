import { t } from "./i18n";
import { normalizeTerminalProfileSettings } from "./terminal-settings-state";
import { terminalSemanticTokens } from "./terminal-semantic-highlighting";
import type { TerminalSemanticTokenKind } from "./terminal-semantic-highlighting";
import { terminalSemanticPalettes } from "./terminal-semantic-palette";
import { normalizeTerminalTheme, terminalTheme } from "./terminal-theme";
import type { SessionProfile } from "./types";

// Use the same code-point ranges as the live terminal, without opening a PTY.
const samples = [
  "edge:~$ tail -f /var/log/link.log",
  "14:08:21  WARN   peer=192.0.2.24",
  "14:08:22  ERROR  HTTP/2 503",
  "14:08:23  OK     latency=12ms",
].map(text => {
  const chars = Array.from(text);
  const runs: Array<{ text: string; role?: TerminalSemanticTokenKind }> = [];
  let cursor = 0;
  for (const { start, end, kind } of terminalSemanticTokens(text)) {
    if (cursor < start) runs.push({ text: chars.slice(cursor, start).join("") });
    runs.push({ text: chars.slice(start, end).join(""), role: kind });
    cursor = end;
  }
  if (cursor < chars.length) runs.push({ text: chars.slice(cursor).join("") });
  return { text, runs };
});

const ansi = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
] as const;

export default function TerminalColorPreview({ settings, semantic }: { settings: SessionProfile["terminal"]; semantic: boolean }) {
  const normalized = normalizeTerminalProfileSettings(settings);
  const id = normalizeTerminalTheme(normalized.theme);
  const theme = terminalTheme(id, normalized.backgroundOpacity);
  const palette = terminalSemanticPalettes[id];
  return (
    <figure className="terminal-color-preview" aria-label={t("terminal-color-preview")}>
      <figcaption><span>{t("terminal-color-preview")}</span><span aria-hidden="true">ANSI 16</span></figcaption>
      <div className="terminal-color-sample" dir="ltr" style={{
        backgroundColor: theme.background, color: theme.foreground, fontFamily: normalized.fontFamily, fontSize: normalized.fontSize,
      }}>
        <div className="terminal-color-swatch-strip" aria-hidden="true">
          {ansi.map(key => <i key={key} style={{ backgroundColor: theme[key] }} />)}
        </div>
        {samples.map(({ text, runs }) => <div key={text}>
          {semantic ? runs.map(({ text: part, role }, i) => role ? <span key={i} style={{ color: palette[role] }}>{part}</span> : part) : text}
        </div>)}
      </div>
    </figure>
  );
}
