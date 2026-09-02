import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import CredentialDialog from "./CredentialDialog";
import type { CredentialPromptState } from "./CredentialDialog";

const baseRequest: CredentialPromptState = {
  requestId: 1,
  target: "root@example.test:22",
  initialUsername: "root",
  oneKeys: [],
  hasIdentityFiles: false,
  hasSavedPassword: false,
  hasSavedPassphrase: false,
  needsPassword: true,
  authOrder: ["password"],
};

describe("credential dialog Stronghold guidance", () => {
  it.each([
    ["not-created", "尚未创建 Stronghold，保存密码前请先创建密钥库。"],
    ["locked", "Stronghold 已锁定，保存密码前请先解锁密钥库。"],
  ] as const)("offers a direct Stronghold action when the vault is %s", (strongholdStatus, message) => {
    const html = renderToStaticMarkup(
      <CredentialDialog
        request={{ ...baseRequest, strongholdStatus }}
        onCancel={() => {}}
        onSubmit={() => {}}
        onOpenStronghold={() => {}}
      />,
    );

    expect(html).toContain(message);
    expect(html).toContain("打开 Stronghold");
  });

  it("does not show a setup warning after the vault is unlocked", () => {
    const html = renderToStaticMarkup(
      <CredentialDialog
        request={{ ...baseRequest, strongholdStatus: "unlocked" }}
        onCancel={() => {}}
        onSubmit={() => {}}
        onOpenStronghold={() => {}}
      />,
    );

    expect(html).not.toContain("打开 Stronghold");
    expect(html).not.toContain("需先解锁");
  });

  it("keeps Stronghold save controls unavailable while the vault is locked", () => {
    const html = renderToStaticMarkup(
      <CredentialDialog
        request={{ ...baseRequest, strongholdStatus: "locked" }}
        onCancel={() => {}}
        onSubmit={() => {}}
        onOpenStronghold={() => {}}
      />,
    );

    expect(html).toMatch(/type="checkbox"[^>]*disabled=""/);
    expect(html).not.toContain("保存并连接");
  });
});
