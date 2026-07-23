import { describe, expect, it } from "vitest";
import {
  OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS,
  parseOpenSshConfig,
} from "./openssh-config-import";

describe("OpenSSH config import", () => {
  it("maps literal Host entries to PortMate SSH fields", () => {
    const result = parseOpenSshConfig(`
Host production
  HostName app.example.test
  User deploy
  Port 2202
  IdentityFile ~/.ssh/id_deploy
  IdentityFile ~/.ssh/id_fallback
  ServerAliveInterval 45
  ServerAliveCountMax 5
  IdentitiesOnly yes
  ForwardAgent no
  ProxyJump ops@bastion.example.test:2222,edge.example.test
`);

    expect(result.error).toBeNull();
    expect(result.warnings).toEqual([]);
    expect(result.candidates).toEqual([{
      id: "production",
      hostAlias: "production",
      host: "app.example.test",
      port: 2202,
      username: "deploy",
      identityFiles: ["~/.ssh/id_deploy", "~/.ssh/id_fallback"],
      keepaliveEnabled: true,
      keepaliveIntervalSeconds: 45,
      keepaliveMaxMissed: 5,
      identitiesOnly: true,
      forwardAgent: false,
      jumps: [
        { host: "bastion.example.test", port: 2222, username: "ops" },
        { host: "edge.example.test", port: 22, username: "" },
      ],
      warnings: [],
    }]);
  });

  it("keeps first scalar values while accumulating identity files for repeated Host blocks", () => {
    const result = parseOpenSshConfig(`
Host api staging
  User first-user
  IdentityFile ~/.ssh/id_shared
Host api
  User later-user
  HostName api.internal.test
  IdentityFile ~/.ssh/id_api
Host staging
  Port 2223
`);

    expect(result.candidates).toEqual(expect.arrayContaining([
      expect.objectContaining({
        hostAlias: "api",
        host: "api.internal.test",
        username: "first-user",
        port: 22,
        identityFiles: ["~/.ssh/id_shared", "~/.ssh/id_api"],
      }),
      expect.objectContaining({
        hostAlias: "staging",
        host: "staging",
        username: "first-user",
        port: 2223,
        identityFiles: ["~/.ssh/id_shared"],
      }),
    ]));
  });

  it("skips wildcard and conditional configuration while retaining actionable warnings", () => {
    const result = parseOpenSshConfig(`
Host *
  ServerAliveInterval 60
Host safe
  HostName safe.example.test
  ProxyCommand ssh gateway nc %h %p
Match exec "test -n \"$SSH_TTY\""
  User ignored
Include ~/.ssh/config.d/*
`);

    expect(result.candidates).toEqual([expect.objectContaining({
      hostAlias: "safe",
      host: "safe.example.test",
      username: "",
      warnings: ["proxycommand 未导入"],
    })]);
    expect(result.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("Host * 不是字面条目"),
      expect.stringContaining("proxycommand"),
      expect.stringContaining("Match 条件块未导入"),
      expect.stringContaining("Include 未读取外部文件"),
    ]));
  });

  it("rejects unsupported dynamic values and invalid numeric ranges instead of changing them", () => {
    const result = parseOpenSshConfig(`
Host bounded
  HostName %h.example.test
  Port 70000
  IdentityFile ~other/id_ed25519
  ServerAliveInterval 3601
  ServerAliveCountMax 0
  IdentitiesOnly ask
  ForwardAgent confirm
  ProxyJump ssh://jump.example.test
`);

    expect(result.candidates).toEqual([expect.objectContaining({
      hostAlias: "bounded",
      host: "bounded",
      port: 22,
      identityFiles: [],
      jumps: [],
      warnings: expect.arrayContaining([
        "HostName 不是可直接导入的字面地址",
        "Port 必须是 1 到 65535 的整数",
        "IdentityFile 不是可直接导入的本地路径",
        "ServerAliveInterval 必须是 0 到 3600 的整数",
        "ServerAliveCountMax 必须是 1 到 20 的整数",
        "IdentitiesOnly 仅支持 yes 或 no",
        "ForwardAgent 仅支持 yes 或 no",
        "ProxyJump 仅支持逗号分隔的 [user@]host[:port] 字面地址",
      ]),
    })]);
  });

  it("bounds source input before splitting it into lines", () => {
    const result = parseOpenSshConfig("x".repeat(OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS + 1));

    expect(result.candidates).toEqual([]);
    expect(result.warnings).toEqual([]);
    expect(result.error).toContain("字符限制");
  });
});
