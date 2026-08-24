import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { KeyRound, X } from "lucide-react";
import { selectedSshOneKey } from "./one-key-login-state";
import type { AuthMethod, OneKeySummary } from "./types";

export type ConnectionCredentials = {
  username: string | null;
  password: string | null;
  passphrase: string | null;
  oneKeyId: string | null;
  savePassword: boolean;
  savePassphrase: boolean;
};

export type CredentialPromptState = {
  requestId: number;
  target: string;
  initialUsername: string;
  oneKeys: OneKeySummary[];
  hasIdentityFiles: boolean;
  hasSavedPassword: boolean;
  hasSavedPassphrase: boolean;
  needsPassword: boolean;
  authOrder: AuthMethod[];
};

export default function CredentialDialog({
  request,
  onCancel,
  onSubmit,
}: {
  request: CredentialPromptState;
  onCancel: () => void;
  onSubmit: (credentials: ConnectionCredentials) => void;
}) {
  const [username, setUsername] = useState(request.initialUsername);
  const [password, setPassword] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [oneKeyId, setOneKeyId] = useState("");
  const [savePassword, setSavePassword] = useState(false);
  const [savePassphrase, setSavePassphrase] = useState(false);
  const usernameRef = useRef<HTMLInputElement | null>(null);
  const selectedOneKey = selectedSshOneKey(request.oneKeys, oneKeyId);

  useEffect(() => {
    usernameRef.current?.focus();
    usernameRef.current?.select();
  }, []);

  function selectOneKey(nextOneKeyId: string) {
    const oneKey = selectedSshOneKey(request.oneKeys, nextOneKeyId);
    setOneKeyId(oneKey?.id ?? "");
    if (oneKey) {
      setUsername(oneKey.username);
      setPassword("");
      setPassphrase("");
      setSavePassword(false);
      setSavePassphrase(false);
    }
  }

  function submitCredentials(forceSave = false) {
    const nextUsername = (selectedOneKey?.username ?? username).trim();
    if (!nextUsername) {
      usernameRef.current?.focus();
      return;
    }
    onSubmit({
      username: nextUsername,
      password: !selectedOneKey && request.needsPassword ? password : null,
      passphrase: !selectedOneKey && request.hasIdentityFiles ? passphrase : null,
      oneKeyId: selectedOneKey?.id ?? null,
      savePassword: !selectedOneKey && request.needsPassword && (forceSave || savePassword),
      savePassphrase: !selectedOneKey && request.hasIdentityFiles && (forceSave || savePassphrase),
    });
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    submitCredentials(false);
  }

  return (
    <div className="dialog-backdrop credential-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) onCancel();
    }}>
      <form className="wind-dialog credential-dialog" onSubmit={submit}>
        <header className="dialog-title credential-title">
          <span className="app-icon" />
          <div>
            <strong>SSH 连接</strong>
            <small>{request.target}</small>
          </div>
          <button type="button" onClick={onCancel}><X size={20} /></button>
        </header>
        <section className="credential-content">
          <label className="credential-field">
            <span>OneKey</span>
            <select value={oneKeyId} onChange={(event) => selectOneKey(event.target.value)} disabled={!request.oneKeys.length}>
              <option value="">{request.oneKeys.length ? "手动输入" : "没有绑定 OneKey"}</option>
              {request.oneKeys.map((oneKey) => (
                <option key={oneKey.id} value={oneKey.id}>{oneKey.label}</option>
              ))}
            </select>
          </label>
          {selectedOneKey ? (
            <div className="credential-one-key-meta">
              <KeyRound size={14} />
              <span>
                <strong>{selectedOneKey.label}</strong>
                <small>{[
                  selectedOneKey.hasPassword ? "密码" : "",
                  selectedOneKey.hasPassphrase ? "私钥口令" : "",
                  selectedOneKey.identity ? `公钥身份 · ${selectedOneKey.identity.label}` : "",
                ].filter(Boolean).join(" / ")}</small>
              </span>
            </div>
          ) : null}
          <label className="credential-field">
            <span>用户名</span>
            <input ref={usernameRef} value={username} onChange={(event) => setUsername(event.target.value)} autoComplete="username" disabled={Boolean(selectedOneKey)} />
          </label>
          {request.needsPassword ? (
            <label className="credential-field">
              <span>{selectedOneKey ? "OneKey 密码" : request.hasSavedPassword ? "登录密码(已存)" : "登录密码"}</span>
              <input value={password} onChange={(event) => setPassword(event.target.value)} type="password" autoComplete="current-password" disabled={Boolean(selectedOneKey)} placeholder={selectedOneKey ? selectedOneKey.hasPassword ? "已安全保存" : "未保存" : ""} />
            </label>
          ) : null}
          {request.needsPassword && !selectedOneKey ? (
            <label className="credential-check">
              <input type="checkbox" checked={savePassword} onChange={(event) => setSavePassword(event.target.checked)} disabled={!password} />
              <span>保存登录密码到 Stronghold（需先解锁）</span>
            </label>
          ) : null}
          {request.hasIdentityFiles ? (
            <label className="credential-field">
              <span>{selectedOneKey ? "OneKey 私钥口令" : request.hasSavedPassphrase ? "私钥口令(已存)" : "私钥口令"}</span>
              <input value={passphrase} onChange={(event) => setPassphrase(event.target.value)} type="password" autoComplete="off" disabled={Boolean(selectedOneKey)} placeholder={selectedOneKey ? selectedOneKey.hasPassphrase ? "已安全保存" : "未保存" : "没有可留空"} />
            </label>
          ) : null}
          {request.hasIdentityFiles && !selectedOneKey ? (
            <label className="credential-check">
              <input type="checkbox" checked={savePassphrase} onChange={(event) => setSavePassphrase(event.target.checked)} disabled={!passphrase} />
              <span>保存私钥口令到 Stronghold（需先解锁）</span>
            </label>
          ) : null}
          <div className="credential-meta">
            <span>本次连接</span>
            <span>{request.authOrder.join(" / ")}</span>
          </div>
        </section>
        <footer className="credential-actions">
          <button type="button" onClick={onCancel}>取消</button>
          <button type="submit">连接</button>
          {!selectedOneKey && (Boolean(password) || Boolean(passphrase)) ? (
            <button type="button" className="primary" onClick={(event) => {
              event.preventDefault();
              submitCredentials(true);
            }}>
              <KeyRound size={14} />保存并连接
            </button>
          ) : null}
        </footer>
      </form>
    </div>
  );
}
