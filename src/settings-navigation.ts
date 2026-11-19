import { workspaceHotkeyCommands } from "./workspace-hotkeys";
import { t } from "./i18n";

// Only static labels enter this index, never profile names, hosts or credentials.
const sharedNetwork = ["host-h", "port-p", "auto-reconnect", "reconnect-delay-ms"];
const ssh = [...sharedNetwork, "username-u", "alias-a", "ui-jump-host", "ssh-keepalive", "ui-tcp-keepalive", "reconnect-ignore-host-key-changes"];
const fields: Record<string, readonly string[]> = {
  session: ["name-n", "group-g", "tags-l"],
  "terminal-appearance": ["theme-m", "font-f", "font-size-z", "background-opacity-o"],
  "terminal-behavior": ["terminal-t", "rows-r", "columns-c", "scrollback-s"],
  logs: ["enabled-e", "raw-not-redacted-r", "ui-text-t", "sensitive-fields-s", "path-p", "retention-days-d"],
  transfers: ["SFTP:", "SCP:", "TFTP:", "XModem:", "YModem:", "ZModem:", "rate-limit-b-s", "default-directory-d"],
  Shell: ["program-p", "arguments-a", "directory-w", "shell-elevation"], triggers: ["add-trigger"],
  SSH: ssh, Tmux: ssh, Tcp: [...sharedNetwork, "ui-tcp-keepalive"], Telnet: [...sharedNetwork, "ui-tcp-keepalive", "BINARY:", "NAWS:"],
  serial: ["serial-port-s", "baud-rate-b", "data-bits-d", "stop-bits-s", "parity-p", "flow-control-f", "auto-reconnect", "reconnect-delay-ms", "receive-idle-timeout"],
  proxy: ["enable-proxy", "protocol", "proxy-host", "proxy-port"],
  verification: ["ui-hostkey", "rotation-r", "verify-ip-i", "trust-scope-s", "scan-2", "health"], "ssh-agent": ["ui-agent-a", "ui-forward-f", "ui-offer-o"],
  password: ["password-reference", "passphrase-reference"], "public-key": ["remember-successful-method-r", "public-key-k-client-identity"],
  application: ["interface-language", "no-sessions-n", "previous-sessions-l", "specific-session-or-session-group-s", "default-directory"],
  appearance: ["multicolor-commands-and-output"], "keyboard-shortcuts": workspaceHotkeyCommands.map(command => command.label),
  security: ["lock-screen-after-idle-time", "lock-timeout-minutes", "lock-screen-on-startup"],
  mouse: ["allow-terminal-applications-to-receive-mouse-events", "copy-on-selection"],
  autocomplete: ["enable-autocomplete-a", "onekey-terminal-prompt-completion-k", "command-names-n", "command-options-o", "subcommands-and-arguments-p", "command-history-h", "quick-commands-q", "start-autocomplete-after-s", "completion-list-height-h", "preview-best-match-p"],
  "command-history-settings": ["history-retention-days-d", "history-size-h", "save-command-history-to-disk-s", "saved-command-history"],
  "synchronized-input": ["newline-policy", "delay-between-targets-ms", "batch-prefix", "batch-suffix"],
};
const aliases: Record<string, string[]> = {
  "multicolor-commands-and-output": ["theme-m", "terminal-color-preview"], "theme-m": ["terminal-color-preview", "multicolor-commands-and-output"],
  "font-size-z": ["terminal-font-size"], "terminal-t": ["TERM"], "baud-rate-b": ["baud-rate"],
};
export type FieldLabel = { label: string; labelKey?: never } | { label?: never; labelKey: string };
export type SettingsHit = { page: string; field?: string };
export const settingsLabel = (key: string) => t(key).replace(/\s*[:：]?\s*\([A-Za-z]\)\s*$/, "").replace(/[:：]\s*$/, "").trim();
export function findSettings(pages: readonly string[], query: string): SettingsHit[] {
  const words = query.trim().slice(0, 160).toLocaleLowerCase().split(/\s+/u);
  if (!words[0]) return [];
  const hits: SettingsHit[] = [];
  const sshTransfers = pages.includes("SSH") || pages.includes("Tmux");
  for (const page of pages) {
    const pageLabel = settingsLabel(page).toLocaleLowerCase();
    if (words.every(word => pageLabel.includes(word))) hits.push({ page });
    for (const field of fields[page] ?? []) {
      if (page === "transfers" && !sshTransfers && (field === "SFTP:" || field === "SCP:")) continue;
      const label = [settingsLabel(field), field.replaceAll("-", " "), ...(aliases[field] ?? []).map(settingsLabel)].join(" ").toLocaleLowerCase();
      if (words.some(word => label.includes(word)) && words.every(word => label.includes(word) || pageLabel.includes(word))) hits.push({ page, field });
    }
  }
  return hits;
}
