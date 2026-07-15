import { useMemo, useState } from "react";
import type { FormEvent } from "react";
import { KeyRound, Plus, Save, Send, Trash2, UserRound, X } from "lucide-react";
import { invokeBackend } from "./api";
import {
  oneKeyIdentityCandidates,
  oneKeyIdentitySelectionKey,
  oneKeyIdentityUpdate,
  selectionFromOneKeyIdentity,
} from "./one-key-identity-state";
import type { OneKeyIdentitySelection } from "./one-key-identity-state";
import type {
  OneKeyIdentitySummary,
  OneKeyKind,
  OneKeyMutationResponse,
  OneKeySecretUpdate,
  OneKeySummary,
  SaveOneKeyRequest,
  SessionSummary,
} from "./types";

const MAX_ONE_KEYS = 64;

type SecretStorageChoice = "auto" | "native" | "portable";

type OneKeyDraft = {
  id: string | null;
  label: string;
  kind: OneKeyKind;
  username: string;
  password: string;
  passphrase: string;
  clearPassword: boolean;
  clearPassphrase: boolean;
  hasPassword: boolean;
  hasPassphrase: boolean;
  currentIdentity: OneKeyIdentitySummary | null;
  identitySelection: OneKeyIdentitySelection | null;
  storage: SecretStorageChoice;
  sessionIds: string[];
};

function emptyDraft(): OneKeyDraft {
  return {
    id: null,
    label: "",
    kind: "account",
    username: "",
    password: "",
    passphrase: "",
    clearPassword: false,
    clearPassphrase: false,
    hasPassword: false,
    hasPassphrase: false,
    currentIdentity: null,
    identitySelection: null,
    storage: "auto",
    sessionIds: [],
  };
}

function draftFromItem(item: OneKeySummary): OneKeyDraft {
  return {
    id: item.id,
    label: item.label,
    kind: item.kind,
    username: item.username,
    password: "",
    passphrase: "",
    clearPassword: false,
    clearPassphrase: false,
    hasPassword: item.hasPassword,
    hasPassphrase: item.hasPassphrase,
    currentIdentity: item.identity ?? null,
    identitySelection: selectionFromOneKeyIdentity(item.identity),
    storage: "auto",
    sessionIds: [...item.sessionIds],
  };
}

function secretUpdate(
  value: string,
  clear: boolean,
  storage: SecretStorageChoice,
): OneKeySecretUpdate {
  if (value) {
    return {
      action: "set",
      secret: value,
      storage: storage === "auto" ? null : storage,
    };
  }
  return clear ? { action: "clear" } : { action: "preserve" };
}

export default function OneKeyDialog({
  oneKeys,
  sessions,
  activeId,
  onChange,
  onClose,
}: {
  oneKeys: OneKeySummary[];
  sessions: SessionSummary[];
  activeId: string;
  onChange: (items: OneKeySummary[]) => void;
  onClose: () => void;
}) {
  const [items, setItems] = useState(() => oneKeys.map((item) => ({ ...item, sessionIds: [...item.sessionIds] })));
  const [selectedId, setSelectedId] = useState(oneKeys[0]?.id ?? "");
  const [draft, setDraft] = useState<OneKeyDraft>(() => oneKeys[0] ? draftFromItem(oneKeys[0]) : emptyDraft());
  const [busy, setBusy] = useState<"save" | "delete" | "send" | null>(null);
  const [feedback, setFeedback] = useState<{ kind: "error" | "status"; text: string } | null>(null);
  const active = sessions.find((session) => session.profile.id === activeId);
  const compatibleSessions = useMemo(
    () => sessions.filter((session) => draft.kind === "account" || session.profile.kind === "ssh" || session.profile.kind === "tmux"),
    [draft.kind, sessions],
  );
  const identityCandidates = useMemo(
    () => oneKeyIdentityCandidates(sessions, draft.sessionIds),
    [draft.sessionIds, sessions],
  );
  const currentIdentitySelection = selectionFromOneKeyIdentity(draft.currentIdentity);
  const showSavedIdentityOption = Boolean(
    currentIdentitySelection
      && draft.sessionIds.includes(currentIdentitySelection.sourceProfileId)
      && !identityCandidates.some((item) => (
        item.sourceProfileId === currentIdentitySelection.sourceProfileId
        && item.identity.id === currentIdentitySelection.identityId
      )),
  );

  function selectItem(item: OneKeySummary) {
    setSelectedId(item.id);
    setDraft(draftFromItem(item));
    setFeedback(null);
  }

  function addItem() {
    if (items.length >= MAX_ONE_KEYS) {
      setFeedback({ kind: "error", text: `OneKey 最多保存 ${MAX_ONE_KEYS} 条。` });
      return;
    }
    setSelectedId("");
    setDraft(emptyDraft());
    setFeedback(null);
  }

  function toggleSession(sessionId: string) {
    setDraft((current) => {
      const removing = current.sessionIds.includes(sessionId);
      return {
        ...current,
        sessionIds: removing
          ? current.sessionIds.filter((id) => id !== sessionId)
          : [...current.sessionIds, sessionId],
        identitySelection: removing && current.identitySelection?.sourceProfileId === sessionId
          ? null
          : current.identitySelection,
      };
    });
    setFeedback(null);
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    if (!draft.label.trim() || !draft.username.trim()) {
      setFeedback({ kind: "error", text: "名称和用户名不能为空。" });
      return;
    }
    if (!draft.sessionIds.length) {
      setFeedback({ kind: "error", text: "至少绑定一个会话。" });
      return;
    }
    const hasPasswordAfterSave = Boolean(draft.password) || (draft.hasPassword && !draft.clearPassword);
    const hasPassphraseAfterSave = draft.kind === "ssh" && (Boolean(draft.passphrase) || (draft.hasPassphrase && !draft.clearPassphrase));
    const hasIdentityAfterSave = draft.kind === "ssh" && draft.identitySelection !== null;
    if (!hasPasswordAfterSave && !hasPassphraseAfterSave && !hasIdentityAfterSave) {
      setFeedback({ kind: "error", text: "至少保存密码、私钥口令或公钥身份。" });
      return;
    }
    const request: SaveOneKeyRequest = {
      id: draft.id,
      label: draft.label,
      kind: draft.kind,
      username: draft.username,
      passwordUpdate: secretUpdate(draft.password, draft.clearPassword, draft.storage),
      passphraseUpdate: draft.kind === "ssh"
        ? secretUpdate(draft.passphrase, draft.clearPassphrase, draft.storage)
        : { action: "clear" },
      identityUpdate: oneKeyIdentityUpdate(draft.kind, draft.currentIdentity, draft.identitySelection),
      sessionIds: draft.sessionIds,
    };
    setBusy("save");
    setFeedback(null);
    try {
      const response = await invokeBackend<OneKeyMutationResponse>("save_one_key", { request });
      setItems(response.items);
      onChange(response.items);
      const saved = response.items.find((item) => item.id === response.savedId);
      if (saved) {
        setSelectedId(saved.id);
        setDraft(draftFromItem(saved));
      }
      setFeedback({ kind: "status", text: "OneKey 已保存。" });
    } catch (error) {
      setFeedback({ kind: "error", text: String(error) });
    } finally {
      setBusy(null);
    }
  }

  async function remove() {
    if (!draft.id || !window.confirm(`删除 OneKey “${draft.label}”？`)) return;
    setBusy("delete");
    setFeedback(null);
    try {
      const next = await invokeBackend<OneKeySummary[]>("delete_one_key", { request: { id: draft.id } });
      setItems(next);
      onChange(next);
      const replacement = next[0];
      setSelectedId(replacement?.id ?? "");
      setDraft(replacement ? draftFromItem(replacement) : emptyDraft());
      setFeedback({ kind: "status", text: "OneKey 已删除。" });
    } catch (error) {
      setFeedback({ kind: "error", text: String(error) });
    } finally {
      setBusy(null);
    }
  }

  async function sendField(field: "username" | "password" | "passphrase") {
    if (!draft.id || !active) return;
    setBusy("send");
    setFeedback(null);
    try {
      await invokeBackend("send_one_key", {
        request: { id: draft.id, sessionId: active.profile.id, field },
      });
      setFeedback({ kind: "status", text: `${field === "username" ? "用户名" : field === "password" ? "密码" : "私钥口令"}已发送。` });
    } catch (error) {
      setFeedback({ kind: "error", text: String(error) });
    } finally {
      setBusy(null);
    }
  }

  const canSend = Boolean(
    draft.id
      && active
      && active.runtime.status === "connected"
      && draft.sessionIds.includes(active.profile.id),
  );

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="wind-dialog utility-dialog one-key-dialog" role="dialog" aria-modal="true" aria-labelledby="one-key-dialog-title" onSubmit={(event) => void save(event)}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong id="one-key-dialog-title">OneKey 管理器</strong>
          <button type="button" title="关闭" aria-label="关闭 OneKey 管理器" onClick={onClose}><X size={20} /></button>
        </header>
        <section className="one-key-content">
          <aside className="one-key-list">
            <header>
              <strong>OneKeys</strong>
              <span>{items.length}/{MAX_ONE_KEYS}</span>
              <button type="button" title="添加 OneKey" aria-label="添加 OneKey" onClick={addItem} disabled={items.length >= MAX_ONE_KEYS}><Plus size={14} /></button>
            </header>
            <div role="listbox" aria-label="OneKey 列表">
              {items.map((item) => (
                <button key={item.id} type="button" role="option" aria-selected={item.id === selectedId} className={item.id === selectedId ? "active" : ""} onClick={() => selectItem(item)}>
                  {item.kind === "ssh" ? <KeyRound size={13} /> : <UserRound size={13} />}
                  <span><strong>{item.label}</strong><small>{item.username}</small></span>
                </button>
              ))}
              {!items.length ? <div className="one-key-list-empty">没有 OneKey</div> : null}
            </div>
          </aside>
          <section className="one-key-editor">
            <div className="one-key-fields">
              <label><span>名称</span><input value={draft.label} maxLength={64} onChange={(event) => setDraft((current) => ({ ...current, label: event.target.value }))} /></label>
              <label><span>类型</span><select value={draft.kind} onChange={(event) => {
                const kind = event.target.value as OneKeyKind;
                setDraft((current) => ({
                  ...current,
                  kind,
                  identitySelection: kind === "ssh" ? current.identitySelection : null,
                  sessionIds: current.sessionIds.filter((id) => {
                    const session = sessions.find((candidate) => candidate.profile.id === id);
                    return kind === "account" || session?.profile.kind === "ssh" || session?.profile.kind === "tmux";
                  }),
                }));
              }}><option value="account">Account</option><option value="ssh">SSH</option></select></label>
              <label><span>用户名</span><input value={draft.username} maxLength={256} autoComplete="username" onChange={(event) => setDraft((current) => ({ ...current, username: event.target.value }))} /></label>
              <label><span>{draft.hasPassword ? "密码（已存）" : "密码"}</span><input type="password" value={draft.password} autoComplete="new-password" placeholder={draft.hasPassword ? "留空保持原值" : ""} onChange={(event) => setDraft((current) => ({ ...current, password: event.target.value, clearPassword: false }))} /></label>
              {draft.hasPassword ? <label className="one-key-clear"><input type="checkbox" checked={draft.clearPassword} disabled={Boolean(draft.password)} onChange={(event) => setDraft((current) => ({ ...current, clearPassword: event.target.checked }))} /><span>清除已存密码</span></label> : null}
              {draft.kind === "ssh" ? <label><span>{draft.hasPassphrase ? "私钥口令（已存）" : "私钥口令"}</span><input type="password" value={draft.passphrase} autoComplete="off" placeholder={draft.hasPassphrase ? "留空保持原值" : ""} onChange={(event) => setDraft((current) => ({ ...current, passphrase: event.target.value, clearPassphrase: false }))} /></label> : null}
              {draft.kind === "ssh" && draft.hasPassphrase ? <label className="one-key-clear"><input type="checkbox" checked={draft.clearPassphrase} disabled={Boolean(draft.passphrase)} onChange={(event) => setDraft((current) => ({ ...current, clearPassphrase: event.target.checked }))} /><span>清除已存口令</span></label> : null}
              {draft.kind === "ssh" ? <label><span>公钥身份</span><select value={draft.identitySelection ? oneKeyIdentitySelectionKey(draft.identitySelection) : ""} onChange={(event) => {
                const candidate = identityCandidates.find((item) => oneKeyIdentitySelectionKey({ sourceProfileId: item.sourceProfileId, identityId: item.identity.id }) === event.target.value);
                const saved = currentIdentitySelection && oneKeyIdentitySelectionKey(currentIdentitySelection) === event.target.value
                  ? currentIdentitySelection
                  : null;
                setDraft((current) => ({
                  ...current,
                  identitySelection: candidate
                    ? { sourceProfileId: candidate.sourceProfileId, identityId: candidate.identity.id }
                    : saved,
                }));
              }}>
                <option value="">不使用</option>
                {showSavedIdentityOption && currentIdentitySelection
                  ? <option value={oneKeyIdentitySelectionKey(currentIdentitySelection)}>{draft.currentIdentity?.label ?? currentIdentitySelection.identityId} · 已保存</option>
                  : null}
                {identityCandidates.map((item) => {
                  const selection = { sourceProfileId: item.sourceProfileId, identityId: item.identity.id };
                  return <option key={oneKeyIdentitySelectionKey(selection)} value={oneKeyIdentitySelectionKey(selection)}>{item.identity.label} · {item.sourceProfileName}</option>;
                })}
              </select></label> : null}
              <label><span>新 Secret 存储</span><select value={draft.storage} onChange={(event) => setDraft((current) => ({ ...current, storage: event.target.value as SecretStorageChoice }))}><option value="auto">自动</option><option value="native">系统密钥库</option><option value="portable">Portable Stronghold</option></select></label>
            </div>
            <section className="one-key-sessions">
              <header><strong>绑定会话</strong><span>{draft.sessionIds.length}</span></header>
              <div>
                {compatibleSessions.map((session) => <label key={session.profile.id}><input type="checkbox" checked={draft.sessionIds.includes(session.profile.id)} onChange={() => toggleSession(session.profile.id)} /><span><strong>{session.profile.name}</strong><small>{session.profile.kind}</small></span></label>)}
                {!compatibleSessions.length ? <div>没有兼容会话</div> : null}
              </div>
            </section>
            <div className="one-key-editor-actions">
              <button type="button" title="发送用户名" onClick={() => void sendField("username")} disabled={!canSend || busy !== null}><UserRound size={14} /><span>用户名</span></button>
              <button type="button" title="发送密码" onClick={() => void sendField("password")} disabled={!canSend || !draft.hasPassword || busy !== null}><Send size={14} /><span>密码</span></button>
              {draft.kind === "ssh" ? <button type="button" title="发送私钥口令" onClick={() => void sendField("passphrase")} disabled={!canSend || !draft.hasPassphrase || busy !== null}><KeyRound size={14} /><span>口令</span></button> : null}
              <span />
              {draft.id ? <button type="button" className="danger" title="删除 OneKey" aria-label="删除 OneKey" onClick={() => void remove()} disabled={busy !== null}><Trash2 size={14} /></button> : null}
              <button type="submit" className="primary" title="保存 OneKey" disabled={busy !== null}><Save size={14} /><span>保存</span></button>
            </div>
          </section>
        </section>
        <footer className="utility-actions one-key-dialog-actions">
          <span className={feedback?.kind ?? ""} role={feedback?.kind === "error" ? "alert" : "status"}>{feedback?.text ?? ""}</span>
          <span>{active ? `当前：${active.profile.name}` : "未选择会话"}</span>
          <button type="button" onClick={onClose}>关闭</button>
        </footer>
      </form>
    </div>
  );
}
