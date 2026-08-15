import { useEffect, useMemo, useState } from "react";
import { Braces, Play, Plus, Save, Trash2, X } from "lucide-react";
import { invokeBackend } from "./api";
import {
  customScriptDraft,
  customScriptDraftMatches,
  MAX_CUSTOM_SCRIPTS,
  MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS,
  MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS,
  MAX_CUSTOM_SCRIPT_NAME_CHARACTERS,
  newCustomScriptDraft,
  normalizeCustomScriptDraft,
  runnableCustomScriptSessions,
  scriptAllowsSession,
  validateCustomScriptDraft,
} from "./custom-script-state";
import type { CustomScript, RunCustomScriptRequest, SaveCustomScriptRequest, SaveCustomScriptResponse, SessionEvent, SessionSummary } from "./types";

export default function CustomScriptDialog({
  sessions,
  activeId,
  onClose,
  onNotice,
}: {
  sessions: SessionSummary[];
  activeId: string;
  onClose: () => void;
  onNotice: (message: string) => void;
}) {
  const [scripts, setScripts] = useState<CustomScript[]>([]);
  const [draft, setDraft] = useState<SaveCustomScriptRequest | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [runSessionId, setRunSessionId] = useState(activeId);
  const runnableSessions = useMemo(() => draft ? runnableCustomScriptSessions(draft, sessions) : [], [draft, sessions]);
  const selectedScript = draft?.id ? scripts.find((script) => script.id === draft.id) : null;
  const hasUnsavedChanges = Boolean(draft && (!selectedScript || !customScriptDraftMatches(draft, selectedScript)));

  useEffect(() => {
    let cancelled = false;
    void invokeBackend<CustomScript[]>("list_custom_scripts", {})
      .then((items) => {
        if (cancelled) return;
        setScripts(items);
        setDraft(items[0] ? customScriptDraft(items[0]) : null);
      })
      .catch((reason) => !cancelled && setError(formatError(reason)))
      .finally(() => !cancelled && setLoading(false));
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    if (runnableSessions.some((session) => session.profile.id === runSessionId)) return;
    const preferred = runnableSessions.find((session) => session.profile.id === activeId) ?? runnableSessions[0];
    setRunSessionId(preferred?.profile.id ?? "");
  }, [activeId, runSessionId, runnableSessions]);

  function selectScript(script: CustomScript) {
    if (busy) return;
    setDraft(customScriptDraft(script));
    setError("");
  }

  function createScript() {
    if (scripts.length >= MAX_CUSTOM_SCRIPTS) {
      setError(`自定义脚本最多保存 ${MAX_CUSTOM_SCRIPTS} 条。`);
      return;
    }
    setDraft(newCustomScriptDraft(activeId));
    setError("");
  }

  function updateDraft(patch: Partial<SaveCustomScriptRequest>) {
    setDraft((current) => current ? { ...current, ...patch } : current);
    setError("");
  }

  function toggleSession(sessionId: string) {
    if (!draft) return;
    updateDraft({
      allowedSessionIds: draft.allowedSessionIds.includes(sessionId)
        ? draft.allowedSessionIds.filter((id) => id !== sessionId)
        : [...draft.allowedSessionIds, sessionId],
    });
  }

  async function saveScript() {
    if (!draft || busy) return;
    const normalized = normalizeCustomScriptDraft(draft);
    const validation = validateCustomScriptDraft(normalized);
    if (validation) {
      setError(validation);
      return;
    }
    setBusy(true);
    setError("");
    try {
      const response = await invokeBackend<SaveCustomScriptResponse>("save_custom_script", { request: normalized });
      const saved = response.scripts.find((script) => script.id === response.savedId);
      if (!saved) throw new Error("保存响应没有包含已提交的自定义脚本，请重新打开后检查。");
      setScripts(response.scripts);
      setDraft(customScriptDraft(saved));
    } catch (reason) {
      setError(formatError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function deleteScript() {
    if (!selectedScript || busy) return;
    setBusy(true);
    setError("");
    try {
      const items = await invokeBackend<CustomScript[]>("delete_custom_script", {
        request: { id: selectedScript.id, expectedUpdatedAt: selectedScript.updatedAt },
      });
      setScripts(items);
      setDraft(items[0] ? customScriptDraft(items[0]) : null);
    } catch (reason) {
      setError(formatError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function runScript() {
    if (!selectedScript || !runSessionId || busy || hasUnsavedChanges) return;
    setBusy(true);
    setError("");
    try {
      const request: RunCustomScriptRequest = {
        scriptId: selectedScript.id,
        sessionId: runSessionId,
        expectedUpdatedAt: selectedScript.updatedAt,
      };
      await invokeBackend<SessionEvent>("run_custom_script", {
        request,
      });
      const session = sessions.find((item) => item.profile.id === runSessionId);
      onNotice(`已在 ${session?.profile.name ?? runSessionId} 运行 ${selectedScript.name}`);
    } catch (reason) {
      setError(formatError(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog custom-script-dialog" role="dialog" aria-modal="true" aria-labelledby="custom-script-title">
        <header className="dialog-title">
          <Braces size={18} />
          <strong id="custom-script-title">自定义脚本</strong>
          <button type="button" title="关闭" aria-label="关闭自定义脚本" onClick={onClose}><X size={20} /></button>
        </header>
        <div className="custom-script-content">
          <aside className="custom-script-list">
            <header>
              <strong>脚本</strong>
              <span>{scripts.length}/{MAX_CUSTOM_SCRIPTS}</span>
              <button type="button" title="添加脚本" aria-label="添加自定义脚本" disabled={busy || scripts.length >= MAX_CUSTOM_SCRIPTS} onClick={createScript}><Plus size={14} /></button>
            </header>
            <div role="listbox" aria-label="自定义脚本列表">
              {draft && !draft.id ? (
                <button type="button" className="active" role="option" aria-selected="true"><Braces size={13} /><span>新脚本</span></button>
              ) : null}
              {scripts.map((script) => (
                <button key={script.id} type="button" role="option" aria-selected={draft?.id === script.id} className={draft?.id === script.id ? "active" : ""} onClick={() => selectScript(script)}>
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
              <div className="custom-script-meta-fields">
                <label><span>名称</span><input aria-label="脚本名称" maxLength={MAX_CUSTOM_SCRIPT_NAME_CHARACTERS} value={draft.name} onChange={(event) => updateDraft({ name: event.target.value })} /></label>
                <label><span>说明</span><input aria-label="脚本说明" maxLength={MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS} value={draft.description} onChange={(event) => updateDraft({ description: event.target.value })} /></label>
              </div>
              <label className="custom-script-body">
                <span>脚本</span>
                <textarea aria-label="脚本正文" spellCheck={false} maxLength={MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS} value={draft.content} onChange={(event) => updateDraft({ content: event.target.value.replace(/\r\n?/g, "\n") })} />
              </label>
              <div className="custom-script-boundary">
                <div className="custom-script-toggles">
                  <label><input type="checkbox" checked={draft.allowAllSessions} onChange={(event) => updateDraft({ allowAllSessions: event.target.checked, allowedSessionIds: event.target.checked ? [] : activeId ? [activeId] : [] })} /><span>全部会话</span></label>
                  <label><input type="checkbox" checked={draft.mcpEnabled} onChange={(event) => updateDraft({ mcpEnabled: event.target.checked })} /><span>开放给 MCP</span></label>
                </div>
                {!draft.allowAllSessions ? (
                  <div className="custom-script-session-list" aria-label="脚本允许会话">
                    {sessions.map((session) => <label key={session.profile.id}><input type="checkbox" checked={draft.allowedSessionIds.includes(session.profile.id)} onChange={() => toggleSession(session.profile.id)} /><span>{session.profile.name}</span></label>)}
                  </div>
                ) : null}
              </div>
              <footer className="custom-script-actions">
                <div className="custom-script-run">
                  <select aria-label="运行脚本的会话" value={runSessionId} disabled={!selectedScript || !runnableSessions.length || busy} onChange={(event) => setRunSessionId(event.target.value)}>
                    {!runnableSessions.length ? <option value="">没有可运行会话</option> : null}
                    {runnableSessions.map((session) => <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>)}
                  </select>
                  <button type="button" title={hasUnsavedChanges ? "保存后运行" : "运行脚本"} aria-label="运行自定义脚本" disabled={!selectedScript || !runSessionId || busy || hasUnsavedChanges || !scriptAllowsSession(draft, runSessionId)} onClick={() => void runScript()}><Play size={14} /></button>
                </div>
                <span role={error ? "alert" : undefined}>{error}</span>
                <button type="button" className="danger" title="删除脚本" aria-label="删除自定义脚本" disabled={!selectedScript || busy} onClick={() => void deleteScript()}><Trash2 size={14} /></button>
                <button type="button" className="primary" title="保存脚本" aria-label="保存自定义脚本" disabled={busy} onClick={() => void saveScript()}><Save size={14} /></button>
              </footer>
            </section>
          ) : (
            <section className="custom-script-editor custom-script-editor-empty">
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
