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
  HostKeyAlias production-device
  IdentityFile ~/.ssh/id_deploy
  IdentityFile ~/.ssh/id_fallback
  ServerAliveInterval 45
  ServerAliveCountMax 5
  TCPKeepAlive no
  IdentitiesOnly yes
  ForwardAgent no
  ProxyJump ops@bastion.example.test:2222,edge.example.test
  LocalForward 127.0.0.1:15432 db.example.test:5432
  RemoteForward 0 127.0.0.1:22
  DynamicForward [::1]:1080
`);

    expect(result.error).toBeNull();
    expect(result.warnings).toEqual([]);
    expect(result.candidates).toEqual([{
      id: "production",
      hostAlias: "production",
      host: "app.example.test",
      port: 2202,
      username: "deploy",
      hostKeyAlias: "production-device",
      identityFiles: ["~/.ssh/id_deploy", "~/.ssh/id_fallback"],
      keepaliveEnabled: true,
      keepaliveIntervalSeconds: 45,
      keepaliveMaxMissed: 5,
      tcpKeepaliveEnabled: false,
      identitiesOnly: true,
      forwardAgent: false,
      jumps: [
        { host: "bastion.example.test", port: 2222, username: "ops" },
        { host: "edge.example.test", port: 22, username: "" },
      ],
      forwards: [
        { mode: "local", bindHost: "127.0.0.1", bindPort: 15432, targetHost: "db.example.test", targetPort: 5432 },
        { mode: "remote", bindHost: "", bindPort: 0, targetHost: "127.0.0.1", targetPort: 22 },
        { mode: "dynamic", bindHost: "::1", bindPort: 1080, targetHost: "", targetPort: 0 },
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

  it("applies standalone Host * defaults while retaining conditional and external-config warnings", () => {
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
      keepaliveEnabled: true,
      keepaliveIntervalSeconds: 60,
      warnings: ["proxycommand 未导入"],
    })]);
    expect(result.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("proxycommand"),
      expect.stringContaining("Match 条件块未导入"),
      expect.stringContaining("Include 未读取外部文件"),
    ]));
  });

  it("uses Host * defaults in OpenSSH configuration order without treating complex patterns as defaults", () => {
    const result = parseOpenSshConfig(`
Host specific
  HostName specific.example.test
  User profile-user
  Port 2202
  IdentityFile ~/.ssh/id_specific
Host *
  User default-user
  Port 2222
  HostKeyAlias lab-default
  IdentityFile ~/.ssh/id_default
  ServerAliveInterval 45
  ServerAliveCountMax 5
  TCPKeepAlive yes
  IdentitiesOnly yes
  ForwardAgent no
  ProxyJump ops@jump.example.test:2222
  LocalForward 15432 db.example.test:5432
Host later
  HostName later.example.test
  User ignored-after-default
  IdentityFile ~/.ssh/id_later
Host *.example.test
  User ignored-pattern
`);

    expect(result.candidates).toEqual(expect.arrayContaining([
      expect.objectContaining({
        hostAlias: "specific",
        host: "specific.example.test",
        username: "profile-user",
        port: 2202,
        hostKeyAlias: "lab-default",
        identityFiles: ["~/.ssh/id_specific", "~/.ssh/id_default"],
        keepaliveEnabled: true,
        keepaliveIntervalSeconds: 45,
        keepaliveMaxMissed: 5,
        tcpKeepaliveEnabled: true,
        identitiesOnly: true,
        forwardAgent: false,
        jumps: [{ host: "jump.example.test", port: 2222, username: "ops" }],
        forwards: [{ mode: "local", bindHost: "127.0.0.1", bindPort: 15432, targetHost: "db.example.test", targetPort: 5432 }],
      }),
      expect.objectContaining({
        hostAlias: "later",
        host: "later.example.test",
        username: "default-user",
        port: 2222,
        hostKeyAlias: "lab-default",
        identityFiles: ["~/.ssh/id_default", "~/.ssh/id_later"],
        keepaliveEnabled: true,
        keepaliveIntervalSeconds: 45,
        keepaliveMaxMissed: 5,
        tcpKeepaliveEnabled: true,
        identitiesOnly: true,
        forwardAgent: false,
        jumps: [{ host: "jump.example.test", port: 2222, username: "ops" }],
        forwards: [{ mode: "local", bindHost: "127.0.0.1", bindPort: 15432, targetHost: "db.example.test", targetPort: 5432 }],
      }),
    ]));
    expect(result.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("Host *.example.test 不是字面条目"),
    ]));
    expect(result.candidates).toHaveLength(2);
  });

  it("rejects Unix-socket, dynamic, wildcard, and incomplete forwarding forms", () => {
    const result = parseOpenSshConfig(`
Host bounded
  LocalForward 15432 /run/postgresql/.s.PGSQL.5432
  RemoteForward *:2200 127.0.0.1:22
  DynamicForward %h:1080
  LocalForward 8080
  DynamicForward 1080
`);

    expect(result.candidates).toEqual([expect.objectContaining({
      hostAlias: "bounded",
      forwards: [{ mode: "dynamic", bindHost: "127.0.0.1", bindPort: 1080, targetHost: "", targetPort: 0 }],
      warnings: expect.arrayContaining([
        "localforward 仅支持安全的 TCP [bind_host:]port 和 host:port 字面地址",
        "remoteforward 仅支持安全的 TCP [bind_host:]port 和 host:port 字面地址",
        "dynamicforward 仅支持安全的 TCP [bind_host:]port 和 host:port 字面地址",
      ]),
    })]);
  });

  it("rejects unsupported dynamic values and invalid numeric ranges instead of changing them", () => {
    const result = parseOpenSshConfig(`
Host bounded
  HostName %h.example.test
  Port 70000
  IdentityFile ~other/id_ed25519
  ServerAliveInterval 3601
  ServerAliveCountMax 21
  TCPKeepAlive maybe
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
        "ServerAliveCountMax 必须是 0 到 20 的整数",
        "TCPKeepAlive 仅支持 yes 或 no",
        "IdentitiesOnly 仅支持 yes 或 no",
        "ForwardAgent 仅支持 yes 或 no",
        "ProxyJump 仅支持逗号分隔的 [user@]host[:port] 字面地址",
      ]),
    })]);
  });

  it("skips ServerAliveCountMax zero because its immediate-timeout behavior is not representable", () => {
    const result = parseOpenSshConfig(`
Host long-lived
  HostName long-lived.example.test
  ServerAliveInterval 45
  ServerAliveCountMax 0
`);

    expect(result.candidates).toEqual([expect.objectContaining({
      hostAlias: "long-lived",
      keepaliveEnabled: true,
      keepaliveIntervalSeconds: 45,
      warnings: ["ServerAliveCountMax=0 会在首个保活探测前断开，PortMate 未导入该值"],
    })]);
    expect(result.candidates[0]).not.toHaveProperty("keepaliveMaxMissed");
    expect(result.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("ServerAliveCountMax=0 会在首个保活探测前断开"),
    ]));
  });

  it("bounds source input before splitting it into lines", () => {
    const result = parseOpenSshConfig("x".repeat(OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS + 1));

    expect(result.candidates).toEqual([]);
    expect(result.warnings).toEqual([]);
    expect(result.error).toContain("字符限制");
  });
});
