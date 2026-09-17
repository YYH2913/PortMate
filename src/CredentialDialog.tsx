import { t, useLocale } from "./i18n";
import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { Eye, EyeOff, KeyRound, LoaderCircle, Unlock, X } from "lucide-react";
import { selectedSshOneKey } from "./one-key-login-state";
import { formatPortableVaultError } from "./portable-vault-error";
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
  strongholdStatus?: "unlocked" | "locked" | "not-created" | "unknown";
};

export default function CredentialDialog({
  request,
  onCancel,
  onSubmit,
  onOpenStronghold,
  onUnlockStronghold,
}: {
  request: CredentialPromptState;
  onCancel: () => void;
  onSubmit: (credentials: ConnectionCredentials) => void;
  onOpenStronghold?: () => void;
  onUnlockStronghold?: (password: string) => Promise<void>;
}) {
  useLocale();
  const [username, setUsername] = useState(request.initialUsername);
  const [password, setPassword] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [oneKeyId, setOneKeyId] = useState("");
  const [savePassword, setSavePassword] = useState(false);
  const [savePassphrase, setSavePassphrase] = useState(false);
  const [vaultPassword, setVaultPassword] = useState("");
  const [vaultUnlockBusy, setVaultUnlockBusy] = useState(false);
  const [vaultUnlockError, setVaultUnlockError] = useState("");
  const [showVaultPassword, setShowVaultPassword] = useState(false);
  const usernameRef = useRef<HTMLInputElement | null>(null);
  const vaultPasswordRef = useRef<HTMLInputElement | null>(null);
  const selectedOneKey = selectedSshOneKey(request.oneKeys, oneKeyId);
  const canSaveToStronghold = request.strongholdStatus === "unlocked";
  const canUnlockInline = Boolean(onUnlockStronghold) && request.strongholdStatus === "locked" && !selectedOneKey;

  useEffect(() => {
    usernameRef.current?.focus();
    usernameRef.current?.select();
  }, []);

  useEffect(() => {
    if (request.strongholdStatus === "unlocked") {
      setVaultPassword("");
      setVaultUnlockError("");
      setShowVaultPassword(false);
    }
  }, [request.strongholdStatus]);

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
      savePassword: !selectedOneKey && canSaveToStronghold && request.needsPassword && (forceSave || savePassword),
      savePassphrase: !selectedOneKey && canSaveToStronghold && request.hasIdentityFiles && (forceSave || savePassphrase),
    });
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    submitCredentials(false);
  }

  async function unlockVault() {
    if (!onUnlockStronghold || !vaultPassword || vaultUnlockBusy) return;
    setVaultUnlockBusy(true);
    setVaultUnlockError("");
    try {
      await onUnlockStronghold(vaultPassword);
      setVaultPassword("");
    } catch (error) {
      setVaultUnlockError(formatPortableVaultError(error));
      window.requestAnimationFrame(() => {
        vaultPasswordRef.current?.focus();
        vaultPasswordRef.current?.select();
      });
    } finally {
      setVaultUnlockBusy(false);
    }
  }

  return (
    <div className="dialog-backdrop credential-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) onCancel();
    }}>
      <form className="wind-dialog credential-dialog" onSubmit={submit}>
        <header className="dialog-title credential-title">
          <span className="app-icon" />
          <div>
            <strong>{t("ssh-connection")}</strong>
            <small>{request.target}</small>
          </div>
          <button type="button" onClick={onCancel}><X size={20} /></button>
        </header>
        <section className="credential-content">
          <label className="credential-field">
            <span>OneKey</span>
            <select value={oneKeyId} onChange={(event) => selectOneKey(event.target.value)} disabled={!request.oneKeys.length}>
              <option value="">{request.oneKeys.length ? t("manual-entry") : t("no-onekey-bound")}</option>
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
                  selectedOneKey.hasPassword ? t("password") : "",
                  selectedOneKey.hasPassphrase ? t("private-key-passphrase") : "",
                  selectedOneKey.identity ? t("public-key-identity", [selectedOneKey.identity.label]) : "",
                ].filter(Boolean).join(" / ")}</small>
              </span>
            </div>
          ) : null}
          <label className="credential-field">
            <span>{t("username")}</span>
            <input ref={usernameRef} value={username} onChange={(event) => setUsername(event.target.value)} autoComplete="username" disabled={Boolean(selectedOneKey)} />
          </label>
          {request.needsPassword ? (
            <label className="credential-field">
              <span>{selectedOneKey ? t("onekey-password") : request.hasSavedPassword ? t("login-password-saved") : t("login-password")}</span>
              <input value={password} onChange={(event) => setPassword(event.target.value)} type="password" autoComplete="current-password" disabled={Boolean(selectedOneKey)} placeholder={selectedOneKey ? selectedOneKey.hasPassword ? t("securely-saved") : t("not-saved") : ""} />
            </label>
          ) : null}
          {request.needsPassword && !selectedOneKey ? (
            <label className="credential-check">
              <input type="checkbox" checked={savePassword} onChange={(event) => setSavePassword(event.target.checked)} disabled={!password || !canSaveToStronghold} />
              <span>{t("save-login-password-to-stronghold")}{request.strongholdStatus === "unlocked" ? "" : t("unlock-first")}</span>
            </label>
          ) : null}
          {request.hasIdentityFiles ? (
            <label className="credential-field">
              <span>{selectedOneKey ? t("onekey-passphrase") : request.hasSavedPassphrase ? t("private-key-passphrase-saved") : t("private-key-passphrase")}</span>
              <input value={passphrase} onChange={(event) => setPassphrase(event.target.value)} type="password" autoComplete="off" disabled={Boolean(selectedOneKey)} placeholder={selectedOneKey ? selectedOneKey.hasPassphrase ? t("securely-saved") : t("not-saved") : t("leave-blank-if-none")} />
            </label>
          ) : null}
          {request.hasIdentityFiles && !selectedOneKey ? (
            <label className="credential-check">
              <input type="checkbox" checked={savePassphrase} onChange={(event) => setSavePassphrase(event.target.checked)} disabled={!passphrase || !canSaveToStronghold} />
              <span>{t("save-private-key-passphrase-to-stronghold")}{request.strongholdStatus === "unlocked" ? "" : t("unlock-first")}</span>
            </label>
          ) : null}
          {canUnlockInline ? (
            <div className="credential-vault-unlock" role="group" aria-labelledby="credential-vault-unlock-title">
              <div className="credential-vault-unlock-copy">
                <KeyRound size={14} />
                <div>
                  <strong id="credential-vault-unlock-title">{t("unlock-stronghold")}</strong>
                  <span>{t("enter-the-stronghold-master-password-to-use-saved")}</span>
                </div>
              </div>
              <label className="credential-field">
                <span>{t("stronghold-master-password")}</span>
                <span className="credential-password-row">
                  <input
                    ref={vaultPasswordRef}
                    value={vaultPassword}
                    type={showVaultPassword ? "text" : "password"}
                    autoComplete="off"
                    disabled={vaultUnlockBusy}
                    aria-invalid={vaultUnlockError ? true : undefined}
                    onChange={(event) => {
                      setVaultPassword(event.target.value);
                      if (vaultUnlockError) setVaultUnlockError("");
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        void unlockVault();
                      }
                    }}
                  />
                  <button
                    type="button"
                    className="credential-password-toggle"
                    title={showVaultPassword ? t("hide-master-password") : t("show-master-password")}
                    aria-label={showVaultPassword ? t("hide-master-password") : t("show-master-password")}
                    onClick={() => setShowVaultPassword((current) => !current)}
                  >
                    {showVaultPassword ? <EyeOff size={14} /> : <Eye size={14} />}
                  </button>
                </span>
              </label>
              <div className="credential-vault-unlock-actions">
                <button type="button" className="primary" disabled={vaultUnlockBusy || !vaultPassword} aria-busy={vaultUnlockBusy} onClick={() => void unlockVault()}>
                  {vaultUnlockBusy ? <LoaderCircle size={14} /> : <Unlock size={14} />}
                  {vaultUnlockBusy ? t("verifying") : t("unlock-stronghold")}
                </button>
                {onOpenStronghold ? <button type="button" onClick={onOpenStronghold}>{t("open-stronghold")}</button> : null}
              </div>
              <small>{t("unlocking-stronghold-may-take-a-few-seconds")}</small>
              {vaultUnlockError ? <p className="credential-vault-unlock-error" role="alert">{vaultUnlockError}</p> : null}
            </div>
          ) : !selectedOneKey && request.strongholdStatus && request.strongholdStatus !== "unlocked" ? (
            <div className="credential-vault-hint" role="note">
              <KeyRound size={14} />
              <span>{request.strongholdStatus === "not-created"
                ? t("stronghold-has-not-been-created-create-the-vault-before")
                : request.strongholdStatus === "locked"
                  ? t("stronghold-is-locked-unlock-the-vault-before-saving-passwords")
                  : t("reading-stronghold-status-wait-before-saving-credentials")}</span>
              {onOpenStronghold ? <button type="button" onClick={onOpenStronghold}>{t("open-stronghold")}</button> : null}
            </div>
          ) : null}
          <div className="credential-meta">
            <span>{t("this-connection")}</span>
            <span>{request.authOrder.join(" / ")}</span>
          </div>
        </section>
        <footer className="credential-actions">
          <button type="button" onClick={onCancel}>{t("cancel")}</button>
          <button type="submit">{t("connect")}</button>
          {!selectedOneKey && canSaveToStronghold && (Boolean(password) || Boolean(passphrase)) ? (
            <button type="button" className="primary" onClick={(event) => {
              event.preventDefault();
              submitCredentials(true);
            }}>
              <KeyRound size={14} />{t("save-and-connect")}</button>
          ) : null}
        </footer>
      </form>
    </div>
  );
}
