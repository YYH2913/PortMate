import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { Braces, Play, Plus, RefreshCw, Save, Trash2, X } from "lucide-react";
import { invokeBackend } from "./api";
import { KeyedRequestGate } from "./keyed-request-gate";
import {
  customScriptDraft,
  customScriptDraftMatches,
  MAX_CUSTOM_SCRIPTS,
  MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS,
  MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS,
  MAX_CUSTOM_SCRIPT_NAME_CHARACTERS,
  newCustomScriptDraft,
  normalizeCustomScriptDraft,
  validateCustomScriptDraft,
} from "./custom-script-state";
import type { CustomScript, HostScriptConfig, HostScriptResult, McpGrant, RunHostScriptRequest, SaveCustomScriptRequest, SaveCustomScriptResponse } from "./types";

export default function CustomScriptDialog({
  onClose,
  onNotice,
}: {
  onClose: () => void;
  onNotice: (message: string) => void;
}) {
  useLocale();
  const [scripts, setScripts] = useState<CustomScript[]>([]);
  const [draft, setDraft] = useState<SaveCustomScriptRequest | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [clients, setClients] = useState<McpGrant[]>([]);
  const [parameters, setParameters] = useState("{}");
  const [result, setResult] = useState<HostScriptResult | null>(null);
  const [runId, setRunId] = useState<string | null>(null);
  const activeRunRef = useRef<string | null>(null);
  const operationGate = useRef(new KeyedRequestGate<"operation">());
  const selectedScript = draft?.id ? scripts.find((script) => script.id === draft.id) : null;
  const hasUnsavedChanges = Boolean(draft && (!selectedScript || !customScriptDraftMatches(draft, selectedScript)));

  useEffect(() => {
    const gate = operationGate.current;
    const token = gate.replace("operation");
    void Promise.all([invokeBackend<CustomScript[]>("list_custom_scripts", {}), invokeBackend<McpGrant[]>("list_mcp_grants", {})])
      .then(([items, grants]) => {
        if (!gate.isCurrent("operation", token)) return;
        setClients(grants);
        setScripts(items);
        setDraft(items[0] ? customScriptDraft(items[0]) : null);
      })
      .catch((reason) => gate.isCurrent("operation", token) && setError(formatError(reason)))
      .finally(() => {
        if (gate.finish("operation", token)) setLoading(false);
      });
    return () => {
      gate.invalidateAll();
      if (activeRunRef.current) void invokeBackend("cancel_host_script", { runId: activeRunRef.current }).catch(() => {});
    };
  }, []);

  function selectScript(script: CustomScript) {
    if (operationGate.current.isActive("operation") || loading || busy) return;
    if (!confirmDiscardChanges(t("switch-script"))) return;
    setDraft(customScriptDraft(script));
    setResult(null);
    setParameters("{}");
    setError("");
  }

  function createScript() {
    if (operationGate.current.isActive("operation") || loading || busy) return;
    if (scripts.length >= MAX_CUSTOM_SCRIPTS) {
      setError(t("at-most-custom-scripts-can-be-saved", [MAX_CUSTOM_SCRIPTS]));
      return;
    }
    if (!confirmDiscardChanges(t("new-script"))) return;
    setDraft(newCustomScriptDraft());
    setResult(null);
    setParameters("{}");
    setError("");
  }

  function confirmDiscardChanges(action: string): boolean {
    return !hasUnsavedChanges || window.confirm(t("this-script-has-unsaved-changes-will-discard-them-continue", [action]));
  }

  function closeDialog() {
    if (operationGate.current.isActive("operation") || loading || busy || !confirmDiscardChanges(t("close-window"))) return;
    onClose();
  }

  function updateDraft(patch: Partial<SaveCustomScriptRequest>) {
    setDraft((current) => current ? { ...current, ...patch } : current);
    setResult(null);
    setError("");
  }

  async function refreshScripts() {
    if (loading || busy || operationGate.current.isActive("operation")) return;
    if (!confirmDiscardChanges(t("refresh-scripts"))) return;
    const gate = operationGate.current;
    const token = gate.begin("operation");
    if (token === null) return;
    const selectedId = draft?.id ?? null;
    setLoading(true);
    setError("");
    try {
      const [items, grants] = await Promise.all([invokeBackend<CustomScript[]>("list_custom_scripts", {}), invokeBackend<McpGrant[]>("list_mcp_grants", {})]);
      if (!gate.isCurrent("operation", token)) return;
      const selected = items.find((script) => script.id === selectedId) ?? items[0];
      setClients(grants);
      setResult(null);
      setScripts(items);
      setDraft(selected ? customScriptDraft(selected) : null);
      if (selected?.id !== selectedId) setParameters("{}");
    } catch (reason) {
      if (gate.isCurrent("operation", token)) setError(formatError(reason));
    } finally {
      if (gate.finish("operation", token)) setLoading(false);
    }
  }

  function updateHost(patch: Partial<HostScriptConfig>) {
    if (draft) updateDraft({ host: { ...draft.host, ...patch } });
  }

  async function saveScript() {
    if (!draft || busy || operationGate.current.isActive("operation")) return;
    const normalized = normalizeCustomScriptDraft(draft);
    const validation = validateCustomScriptDraft(normalized);
    if (validation) {
      setError(validation);
      return;
    }
    const gate = operationGate.current;
    const token = gate.begin("operation");
    if (token === null) return;
    setBusy(true);
    setError("");
    try {
      const response = await invokeBackend<SaveCustomScriptResponse>("save_custom_script", { request: normalized });
      if (!gate.isCurrent("operation", token)) return;
      const saved = response.scripts.find((script) => script.id === response.savedId);
      if (!saved) throw new Error(t("the-save-response-did-not-contain-the-submitted-script"));
      setScripts(response.scripts);
      setDraft(customScriptDraft(saved));
    } catch (reason) {
      if (gate.isCurrent("operation", token)) setError(formatError(reason));
    } finally {
      if (gate.finish("operation", token)) setBusy(false);
    }
  }

  async function deleteScript() {
    if (!selectedScript || busy || operationGate.current.isActive("operation")) return;
    const unsavedWarning = hasUnsavedChanges ? t("unsaved-editor-changes-will-also-be-discarded") : "";
    if (!window.confirm(t("delete-custom-script", [selectedScript.name, unsavedWarning]))) return;
    const gate = operationGate.current;
    const token = gate.begin("operation");
    if (token === null) return;
    const pendingScript = selectedScript;
    setBusy(true);
    setError("");
    try {
      const items = await invokeBackend<CustomScript[]>("delete_custom_script", {
        request: { id: pendingScript.id, expectedUpdatedAt: pendingScript.updatedAt },
      });
      if (!gate.isCurrent("operation", token)) return;
      setScripts(items);
      setDraft(items[0] ? customScriptDraft(items[0]) : null);
      setResult(null);
      setParameters("{}");
    } catch (reason) {
      if (gate.isCurrent("operation", token)) setError(formatError(reason));
    } finally {
      if (gate.finish("operation", token)) setBusy(false);
    }
  }

  async function runScript() {
    if (!selectedScript || busy || hasUnsavedChanges || operationGate.current.isActive("operation")) return;
    let input: Record<string, unknown>;
    try {
      input = JSON.parse(parameters);
      if (!input || typeof input !== "object" || Array.isArray(input)) throw new Error(t("run-parameters-must-be-a-json-object"));
    } catch (reason) { setError(formatError(reason)); return; }
    const gate = operationGate.current;
    const token = gate.begin("operation");
    if (token === null) return;
    const id = crypto.randomUUID();
    setRunId(id);
    activeRunRef.current = id;
    setBusy(true);
    setError("");
    setResult(null);
    try {
      const request: RunHostScriptRequest = {
        scriptId: selectedScript.id, expectedUpdatedAt: selectedScript.updatedAt, runId: id, parameters: input,
      };
      const response = await invokeBackend<HostScriptResult>("run_host_script", { request });
      if (!gate.isCurrent("operation", token)) return;
      setResult(response);
      if (response.failure) setError(response.failure);
      else onNotice(t("executed-on-the-portmate-host-exit-code", [selectedScript.name, response.exitCode]));
    } catch (reason) {
      if (gate.isCurrent("operation", token)) setError(formatError(reason));
    } finally {
      activeRunRef.current = null;
      if (gate.finish("operation", token)) { setBusy(false); setRunId(null); }
    }
  }

  async function cancelRun() {
    if (!runId) return;
    try { await invokeBackend("cancel_host_script", { runId }); }
    catch (reason) { setError(formatError(reason)); }
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && closeDialog()}>
      <section className="wind-dialog custom-script-dialog" role="dialog" aria-modal="true" aria-labelledby="custom-script-title">
        <header className="dialog-title">
          <Braces size={18} />
          <strong id="custom-script-title">{t("custom-scripts")}</strong>
          <button type="button" title={loading || busy ? t("close-after-the-operation-finishes") : t("close")} aria-label={t("close-custom-scripts")} disabled={loading || busy} onClick={closeDialog}><X size={20} /></button>
        </header>
        <div className="custom-script-content">
          <aside className="custom-script-list">
            <header>
              <strong>{t("scripts")}</strong>
              <span>{scripts.length}/{MAX_CUSTOM_SCRIPTS}</span>
              <button type="button" title={hasUnsavedChanges ? t("confirm-discarding-changes-and-reload-scripts") : t("refresh-scripts")} aria-label={t("refresh-custom-scripts")} disabled={loading || busy} onClick={() => void refreshScripts()}><RefreshCw size={14} /></button>
              <button type="button" title={t("add-script")} aria-label={t("add-custom-script")} disabled={loading || busy || scripts.length >= MAX_CUSTOM_SCRIPTS} onClick={createScript}><Plus size={14} /></button>
            </header>
            <div role="listbox" aria-label={t("custom-script-list")}>
              {draft && !draft.id ? (
                <button type="button" className="active" role="option" aria-selected="true"><Braces size={13} /><span>{t("new-script-2")}</span></button>
              ) : null}
              {scripts.map((script) => (
                <button key={script.id} type="button" role="option" aria-selected={draft?.id === script.id} className={draft?.id === script.id ? "active" : ""} disabled={loading || busy} onClick={() => selectScript(script)}>
                  <Braces size={13} />
                  <span>{script.name}</span>
                  {script.mcpEnabled ? <i title={t("mcp-enabled")} aria-label={t("mcp-enabled")}>MCP</i> : null}
                </button>
              ))}
              {!loading && !scripts.length && !draft ? <div className="custom-script-empty">{t("no-custom-scripts")}</div> : null}
            </div>
          </aside>
          {draft ? (
            <section className="custom-script-editor">
              <div className="host-script-fields">
              <div className="custom-script-meta-fields">
                <label><span>{t("name")}</span><input aria-label={t("script-name")} disabled={loading || busy} maxLength={MAX_CUSTOM_SCRIPT_NAME_CHARACTERS} value={draft.name} onChange={(event) => updateDraft({ name: event.target.value })} /></label>
                <label><span>{t("description")}</span><input aria-label={t("script-description")} disabled={loading || busy} maxLength={MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS} value={draft.description} onChange={(event) => updateDraft({ description: event.target.value })} /></label>
              </div>
              <p className="host-script-warning">{t("scripts-run-on-the-portmate-host-with-the-current")}</p>
              <div className="custom-script-meta-fields">
                <label><span>{t("language")}</span><select aria-label={t("script-language")} disabled={loading || busy} value={draft.host.language} onChange={(event) => updateHost({ language: event.target.value as HostScriptConfig["language"], interpreter: "" })}><option value="python">Python 3</option><option value="shell">{t("platform-shell-unix-sh-windows-powershell-7")}</option></select></label>
                <label><span>{t("absolute-interpreter-path-blank-for-auto-detection")}</span><input aria-label={t("script-interpreter")} disabled={loading || busy} value={draft.host.interpreter} onChange={(event) => updateHost({ interpreter: event.target.value })} /></label>
                <label><span>{t("working-directory-blank-for-the-user-s-home-directory")}</span><input aria-label={t("script-working-directory")} disabled={loading || busy} value={draft.host.workingDirectory} onChange={(event) => updateHost({ workingDirectory: event.target.value })} /></label>
                <label><span>{t("timeout-1-60-seconds")}</span><input aria-label={t("script-timeout")} type="number" min={1} max={60} disabled={loading || busy} value={draft.host.timeoutSeconds} onChange={(event) => updateHost({ timeoutSeconds: Number(event.target.value) })} /></label>
              </div>
              <label className="custom-script-body">
                <span>{t("scripts")}</span>
                <textarea aria-label={t("script-body")} disabled={loading || busy} spellCheck={false} value={draft.content} onChange={(event) => updateDraft({ content: event.target.value.replace(/\r\n?/g, "\n") })} aria-describedby="custom-script-content-limit" />
                <small id="custom-script-content-limit">{t("at-most-characters-oversized-content-is-not-automatically-truncated", [MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS])}</small>
              </label>
              <small>{t("parameters-are-supplied-as-json-through-stdin-and-portmate")}</small>
              <fieldset className="host-script-parameters" disabled={loading || busy}>
                <legend>{t("tool-parameters")}</legend>
                {draft.host.parameters.map((parameter, index) => (
                  <div className="host-script-parameter" key={index}>
                    <input aria-label={t("parameter-name", [index + 1])} placeholder={t("parameter-name-2")} value={parameter.name} onChange={(event) => updateHost({ parameters: draft.host.parameters.map((p, i) => i === index ? { ...p, name: event.target.value } : p) })} />
                    <input aria-label={t("parameter-description", [index + 1])} placeholder={t("description")} value={parameter.description} onChange={(event) => updateHost({ parameters: draft.host.parameters.map((p, i) => i === index ? { ...p, description: event.target.value } : p) })} />
                    <select aria-label={t("parameter-type", [index + 1])} value={parameter.kind} onChange={(event) => updateHost({ parameters: draft.host.parameters.map((p, i) => i === index ? { ...p, kind: event.target.value as typeof p.kind } : p) })}>
                      <option value="string">{t("string")}</option><option value="number">{t("number")}</option><option value="integer">{t("integer")}</option><option value="boolean">{t("boolean")}</option>
                    </select>
                    <label><input type="checkbox" checked={parameter.required} onChange={(event) => updateHost({ parameters: draft.host.parameters.map((p, i) => i === index ? { ...p, required: event.target.checked } : p) })} />{t("required")}</label>
                    <button type="button" aria-label={t("delete-parameter", [index + 1])} onClick={() => updateHost({ parameters: draft.host.parameters.filter((_, i) => i !== index) })}><Trash2 size={14} /></button>
                  </div>
                ))}
                <button type="button" disabled={draft.host.parameters.length >= 32} onClick={() => updateHost({ parameters: [...draft.host.parameters, { name: "", description: "", kind: "string", required: false }] })}>{t("add-parameter")}</button>
              </fieldset>
              <div className="custom-script-boundary">
                <label><input type="checkbox" disabled={loading || busy} checked={draft.mcpEnabled} onChange={(event) => updateDraft({ mcpEnabled: event.target.checked })} />{t("expose-to-selected-mcp-clients")}</label>
                <div className="custom-script-session-list" aria-label={t("allowed-mcp-clients-for-this-script")}>
                  {clients.filter((client) => !client.revokedAt).map((client) => (
                    <label key={client.clientId}><input type="checkbox" disabled={loading || busy} checked={draft.host.allowedClientIds.includes(client.clientId)} onChange={(event) => updateHost({ allowedClientIds: event.target.checked ? [...draft.host.allowedClientIds, client.clientId] : draft.host.allowedClientIds.filter((id) => id !== client.clientId) })} /><span>{client.name}（{client.clientId}）{!client.scopes.includes("run-scripts") ? t("no-run-scripts-permission") : ""}</span></label>
                  ))}
                  {draft.host.allowedClientIds.filter((id) => !clients.some((client) => client.clientId === id && !client.revokedAt)).map((id) => (
                    <label key={id}><input type="checkbox" aria-label={t("remove-client") + id} checked disabled={loading || busy} onChange={() => updateHost({ allowedClientIds: draft.host.allowedClientIds.filter((value) => value !== id) })} /><span>{t("client-revoked-or-missing-remove-it", [id])}</span></label>
                  ))}
                  {!clients.length ? <small>{t("create-a-client-grant-in-mcp-management-first")}</small> : null}
                </div>
                <small>{t("clients-also-require-run-scripts-permission-refresh-the-mcp")}</small>
              </div>
              <label>{t("run-parameters-json")}<textarea aria-label={t("script-run-parameters")} disabled={loading || busy} value={parameters} onChange={(event) => setParameters(event.target.value)} spellCheck={false} /></label>
              {result ? <section className="host-script-result" aria-label={t("script-result")}><strong>{t("exit-code")}{result.exitCode ?? t("abnormal-exit")}</strong><pre>{result.stdout}</pre>{result.stderr ? <pre className="error">{result.stderr}</pre> : null}</section> : null}
              </div>
              <footer className="custom-script-actions">
                <div className="custom-script-run">
                  {runId ? <button type="button" onClick={() => void cancelRun()}>{t("stop-execution")}</button> : null}
                  <button type="button" title={hasUnsavedChanges ? t("save-before-running") : t("run-script")} aria-label={t("run-custom-script")} disabled={!selectedScript || loading || busy || hasUnsavedChanges} onClick={() => void runScript()}><Play size={14} /></button>
                </div>
                <span role={error ? "alert" : undefined}>{localizeDiagnostic(error)}</span>
                <button type="button" className="danger" title={t("delete-script")} aria-label={t("delete-custom-script-2")} disabled={!selectedScript || loading || busy} onClick={() => void deleteScript()}><Trash2 size={14} /></button>
                <button type="button" className="primary" title={t("save-script")} aria-label={t("save-custom-script")} disabled={loading || busy} onClick={() => void saveScript()}><Save size={14} /></button>
              </footer>
            </section>
          ) : (
            <section className="custom-script-editor custom-script-editor-empty">
              {error ? <p role="alert">{localizeDiagnostic(error)}</p> : null}
              <button type="button" disabled={loading || busy} onClick={createScript}><Plus size={14} /><span>{t("add-script")}</span></button>
            </section>
          )}
        </div>
      </section>
    </div>
  );
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
