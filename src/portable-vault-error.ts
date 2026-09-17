import { t, localizeDiagnostic } from "./i18n";

function errorText(error: unknown): string {
  if (typeof error === "string") return error.trim();
  if (error instanceof Error) return error.message.trim();
  try {
    return JSON.stringify(error);
  } catch {
    return String(error ?? "");
  }
}

export function formatPortableVaultError(error: unknown): string {
  const text = errorText(error);
  if (!text) return t("master-password-verification-failed");
  if (/尚未创建|has not been created/i.test(text)) {
    return t("stronghold-has-not-been-created-use-the-setup-below");
  }
  if (/已存在，请使用解锁|already exists/i.test(text)) {
    return t("stronghold-already-exists-use-unlock");
  }
  if (/salt/i.test(text) && /缺失|missing|不存在|not found/i.test(text)) {
    return t("stronghold-vault-files-are-incomplete");
  }
  if (/至少需要 8|at least 8/i.test(text)) {
    return t("the-new-stronghold-master-password-must-contain-at-least");
  }
  if (/必须与当前密码不同|must differ/i.test(text)) {
    return t("the-new-portable-vault-master-password-must-differ-from");
  }
  if (/portable vault 已解锁$|already unlocked/i.test(text)) {
    return t("stronghold-is-already-unlocked");
  }
  if (/已锁定，请先解锁|please unlock/i.test(text)) {
    return t("stronghold-is-locked-unlock-the-vault-before-saving-passwords");
  }
  if (/主密码不正确|解锁失败|当前主密码验证失败|client 加载失败|incorrect|invalid mac|authentication failed|wrong password|verification failed/i.test(text)) {
    return t("the-stronghold-master-password-is-incorrect");
  }
  return localizeDiagnostic(text);
}
