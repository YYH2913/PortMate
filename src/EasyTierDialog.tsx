import { useEffect, useRef, useState } from "react";
import { Download, ExternalLink, Eye, EyeOff, Network, Play, Plus, RefreshCw, Save, Settings2, ShieldCheck, Square, Trash2, X } from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { t, useLocale } from "./i18n";
import { defaultEasyTierConfig, easyTierConfigValid, easyTierEndpoints, easyTierSecretValid } from "./easytier-state";
import type { EasyTierConfig, EasyTierView, NpcapStatus } from "./easytier-state";
import { openTerminalWebLink } from "./terminal-web-link";
import "./easytier.css";

export default function EasyTierDialog({ onClose }: { onClose: () => void }) {
  useLocale();
  const desktop = isBackendAvailable();
  const [config, setConfig] = useState(defaultEasyTierConfig);
  const [view, setView] = useState<EasyTierView | null>(null);
  const [secret, setSecret] = useState("");
  const [peers, setPeers] = useState("");
  const [listeners, setListeners] = useState(defaultEasyTierConfig().listeners.join("\n"));
  const [page, setPage] = useState<"config" | "running" | "component">("config");
  const [method, setMethod] = useState<"public" | "manual" | "standalone">("standalone");
  const [showSecret, setShowSecret] = useState(false);
  const [logsOpen, setLogsOpen] = useState(false);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [refreshError, setRefreshError] = useState("");
  const [npcapBusy, setNpcapBusy] = useState("");
  const npcapPending = useRef(false);
  const mounted = useRef(true);
  const revision = useRef(0);
  const pending = useRef(false);
  const loaded = useRef(false);
  const closeRef = useRef<HTMLButtonElement>(null);

  function accept(next: EasyTierView, acceptConfig: boolean) {
    setView(next);
    if (acceptConfig) {
      if (!loaded.current && ["starting", "running", "stopping"].includes(next.status.phase)) setPage("running");
      setConfig(next.config);
      setPeers(next.config.peers.join("\n"));
      setListeners(next.config.listeners.join("\n"));
      setMethod(next.config.peers.length === 0 ? "standalone" : next.config.peers.length === 1 && ["tcp://public.easytier.top:11010", "udp://public.easytier.top:11010"].includes(next.config.peers[0]) ? "public" : "manual");
      loaded.current = true;
    }
  }

  async function refresh() {
    if (!desktop || pending.current) return;
    const token = revision.current;
    try {
      const next = await invokeBackend<EasyTierView>("easy_tier_status", {});
      if (token === revision.current) { accept(next, !loaded.current); setRefreshError(""); }
    } catch (failure) {
      if (token === revision.current) setRefreshError(errorMessage(failure));
    }
  }

  useEffect(() => {
    mounted.current = true;
    revision.current += 1;
    closeRef.current?.focus();
    void refresh();
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 2000);
    return () => { mounted.current = false; revision.current += 1; window.clearInterval(timer); };
  }, []);

  const phase = view?.status.phase ?? "idle";
  const active = phase === "starting" || phase === "running" || phase === "stopping";
  const npc = view?.npcap;
  const dependencyBusy = Boolean(npcapBusy) || npc?.phase === "downloading" || npc?.phase === "installing";
  const needsNpcap = npc?.required && !npc.installed;
  const locked = Boolean(busy) || dependencyBusy || active || !desktop || !loaded.current;
  const draft: EasyTierConfig = { ...config, peers: easyTierEndpoints(peers), listeners: easyTierEndpoints(listeners) };
  const valid = easyTierConfigValid(draft);

  function patch(value: Partial<EasyTierConfig>) { setConfig(current => ({ ...current, ...value })); }

  async function perform(action: "save" | "connect" | "disconnect") {
    if (pending.current || npcapPending.current || !desktop || dependencyBusy) return;
    if (action === "connect" && needsNpcap) { setPage("component"); return; }
    if ((action === "save" || action === "connect") && !valid) return;
    if (action === "connect" && (!loaded.current || !easyTierSecretValid(secret))) return;
    pending.current = true;
    const token = ++revision.current;
    setBusy(action); setError("");
    try {
      let next: EasyTierView;
      if (action === "connect") {
        next = await invokeBackend<EasyTierView>("start_easy_tier", { config: draft, networkSecret: secret });
      } else if (action === "disconnect") {
        next = await invokeBackend<EasyTierView>("stop_easy_tier", {});
      } else {
        next = await invokeBackend<EasyTierView>("save_easy_tier_config", { config: draft });
      }
      if (token === revision.current) {
        accept(next, true);
        if (action === "connect") { setSecret(""); setShowSecret(false); setPage("running"); }
      }
    } catch (failure) {
      if (failure && typeof failure === "object" && "code" in failure && failure.code === "npcap-required") setPage("component");
      if (token === revision.current) setError(errorMessage(failure));
    } finally {
      pending.current = false;
      if (token === revision.current) setBusy("");
    }
  }

  async function npcAction(action: "download" | "launch" | "recheck") {
    if (npcapPending.current || pending.current || !desktop || active || npc?.phase === "installing") return;
    npcapPending.current = true;
    revision.current += 1;
    setNpcapBusy(action); setError("");
    try {
      const next = await invokeBackend<NpcapStatus>(action === "download" ? "download_npcap_installer" : action === "launch" ? "launch_npcap_installer" : "recheck_npcap", {});
      revision.current += 1;
      if (mounted.current) setView(current => current ? { ...current, npcap: next } : current);
    } catch (failure) {
      if (mounted.current && !(failure && typeof failure === "object" && "code" in failure && failure.code === "cancelled")) setError(errorMessage(failure));
    } finally {
      npcapPending.current = false;
      if (mounted.current) { setNpcapBusy(""); void refresh(); }
    }
  }

  const connectionText = phase === "starting" ? t("easytier-starting")
    : phase === "stopping" ? t("easytier-stopping")
      : phase === "failed" ? t("easytier-failed")
        : phase === "running" ? (view?.status.peers.length ? t("easytier-connected", [view.status.peers.length]) : t("easytier-waiting"))
          : t("easytier-disconnected");

  function selectMethod(next: typeof method) {
    setMethod(next);
    if (next === "public") setPeers("tcp://public.easytier.top:11010");
    if (next === "standalone" || next === "manual" && method !== "manual") setPeers("");
  }

  function newConfiguration() {
    if (locked) return;
    setConfig(defaultEasyTierConfig()); setPeers(""); setListeners(defaultEasyTierConfig().listeners.join("\n"));
    setMethod("standalone"); setSecret(""); setShowSecret(false); setError(""); setPage("config");
  }

  return <div className="dialog-backdrop utility-backdrop" onMouseDown={event => { if (!busy && !npcapBusy && event.target === event.currentTarget) onClose(); }}>
    <section className="wind-dialog easytier-dialog" role="dialog" aria-modal="true" aria-labelledby="easytier-title" onKeyDown={event => {
      if (event.key === "Escape" && !busy && !npcapBusy) { event.preventDefault(); event.stopPropagation(); onClose(); }
    }}>
      <header className="dialog-title">
        <Network size={17} aria-hidden="true" /><strong id="easytier-title">{t("easytier-network")}</strong><span className="easytier-version">v{view?.componentVersion ?? "2.6.4"}</span>
        <button type="button" ref={closeRef} aria-label={t("close")} disabled={Boolean(busy || npcapBusy)} onClick={onClose}><X size={20} /></button>
      </header>
      <div className="easytier-toolbar">
        <button type="button" className="easytier-new" disabled={locked} onClick={newConfiguration}><Plus size={16} />{t("easytier-new-network")}</button>
        <div className="easytier-network-selected"><Network size={18} /><strong>{config.networkName || "PortMate"}</strong><span className={`easytier-badge ${phase}`} role="status" aria-live="polite">{connectionText}</span></div>
        <button type="button" className={page === "component" ? "selected" : ""} onClick={() => setPage(page === "component" ? "config" : "component")} aria-pressed={page === "component"}><Settings2 size={16} />{t("easytier-settings")}</button>
      </div>
      <div className="easytier-steps" role="tablist" aria-label={t("easytier-network")} onKeyDown={event => {
        if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        const next = event.key === "Home" ? "config" : event.key === "End" ? "running" : page === "config" ? "running" : "config";
        setPage(next);
        event.currentTarget.querySelector<HTMLButtonElement>(`#easytier-${next}-tab`)?.focus();
      }}>
        <button type="button" role="tab" id="easytier-config-tab" aria-controls="easytier-config-panel" tabIndex={page === "running" ? -1 : 0} aria-selected={page === "config"} onClick={() => setPage("config")}><span>1</span>{t("easytier-configure")}</button>
        <i aria-hidden="true" />
        <button type="button" role="tab" id="easytier-running-tab" aria-controls="easytier-running-panel" tabIndex={page === "running" ? 0 : -1} aria-selected={page === "running"} onClick={() => setPage("running")}><span>2</span>{t("easytier-running-page")}</button>
      </div>
      <div className="easytier-body">
        <form hidden={page !== "config"} id="easytier-config-panel" role="tabpanel" aria-labelledby="easytier-config-tab" className="easytier-config" onSubmit={event => { event.preventDefault(); void perform("connect"); }}>
          <fieldset className="easytier-basic" disabled={locked}>
            <legend>{t("easytier-basic")}</legend>
            <div className="easytier-address-heading"><label htmlFor="easytier-ip">{t("easytier-virtual-ip")}</label><label className="easytier-checkbox"><input type="checkbox" checked={config.addressMode === "dhcp"} disabled={config.addressMode === "relay" || locked} onChange={event => patch({ addressMode: event.target.checked ? "dhcp" : "static" })} />DHCP</label></div>
            <input id="easytier-ip" value={config.addressMode === "dhcp" ? t("easytier-auto-address") : config.addressMode === "relay" ? t("easytier-relay") : config.virtualIpv4} placeholder="10.144.144.2" disabled={locked || config.addressMode !== "static"} onChange={event => patch({ virtualIpv4: event.target.value })} />
            <div className="easytier-two-fields">
              <label>{t("easytier-network-name")}<input value={config.networkName} maxLength={128} onChange={event => patch({ networkName: event.target.value })} required autoComplete="off" /></label>
              <div className="easytier-secret-field"><label htmlFor="easytier-secret">{t("easytier-network-secret")}</label><div className="easytier-password"><input id="easytier-secret" type={showSecret ? "text" : "password"} value={secret} maxLength={1024} onChange={event => setSecret(event.target.value)} autoComplete="new-password" spellCheck={false} /><button type="button" aria-label={t(showSecret ? "easytier-hide-secret" : "easytier-show-secret")} onClick={() => setShowSecret(value => !value)}>{showSecret ? <EyeOff size={17} /> : <Eye size={17} />}</button></div></div>
            </div>
            <small>{t("easytier-secret-note")}</small>
            <div className="easytier-method">
              <span>{t("easytier-method")}</span>
              <div className="easytier-segmented" role="group" aria-label={t("easytier-method")}>
                {(["public", "manual", "standalone"] as const).map(value => <button type="button" key={value} aria-pressed={method === value} onClick={() => selectMethod(value)}>{t(`easytier-method-${value}`)}</button>)}
              </div>
              {method === "public" && <><select aria-label={t("easytier-peers")} value={peers} onChange={event => setPeers(event.target.value)}><option value="tcp://public.easytier.top:11010">tcp://public.easytier.top:11010</option><option value="udp://public.easytier.top:11010">udp://public.easytier.top:11010</option></select><small>{t("easytier-public-note")}</small></>}
              {method === "manual" && <EndpointList label={t("easytier-peers")} text={peers} onChange={setPeers} placeholder="tcp://192.168.1.20:11010" />}
              {method === "standalone" && <p className="easytier-method-note">{t("easytier-standalone-note")}</p>}
            </div>
          </fieldset>
          <details className="easytier-advanced">
            <summary><span>{t("easytier-advanced")}</span><Plus size={17} /></summary>
            <fieldset disabled={locked}>
              <div className="easytier-two-fields"><label>{t("easytier-hostname")}<input value={config.hostname} maxLength={128} onChange={event => patch({ hostname: event.target.value })} /></label><label>{t("easytier-address-mode")}<select aria-label={t("easytier-address-mode")} value={config.addressMode} onChange={event => patch({ addressMode: event.target.value as EasyTierConfig["addressMode"] })}><option value="dhcp">{t("easytier-dhcp")}</option><option value="static">{t("easytier-static")}</option><option value="relay">{t("easytier-relay")}</option></select></label></div>
              <EndpointList label={t("easytier-listeners")} text={listeners} onChange={setListeners} placeholder="tcp://0.0.0.0:11010" />
              <small>{t("easytier-listeners-note")}</small>
              <div className="easytier-two-fields"><label>MTU<input type="number" min={576} max={9000} step={1} value={config.mtu} onChange={event => patch({ mtu: Number(event.target.value) })} /></label><label className="easytier-checkbox"><input type="checkbox" checked={config.latencyFirst} onChange={event => patch({ latencyFirst: event.target.checked })} />{t("easytier-latency-first")}</label></div>
            </fieldset>
          </details>
          <p className="easytier-permissions"><ShieldCheck size={15} />{config.addressMode === "relay" ? t("easytier-relay-note") : t("easytier-permission-note")}</p>
          {needsNpcap && <p className="easytier-method-note">{t("npcap-required-note")} <button type="button" onClick={() => setPage("component")}>{t("npcap-setup")}</button></p>}
        </form>
        <section hidden={page !== "running"} id="easytier-running-panel" role="tabpanel" aria-labelledby="easytier-running-tab" className="easytier-running">
          <div className="easytier-section-heading"><h2>{t("easytier-my-node")}</h2><button type="button" disabled={!desktop || Boolean(busy)} aria-label={t("refresh")} onClick={() => void refresh()}><RefreshCw size={15} />{t("refresh")}</button></div>
          <div className="easytier-node-overview"><div className="easytier-peer-count"><Network size={21} /><strong>{view?.status.peers.length ?? 0}</strong><span>{t("easytier-nodes")}</span></div><dl><div><dt>{t("easytier-virtual-ip")}</dt><dd>{view?.status.virtualIpv4 || "—"}</dd></div><div><dt>{t("easytier-hostname")}</dt><dd>{view?.status.hostname || "—"}</dd></div></dl>{phase === "running" && !view?.status.error && <span className="easytier-node-mark active"><ShieldCheck size={20} />{t("easytier-encrypted")}</span>}</div>
          <div className="easytier-section-heading"><h2>{t("easytier-nodes")}</h2><button type="button" aria-expanded={logsOpen} onClick={() => setLogsOpen(value => !value)}>{t("easytier-logs")}</button></div>
          <div className="easytier-table-wrap"><table className="easytier-peers"><thead><tr><th>{t("easytier-hostname")}</th><th>{t("easytier-virtual-ip")}</th><th>{t("easytier-route")}</th><th>{t("easytier-latency")}</th><th>{t("easytier-received")}</th><th>{t("easytier-sent")}</th></tr></thead><tbody>{view?.status.peers.length ? view.status.peers.map(peer => <tr key={peer.id}><td><strong>{peer.hostname || peer.id}</strong><small>{peer.protocol}</small></td><td><code>{peer.ipv4 || "—"}</code></td><td>{peer.route}</td><td>{peer.latency} ms</td><td>{peer.received}</td><td>{peer.sent}</td></tr>) : <tr><td colSpan={6} className="easytier-empty"><Network size={30} /><span>{t("easytier-no-peers")}</span></td></tr>}</tbody></table></div>
          {logsOpen && <label className="easytier-logs">{t("easytier-logs")}<pre tabIndex={0}>{view?.status.logs.join("\n") || t("easytier-no-logs")}</pre></label>}
        </section>
        {page === "component" && <section className="easytier-component-panel" aria-label={t("easytier-component")}>
          <div className="easytier-section-heading"><h2>{t("easytier-component")}</h2><span>v{view?.componentVersion ?? "2.6.4"}</span></div>
          <div className="easytier-component-status"><span className="ready"><ShieldCheck size={20} /></span><div><strong>{t("easytier-component-ready", [view?.componentVersion ?? "2.6.4"])}</strong><small>{t("easytier-bundled-note")}</small></div></div>
          <p className="easytier-permissions">{t("easytier-license-note")}</p>
          <p className="easytier-attribution">{t("easytier-attribution")} <OfficialLink href="https://github.com/EasyTier/EasyTier">EasyTier</OfficialLink> · <OfficialLink href="https://github.com/EasyTier/EasyTier/blob/v2.6.4/LICENSE">LGPL-3.0</OfficialLink> · <OfficialLink href="https://github.com/EasyTier/EasyTier/tree/v2.6.4">{t("npcap-source")}</OfficialLink></p>
          <p className="easytier-permissions">{t("easytier-responsible-use")}</p>
          {npc?.required && <section className="npcap-setup" aria-label={t("npcap-setup")}>
            <div className="easytier-section-heading"><h2>Npcap</h2><span>{t(npc.installed ? "npcap-detected" : "npcap-not-installed")}</span></div>
            <p>{t("npcap-independent-note")}</p>
            <p>{t("npcap-license-limits")}</p>
            <div className="npcap-links"><OfficialLink href="https://npcap.com/#download">{t("npcap-official-site")}</OfficialLink><OfficialLink href="https://github.com/nmap/npcap/blob/v1.90/LICENSE">{t("npcap-license")}</OfficialLink><OfficialLink href="https://npcap.com/oem/">{t("npcap-oem")}</OfficialLink></div>
            <p className="npcap-state" role="status">{t(`npcap-phase-${npc.phase}`)}</p>
            {npc.phase === "downloading" && <progress aria-label={t("npcap-download-progress")} value={npc.total ? npc.downloaded : undefined} max={npc.total || undefined} />}
            <p>{t("npcap-interactive-note")}</p>
            <div className="npcap-controls">
              <button type="button" disabled={locked || npcapPending.current} onClick={() => void npcAction("download")}><Download size={14} />{t("npcap-download", [npc.installerVersion])}</button>
              <button type="button" disabled={!desktop || Boolean(busy) || Boolean(npcapBusy) || npc.phase !== "downloaded" || active} onClick={() => void npcAction("launch")}><ExternalLink size={14} />{t("npcap-open-installer")}</button>
              <button type="button" disabled={!desktop || Boolean(busy) || dependencyBusy || active} onClick={() => void npcAction("recheck")}><RefreshCw size={14} />{t("npcap-recheck")}</button>
              {npc.phase === "downloading" && <button type="button" onClick={() => void invokeBackend("cancel_npcap_download", {}).catch(failure => setError(errorMessage(failure)))}>{t("cancel")}</button>}
            </div>
            {(npc.error && !error) && <p role="alert">{errorMessage(npc.error)}</p>}
          </section>}
        </section>}
      </div>
      <div className="easytier-feedback" aria-live="polite">
        {!desktop && <p>{t("easytier-desktop-only")}</p>}
        {(error || refreshError || view?.status.error) && <p role="alert">{error || refreshError || errorMessage(view?.status.error)}</p>}
      </div>
      <footer className="easytier-actions">
        <small title={t("easytier-lifetime")}>{t("easytier-managed-note")}</small><span />
        {!active && <button type="button" disabled={locked || !valid} onClick={() => void perform("save")}><Save size={14} />{t("save")}</button>}
        {active ? <button type="button" disabled={Boolean(busy) || phase === "stopping"} onClick={() => void perform("disconnect")}><Square size={14} />{t("easytier-disconnect")}</button>
          : <button type="button" className="primary" disabled={!desktop || !loaded.current || Boolean(busy) || dependencyBusy || !valid || (!needsNpcap && !easyTierSecretValid(secret))} onClick={() => {
            if (!needsNpcap) void perform("connect");
            else if (page !== "component") setPage("component");
            else void npcAction(npc.phase === "downloaded" ? "launch" : "download");
          }}><Play size={14} />{needsNpcap ? page !== "component" ? t("npcap-setup") : npc.phase === "downloaded" ? t("npcap-open-installer") : t("npcap-download", [npc.installerVersion]) : busy === "connect" ? t("easytier-starting") : t("easytier-connect")}</button>}
        <button type="button" disabled={Boolean(busy || npcapBusy)} onClick={onClose}>{t("close")}</button>
      </footer>
    </section>
  </div>;
}

function OfficialLink({ href, children }: { href: string; children: React.ReactNode }) {
  return <a href={href} target="_blank" rel="noopener noreferrer" onClick={event => openTerminalWebLink(event, href)}>{children}</a>;
}

function EndpointList({ label, text, onChange, placeholder }: { label: string; text: string; onChange: (value: string) => void; placeholder: string }) {
  const values = text ? text.split("\n") : [""];
  return <div className="easytier-endpoints"><span>{label}</span>{values.map((value, index) => <div className="easytier-endpoint-row" key={index}><input aria-label={`${label} ${index + 1}`} value={value} placeholder={placeholder} maxLength={512} spellCheck={false} onChange={event => onChange(values.map((old, key) => key === index ? event.target.value : old).join("\n"))} /><button type="button" aria-label={t("easytier-remove-endpoint", [index + 1])} onClick={() => onChange(values.filter((_, key) => key !== index).join("\n"))}><Trash2 size={16} /></button></div>)}<button type="button" className="easytier-add-endpoint" disabled={values.length >= 32} onClick={() => onChange([...values, ""].join("\n"))}><Plus size={14} />{t("easytier-add-endpoint")}</button></div>;
}

function errorMessage(error: unknown): string {
  const code = error && typeof error === "object" && "code" in error ? String(error.code) : "";
  const known: Record<string, string> = {
    "invalid-network": "easytier-error-network", "invalid-secret": "easytier-error-secret", "invalid-endpoint": "easytier-error-endpoint",
    "invalid-address": "easytier-error-address", "invalid-config": "easytier-error-config", "component-missing": "easytier-component-missing",
    "version-mismatch": "easytier-error-version",
    "already-running": "easytier-error-running", busy: "easytier-error-busy",
    "npcap-conflict": "npcap-conflict-error", "npcap-required": "npcap-required-note", "npcap-verification": "npcap-verification-error",
    "npcap-download": "npcap-download-error", "npcap-setup": "npcap-setup-error", "npcap-installing": "npcap-phase-installing",
  };
  if (known[code]) return t(known[code]);
  const detail = error && typeof error === "object" && "detail" in error ? String(error.detail) : String(error);
  return t("easytier-error", [detail]);
}
