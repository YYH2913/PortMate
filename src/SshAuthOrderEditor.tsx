import { ArrowDown, ArrowUp } from "lucide-react";
import { t, useLocale } from "./i18n";
import { moveSshAuthMethod, SSH_AUTH_METHODS, toggleSshAuthMethod } from "./ssh-connection-settings";
import type { AuthMethod } from "./types";

export default function SshAuthOrderEditor({ order, onChange }: { order: readonly AuthMethod[]; onChange: (order: AuthMethod[]) => void }) {
  useLocale();
  const methods = [...order, ...SSH_AUTH_METHODS.filter(method => !order.includes(method))];
  return <div className="ssh-auth-order" role="group" aria-label={t("ssh-auth-order")}>
    <strong>{t("ssh-auth-order")}</strong>
    <small>{t("ssh-auth-order-note")}</small>
    {methods.map(method => {
      const rank = order.indexOf(method);
      const label = method === "gssapi-with-mic" ? "GSSAPI / Kerberos" : method === "none" ? t("ssh-auth-none") : t(method);
      return <div key={method} className="ssh-auth-order-row" data-auth-method={method}>
        <label><input type="checkbox" checked={rank >= 0} disabled={rank >= 0 && order.length === 1} onChange={() => onChange(toggleSshAuthMethod(order, method))} /><span className="ssh-auth-rank">{rank >= 0 ? rank + 1 : "—"}</span>{label}</label>
        <button type="button" disabled={rank <= 0} aria-label={t("ssh-auth-move-up", [label])} onClick={() => onChange(moveSshAuthMethod(order, method, -1))}><ArrowUp size={14} /></button>
        <button type="button" disabled={rank < 0 || rank === order.length - 1} aria-label={t("ssh-auth-move-down", [label])} onClick={() => onChange(moveSshAuthMethod(order, method, 1))}><ArrowDown size={14} /></button>
      </div>;
    })}
    <small>{t("ssh-auth-gssapi-note")}</small>
  </div>;
}
