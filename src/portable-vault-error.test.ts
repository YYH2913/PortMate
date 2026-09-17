import { describe, expect, it } from "vitest";
import { setLanguagePreference } from "./i18n";
import { formatPortableVaultError } from "./portable-vault-error";

describe("portable vault error mapping", () => {
  it("turns cryptographic unlock failures into a retryable master-password message", () => {
    setLanguagePreference("zh");
    expect(formatPortableVaultError("portable vault 解锁失败: MAC check failed")).toBe("Stronghold 主密码不正确，请重试。");
    expect(formatPortableVaultError("portable vault 当前主密码验证失败: invalid key")).toBe("Stronghold 主密码不正确，请重试。");
    expect(formatPortableVaultError("portable vault 解锁失败: cipher")).toBe("Stronghold 主密码不正确，请重试。");
  });

  it("keeps structural vault problems distinct from a wrong password", () => {
    setLanguagePreference("zh");
    expect(formatPortableVaultError("portable vault snapshot 存在，但 salt 文件缺失，已阻止解锁")).toBe("Stronghold 文件不完整，无法验证主密码。请检查本机密钥库文件后重试。");
    expect(formatPortableVaultError("portable vault 尚未创建，请先使用创建操作")).toBe("Stronghold 尚未创建，请使用下方的创建向导");
    expect(formatPortableVaultError("portable vault 已存在，请使用解锁操作")).toBe("Stronghold 已存在，请使用解锁操作");
    expect(formatPortableVaultError("portable vault 已解锁")).toBe("Stronghold 已解锁。");
  });
});
