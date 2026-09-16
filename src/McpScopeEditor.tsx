import { t, useLocale } from "./i18n";
import type { McpScope } from "./types";

const groups: { label: string; scopes: { id: McpScope; label: string; detail: string }[] }[] = [
  { label: "read-permissions", scopes: [
    { id: "read-sessions", label: "session-status", detail: "view-sessions-within-the-granted-scope" },
    { id: "read-logs", label: "terminal-logs", detail: "read-and-search-session-logs" },
    { id: "read-transfers", label: "transfer-status", detail: "view-file-transfer-progress" },
    { id: "read-tunnels", label: "tunnel-status", detail: "view-visible-forwards-and-proxies" },
    { id: "read-scripts", label: "script-tools", detail: "view-scripts-allowed-for-this-client" },
    { id: "read-mcp", label: "bridge-status", detail: "view-mcp-runtime-and-configuration" },
  ] },
  { label: "action-permissions", scopes: [
    { id: "write-input", label: "send-commands", detail: "send-input-to-authorized-sessions" },
    { id: "transfer", label: "file-transfers", detail: "manage-transfers-including-reading-their-status" },
    { id: "host-files", label: "host-file-access", detail: "high-risk-permits-access-to-portmate-host-paths" },
    { id: "tunnel", label: "tunnels-and-proxies", detail: "manage-forwards-and-proxies-including-reading-tunnel-status" },
    { id: "manage-sessions", label: "session-management", detail: "manage-connections-within-the-granted-session-scope" },
    { id: "run-scripts", label: "run-host-scripts", detail: "run-only-scripts-explicitly-exposed-to-this-client" },
    { id: "manage-mcp", label: "bridge-management", detail: "high-risk-permits-managing-bridge-runtime-state" },
  ] },
];
export const allMcpScopes = groups.flatMap(group => group.scopes.map(scope => scope.id));

export default function McpScopeEditor({ scopes, disabled, onToggle }: {
  scopes: readonly McpScope[];
  disabled: boolean;
  onToggle: (scope: McpScope) => void;
}) {
  useLocale();
  return <div className="mcp-scope-groups">{groups.map(group => (
    <fieldset key={group.label} className="mcp-check-grid">
      <legend>{t(group.label)}</legend>
      {group.scopes.map(scope => <label key={scope.id} title={t(scope.detail)}>
        <input type="checkbox" aria-label={scope.id} value={scope.id} disabled={disabled} checked={scopes.includes(scope.id)} onChange={() => onToggle(scope.id)} />
        <span>{t(scope.label)}<code>{scope.id}</code></span>
      </label>)}
    </fieldset>
  ))}</div>;
}
