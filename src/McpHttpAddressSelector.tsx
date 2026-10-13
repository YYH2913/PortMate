import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { invokeBackend } from "./api";
import { t, localizeDiagnostic } from "./i18n";
import { initialMcpHttpAddressSelection, MCP_HTTP_ADDRESS_SELECTION_KEY, MCP_HTTP_CUSTOM_ADDRESS,
  networkAddressOptionValue, normalizeLocalNetworkAddresses, normalizeMcpHttpAddressSelection,
  rememberedNetworkAddress } from "./mcp-http-address-selection";
import type { McpHttpAddressSelection } from "./mcp-http-address-selection";
import type { McpHttpController } from "./use-mcp-http-controller";
import type { LocalNetworkAddress } from "./types";

function readSelection(): McpHttpAddressSelection | null {
  try { return normalizeMcpHttpAddressSelection(JSON.parse(localStorage.getItem(MCP_HTTP_ADDRESS_SELECTION_KEY) ?? "null")); }
  catch { return null; }
}
function remember(selection: McpHttpAddressSelection) {
  try { localStorage.setItem(MCP_HTTP_ADDRESS_SELECTION_KEY, JSON.stringify(selection)); } catch { /* In-memory choice remains usable. */ }
}

export default function McpHttpAddressSelector({ http, locked }: { http: McpHttpController; locked: boolean }) {
  const [addresses, setAddresses] = useState<LocalNetworkAddress[]>([]);
  const [selection, setSelection] = useState(readSelection);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const [initialized, setInitialized] = useState(false);
  const resetRevision = useRef(http.draftResetRevision);
  const customHost = useRef(selection?.mode === "custom" ? selection.address : http.settings.clientHost);
  useEffect(() => {
    let disposed = false;
    setLoading(true);
    void invokeBackend<unknown>("list_local_network_addresses", {}).then(value => {
      if (!disposed) { setAddresses(normalizeLocalNetworkAddresses(value)); setError(""); }
    }).catch(cause => {
      if (!disposed) { setAddresses([]); setError(String(cause)); }
    }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [revision]);

  useEffect(() => {
    if (loading || http.loading) return;
    if (resetRevision.current !== http.draftResetRevision) {
      resetRevision.current = http.draftResetRevision;
      const next = initialMcpHttpAddressSelection(addresses, null, http.settings.clientHost);
      customHost.current = http.settings.clientHost;
      setSelection(next); remember(next); setInitialized(true);
      return;
    }
    if (!initialized) {
      if (locked) return;
      const next = initialMcpHttpAddressSelection(addresses, selection, http.settings.clientHost);
      setInitialized(true);
      setSelection(next); remember(next);
      return;
    }
    // Restoring an adapter only edits the stopped service's draft. A running
    // listener and existing saved configuration never change in the background.
    if (locked || !selection) return;
    const host = selection.mode === "custom" ? selection.address
      : rememberedNetworkAddress(addresses, selection)?.address;
    if (host !== undefined && host !== http.settings.clientHost) http.update({ clientHost: host });
  }, [loading, http.loading, locked, http.draftResetRevision, http.settings.clientHost, addresses, selection, initialized]);

  const effective = initialized && selection && !http.running ? selection : initialMcpHttpAddressSelection(addresses, null, http.settings.clientHost);
  const selectedAddress = rememberedNetworkAddress(addresses, effective);
  const unavailable = effective.mode === "interface" && !selectedAddress;
  const value = effective.mode === "custom" ? MCP_HTTP_CUSTOM_ADDRESS
    : selectedAddress ? networkAddressOptionValue(selectedAddress) : "unavailable";
  useEffect(() => {
    http.setClientAddressAvailable(!unavailable);
    return () => { http.setClientAddressAvailable(true); };
  }, [unavailable, http.setClientAddressAvailable]);

  function select(value: string) {
    if (locked) return;
    setInitialized(true);
    if (effective.mode === "custom") customHost.current = http.settings.clientHost;
    if (value === MCP_HTTP_CUSTOM_ADDRESS) {
      const next: McpHttpAddressSelection = { mode: "custom", address: customHost.current };
      setSelection(next); remember(next); http.update({ clientHost: next.address });
    } else {
      const entry = addresses.find(entry => networkAddressOptionValue(entry) === value);
      if (!entry) return;
      const next: McpHttpAddressSelection = { mode: "interface", interfaceId: entry.interfaceId, family: entry.family, address: entry.address };
      setSelection(next); remember(next); http.update({ clientHost: entry.address });
    }
  }

  return <div className="mcp-client-address-selector">
    <label><span>{t("access-address-portmate-host")}</span>
      <div className="mcp-client-address-options">
        <select aria-label={t("mcp-http-address-source")} value={value} disabled={locked || loading} onChange={event => select(event.target.value)}>
          {unavailable && effective.mode === "interface" ? <option value="unavailable" disabled>{t("mcp-http-adapter-unavailable", [effective.interfaceId])}</option> : null}
          {addresses.map(entry => <option key={networkAddressOptionValue(entry)} value={networkAddressOptionValue(entry)}>
            {entry.interfaceName} · {entry.address} ({entry.family === "ipv4" ? "IPv4" : "IPv6"})
          </option>)}
          <option value={MCP_HTTP_CUSTOM_ADDRESS}>{t("mcp-http-custom-address")}</option>
        </select>
        <button type="button" className="icon-button" aria-label={t("refresh-local-network-addresses")} title={t("refresh-local-network-addresses")} disabled={locked || loading} onClick={() => setRevision(value => value + 1)}><RefreshCw size={14} /></button>
      </div>
    </label>
    {effective.mode === "custom" ? <input aria-label={t("mcp-http-client-address")} value={http.settings.clientHost} maxLength={253} spellCheck={false} disabled={locked} onChange={event => {
      const next: McpHttpAddressSelection = { mode: "custom", address: event.target.value };
      customHost.current = next.address; setSelection(next); remember(next); http.update({ clientHost: next.address });
    }} /> : null}
    {unavailable ? <p className="mcp-inline-warning">{t("mcp-http-remembered-adapter-unavailable")}</p> : null}
    {error ? <p className="mcp-inline-warning">{t("mcp-http-network-addresses-failed", [localizeDiagnostic(error)])}</p> : null}
    <p className="mcp-help">{t("mcp-http-address-selection-help")}</p>
  </div>;
}
