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
    if (!confirmDiscardChanges("切换脚本")) return;
    setDraft(customScriptDraft(script));
    setResult(null);
    setParameters("{}");
    setError("");
  }

  function createScript() {
    if (operationGate.current.isActive("operation") || loading || busy) return;
    if (scripts.length >= MAX_CUSTOM_SCRIPTS) {
      setError(`自定义脚本最多保存 ${MAX_CUSTOM_SCRIPTS} 条。`);
      return;
    }
    if (!confirmDiscardChanges("新建脚本")) return;
    setDraft(newCustomScriptDraft());
    setResult(null);
    setParameters("{}");
    setError("");
  }

  function confirmDiscardChanges(action: string): boolean {
    return !hasUnsavedChanges || window.confirm(`当前脚本有未保存的更改，${action}将放弃这些内容。是否继续？`);
  }

  function closeDialog() {
    if (operationGate.current.isActive("operation") || loading || busy || !confirmDiscardChanges("关闭窗口")) return;
    onClose();
  }

  function updateDraft(patch: Partial<SaveCustomScriptRequest>) {
    setDraft((current) => current ? { ...current, ...patch } : current);
    setResult(null);
    setError("");
  }

  async function refreshScripts() {
    if (loading || busy || operationGate.current.isActive("operation")) return;
    if (!confirmDiscardChanges("刷新脚本")) return;
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
      if (!saved) throw new Error("保存响应没有包含已提交的自定义脚本，请重新打开后检查。");
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
    const unsavedWarning = hasUnsavedChanges ? "\n\n当前编辑器还有未保存的更改，也会一并丢弃。" : "";
    if (!window.confirm(`删除自定义脚本“${selectedScript.name}”？${unsavedWarning}`)) return;
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
      if (!input || typeof input !== "object" || Array.isArray(input)) throw new Error("运行参数必须是 JSON 对象。");
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
      else onNotice(`已在 PortMate 主机执行 ${selectedScript.name}，退出码 ${response.exitCode}`);
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
          <strong id="custom-script-title">自定义脚本</strong>
          <button type="button" title={loading || busy ? "操作完成后关闭" : "关闭"} aria-label="关闭自定义脚本" disabled={loading || busy} onClick={closeDialog}><X size={20} /></button>
        </header>
        <div className="custom-script-content">
          <aside className="custom-script-list">
            <header>
              <strong>脚本</strong>
              <span>{scripts.length}/{MAX_CUSTOM_SCRIPTS}</span>
              <button type="button" title={hasUnsavedChanges ? "确认放弃编辑后重新加载脚本" : "刷新脚本"} aria-label="刷新自定义脚本" disabled={loading || busy} onClick={() => void refreshScripts()}><RefreshCw size={14} /></button>
              <button type="button" title="添加脚本" aria-label="添加自定义脚本" disabled={loading || busy || scripts.length >= MAX_CUSTOM_SCRIPTS} onClick={createScript}><Plus size={14} /></button>
            </header>
            <div role="listbox" aria-label="自定义脚本列表">
              {draft && !draft.id ? (
                <button type="button" className="active" role="option" aria-selected="true"><Braces size={13} /><span>新脚本</span></button>
              ) : null}
              {scripts.map((script) => (
                <button key={script.id} type="button" role="option" aria-selected={draft?.id === script.id} className={draft?.id === script.id ? "active" : ""} disabled={loading || busy} onClick={() => selectScript(script)}>
                  <Braces size={13} />
                  <span>{script.name}</span>
                  {script.mcpEnabled ? <i title="已开放 MCP" aria-label="已开放 MCP">MCP</i> : null}
                </button>
              ))}
              {!loading && !scripts.length && !draft ? <div className="custom-script-empty">没有自定义脚本</div> : null}
            </div>
          </aside>
          {draft ? (
            <section className="custom-script-editor">
              <div className="host-script-fields">
              <div className="custom-script-meta-fields">
                <label><span>名称</span><input aria-label="脚本名称" disabled={loading || busy} maxLength={MAX_CUSTOM_SCRIPT_NAME_CHARACTERS} value={draft.name} onChange={(event) => updateDraft({ name: event.target.value })} /></label>
                <label><span>说明</span><input aria-label="脚本说明" disabled={loading || busy} maxLength={MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS} value={draft.description} onChange={(event) => updateDraft({ description: event.target.value })} /></label>
              </div>
              <p className="host-script-warning">脚本在 PortMate 主机上以当前用户权限执行，不是沙箱，也不会发送到终端会话。</p>
              <div className="custom-script-meta-fields">
                <label><span>语言</span><select aria-label="脚本语言" disabled={loading || busy} value={draft.host.language} onChange={(event) => updateHost({ language: event.target.value as HostScriptConfig["language"], interpreter: "" })}><option value="python">Python 3</option><option value="shell">平台 Shell（Unix sh / Windows PowerShell 7）</option></select></label>
                <label><span>解释器绝对路径（留空自动检测）</span><input aria-label="脚本解释器" disabled={loading || busy} value={draft.host.interpreter} onChange={(event) => updateHost({ interpreter: event.target.value })} /></label>
                <label><span>工作目录（留空为用户主目录）</span><input aria-label="脚本工作目录" disabled={loading || busy} value={draft.host.workingDirectory} onChange={(event) => updateHost({ workingDirectory: event.target.value })} /></label>
                <label><span>超时（1–60 秒）</span><input aria-label="脚本超时" type="number" min={1} max={60} disabled={loading || busy} value={draft.host.timeoutSeconds} onChange={(event) => updateHost({ timeoutSeconds: Number(event.target.value) })} /></label>
              </div>
              <label className="custom-script-body">
                <span>脚本</span>
                <textarea aria-label="脚本正文" disabled={loading || busy} spellCheck={false} value={draft.content} onChange={(event) => updateDraft({ content: event.target.value.replace(/\r\n?/g, "\n") })} aria-describedby="custom-script-content-limit" />
                <small id="custom-script-content-limit">最多 {MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS} 个字符；超限内容不会被自动截断。</small>
              </label>
              <small>参数通过标准输入 JSON 和 PORTMATE_INPUT_JSON 传入；Shell 可读取 PORTMATE_PARAM_参数名。Python：json.load(sys.stdin)。输出 UTF-8 到 stdout/stderr，每路最多 128 KiB。</small>
              <fieldset className="host-script-parameters" disabled={loading || busy}>
                <legend>工具参数</legend>
                {draft.host.parameters.map((parameter, index) => (
                  <div className="host-script-parameter" key={index}>
                    <input aria-label={`参数 ${index + 1} 名称`} placeholder="参数名" value={parameter.name} onChange={(event) => updateHost({ parameters: draft.host.parameters.map((p, i) => i === index ? { ...p, name: event.target.value } : p) })} />
                    <input aria-label={`参数 ${index + 1} 说明`} placeholder="说明" value={parameter.description} onChange={(event) => updateHost({ parameters: draft.host.parameters.map((p, i) => i === index ? { ...p, description: event.target.value } : p) })} />
                    <select aria-label={`参数 ${index + 1} 类型`} value={parameter.kind} onChange={(event) => updateHost({ parameters: draft.host.parameters.map((p, i) => i === index ? { ...p, kind: event.target.value as typeof p.kind } : p) })}>
                      <option value="string">字符串</option><option value="number">数字</option><option value="integer">整数</option><option value="boolean">布尔值</option>
                    </select>
                    <label><input type="checkbox" checked={parameter.required} onChange={(event) => updateHost({ parameters: draft.host.parameters.map((p, i) => i === index ? { ...p, required: event.target.checked } : p) })} />必填</label>
                    <button type="button" aria-label={`删除参数 ${index + 1}`} onClick={() => updateHost({ parameters: draft.host.parameters.filter((_, i) => i !== index) })}><Trash2 size={14} /></button>
                  </div>
                ))}
                <button type="button" disabled={draft.host.parameters.length >= 32} onClick={() => updateHost({ parameters: [...draft.host.parameters, { name: "", description: "", kind: "string", required: false }] })}>添加参数</button>
              </fieldset>
              <div className="custom-script-boundary">
                <label><input type="checkbox" disabled={loading || busy} checked={draft.mcpEnabled} onChange={(event) => updateDraft({ mcpEnabled: event.target.checked })} />开放给选定 MCP 客户端</label>
                <div className="custom-script-session-list" aria-label="脚本允许 MCP 客户端">
                  {clients.filter((client) => !client.revokedAt).map((client) => (
                    <label key={client.clientId}><input type="checkbox" disabled={loading || busy} checked={draft.host.allowedClientIds.includes(client.clientId)} onChange={(event) => updateHost({ allowedClientIds: event.target.checked ? [...draft.host.allowedClientIds, client.clientId] : draft.host.allowedClientIds.filter((id) => id !== client.clientId) })} /><span>{client.name}（{client.clientId}）{!client.scopes.includes("run-scripts") ? " · 尚无 run-scripts 授权" : ""}</span></label>
                  ))}
                  {draft.host.allowedClientIds.filter((id) => !clients.some((client) => client.clientId === id && !client.revokedAt)).map((id) => (
                    <label key={id}><input type="checkbox" aria-label={"移除客户端 " + id} checked disabled={loading || busy} onChange={() => updateHost({ allowedClientIds: draft.host.allowedClientIds.filter((value) => value !== id) })} /><span>{id} · 客户端已撤销或不存在，请移除</span></label>
                  ))}
                  {!clients.length ? <small>请先在 MCP 管理中创建客户端授权。</small> : null}
                </div>
                <small>客户端还必须拥有 run-scripts 权限。保存后重新获取 MCP 工具列表；不支持列表刷新的客户端需要重连。</small>
              </div>
              <label>运行参数（JSON）<textarea aria-label="脚本运行参数" disabled={loading || busy} value={parameters} onChange={(event) => setParameters(event.target.value)} spellCheck={false} /></label>
              {result ? <section className="host-script-result" aria-label="脚本运行结果"><strong>退出码：{result.exitCode ?? "未正常退出"}</strong><pre>{result.stdout}</pre>{result.stderr ? <pre className="error">{result.stderr}</pre> : null}</section> : null}
              </div>
              <footer className="custom-script-actions">
                <div className="custom-script-run">
                  {runId ? <button type="button" onClick={() => void cancelRun()}>停止运行</button> : null}
                  <button type="button" title={hasUnsavedChanges ? "保存后运行" : "运行脚本"} aria-label="运行自定义脚本" disabled={!selectedScript || loading || busy || hasUnsavedChanges} onClick={() => void runScript()}><Play size={14} /></button>
                </div>
                <span role={error ? "alert" : undefined}>{error}</span>
                <button type="button" className="danger" title="删除脚本" aria-label="删除自定义脚本" disabled={!selectedScript || loading || busy} onClick={() => void deleteScript()}><Trash2 size={14} /></button>
                <button type="button" className="primary" title="保存脚本" aria-label="保存自定义脚本" disabled={loading || busy} onClick={() => void saveScript()}><Save size={14} /></button>
              </footer>
            </section>
          ) : (
            <section className="custom-script-editor custom-script-editor-empty">
              {error ? <p role="alert">{error}</p> : null}
              <button type="button" disabled={loading || busy} onClick={createScript}><Plus size={14} /><span>添加脚本</span></button>
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
