import { describe, expect, it } from "vitest";
import {
  PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS,
  parsePuttySessions,
} from "./putty-session-import";

describe("PuTTY session import", () => {
  it("maps a Unix PuTTY session file into an SSH candidate", () => {
    const result = parsePuttySessions(`
HostName=app.example.test
PortNumber=2202
Protocol=ssh
UserName=deploy
TryAgent=1
AgentFwd=0
ProxyMethod=2
ProxyHost=socks.example.test
ProxyPort=1081
ProxyUsername=relay
PingInterval=1
PingIntervalSecs=15
TCPKeepalives=1
TerminalType=xterm-256color
TermWidth=180
TermHeight=40
ScrollbackLines=5000
`, "production.session");

    expect(result.error).toBeNull();
    expect(result.warnings).toEqual([]);
    expect(result.candidates).toEqual([{
      id: "putty-1-production",
      name: "production",
      kind: "ssh",
      host: "app.example.test",
      port: 2202,
      username: "deploy",
      tryAgent: true,
      forwardAgent: false,
      keepaliveEnabled: true,
      keepaliveIntervalSeconds: 75,
      keepaliveMaxMissed: 0,
      tcpKeepaliveEnabled: true,
      terminal: {
        term: "xterm-256color",
        rows: 40,
        cols: 180,
        scrollback: 5000,
      },
      proxy: {
        kind: "socks5",
        host: "socks.example.test",
        port: 1081,
        username: "relay",
      },
      warnings: [],
    }]);
  });

  it("maps bounded PuTTY keepalive intervals and rejects invalid values", () => {
    const disabled = parsePuttySessions(`
HostName=disabled.example.test
Protocol=ssh
PingInterval=0
PingIntervalSecs=0
`, "disabled");
    expect(disabled.candidates).toEqual([expect.objectContaining({
      kind: "ssh",
      keepaliveEnabled: false,
    })]);

    const invalid = parsePuttySessions(`
HostName=bounded.example.test
Protocol=ssh
PingInterval=60
PingIntervalSecs=1
TCPKeepalives=2
TerminalType=xterm invalid
TermWidth=0
TermHeight=513
ScrollbackLines=10000001
`, "bounded");
    expect(invalid.candidates).toEqual([expect.objectContaining({
      kind: "ssh",
      warnings: expect.arrayContaining([
        "PingInterval 总间隔必须是 0 到 3600 秒，未导入 SSH 保活",
        "TCPKeepalives 仅支持 0 或 1",
        "TerminalType 必须是 64 字节以内的标准终端名称，未导入",
        "TermWidth 必须是 1 到 1024 的整数，未导入终端列数",
        "TermHeight 必须是 1 到 512 的整数，未导入终端行数",
        "ScrollbackLines 必须是 0 到 10000000 的整数，未导入终端滚屏",
      ]),
    })]);
    expect(invalid.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("PingInterval 总间隔必须是 0 到 3600 秒"),
    ]));
  });

  it("imports safe TCP local, remote, and dynamic SSH forwarding rules", () => {
    const result = parsePuttySessions(`
HostName=app.example.test
PortNumber=2202
Protocol=ssh
UserName=deploy
PortForwardings=L15432=db.example.test:5432,R[::1]:2200=[::1]:22,D1080=,L[::1]:1081=D
`, "forwarding.session");

    expect(result.error).toBeNull();
    expect(result.warnings).toEqual([]);
    expect(result.candidates).toEqual([expect.objectContaining({
      id: "putty-1-forwarding",
      kind: "ssh",
      forwards: [
        { mode: "local", bindHost: "127.0.0.1", bindPort: 15432, targetHost: "db.example.test", targetPort: 5432 },
        { mode: "remote", bindHost: "::1", bindPort: 2200, targetHost: "::1", targetPort: 22 },
        { mode: "dynamic", bindHost: "127.0.0.1", bindPort: 1080, targetHost: "", targetPort: 0 },
        { mode: "dynamic", bindHost: "::1", bindPort: 1081, targetHost: "", targetPort: 0 },
      ],
      warnings: [],
    })]);
  });

  it("preserves PuTTY accept-all forwarding intent within PortMate's TCP listener model", () => {
    const result = parsePuttySessions(`
HostName=relay.example.test
Protocol=ssh
LocalPortAcceptAll=1
RemotePortAcceptAll=1
PortForwardings=L15432=db.example.test:5432,R2200=127.0.0.1:22,D1080=
`, "relay");

    expect(result.candidates).toEqual([expect.objectContaining({
      kind: "ssh",
      forwards: [
        { mode: "local", bindHost: "0.0.0.0", bindPort: 15432, targetHost: "db.example.test", targetPort: 5432 },
        { mode: "remote", bindHost: "", bindPort: 2200, targetHost: "127.0.0.1", targetPort: 22 },
        { mode: "dynamic", bindHost: "0.0.0.0", bindPort: 1080, targetHost: "", targetPort: 0 },
      ],
      warnings: ["LocalPortAcceptAll=1 已映射为 0.0.0.0；IPv6 公网监听请在会话设置中另行添加"],
    })]);
    expect(result.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("LocalPortAcceptAll=1 已映射为 0.0.0.0"),
    ]));
  });

  it("keeps valid PuTTY forwards while warning about unrepresentable forwarding forms", () => {
    const result = parsePuttySessions(`
HostName=bounded.example.test
Protocol=ssh
PortForwardings=4L15432=db.example.test:5432,L*:15433=db.example.test:5432,R2200=/run/service.sock,D1080=target,D1081=,L15434=db.example.test:5432
`, "bounded");

    expect(result.candidates).toEqual([expect.objectContaining({
      kind: "ssh",
      forwards: [
        { mode: "dynamic", bindHost: "127.0.0.1", bindPort: 1081, targetHost: "", targetPort: 0 },
        { mode: "local", bindHost: "127.0.0.1", bindPort: 15434, targetHost: "db.example.test", targetPort: 5432 },
      ],
      warnings: expect.arrayContaining([
        "PortForwardings 第 1 条：IPv4/IPv6 强制地址族未导入",
        "PortForwardings 第 2 条：仅支持字面 TCP 监听地址与端口",
        "PortForwardings 第 3 条：仅支持字面 TCP 目标 host:port",
        "PortForwardings 第 4 条：动态转发不能包含目标地址",
      ]),
    })]);
  });

  it("parses Windows registry exports and decodes session names and DWORD values", () => {
    const result = parsePuttySessions(`Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Ops%20SSH]
"HostName"="ops.example.test"
"PortNumber"=dword:0000089a
"Protocol"="ssh"
"UserName"="operator"
"TryAgent"=dword:00000001
"AgentFwd"=dword:00000001
"ProxyMethod"=dword:00000003
"ProxyHost"="proxy.example.test"
"ProxyPort"=dword:00001f90
"ProxyUsername"="proxy-user"
"PortForwardings"="L15432=db.example.test:5432,D1080="

[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Bench%20Serial]
"Protocol"="serial"
"SerialLine"="COM7"
"SerialSpeed"=dword:0001c200
"SerialDataBits"=dword:00000008
"SerialStopHalfbits"=dword:00000004
"SerialParity"=dword:00000002
"SerialFlowControl"=dword:00000002
`);

    expect(result.error).toBeNull();
    expect(result.warnings).toEqual([]);
    expect(result.candidates).toEqual([
      {
        id: "putty-1-Ops SSH",
        name: "Ops SSH",
        kind: "ssh",
        host: "ops.example.test",
        port: 2202,
        username: "operator",
        tryAgent: true,
        forwardAgent: true,
        proxy: {
          kind: "http-connect",
          host: "proxy.example.test",
          port: 8080,
          username: "proxy-user",
        },
        forwards: [
          { mode: "local", bindHost: "127.0.0.1", bindPort: 15432, targetHost: "db.example.test", targetPort: 5432 },
          { mode: "dynamic", bindHost: "127.0.0.1", bindPort: 1080, targetHost: "", targetPort: 0 },
        ],
        warnings: [],
      },
      {
        id: "putty-2-Bench Serial",
        name: "Bench Serial",
        kind: "serial",
        serial: {
          port: "COM7",
          baudRate: 115200,
          dataBits: 8,
          stopBits: 2,
          parity: "even",
          flowControl: "hardware",
        },
        warnings: [],
      },
    ]);
  });

  it("keeps safe sessions while warning about private keys, secrets, and unsupported proxy modes", () => {
    const result = parsePuttySessions(`
HostName=router.example.test
PortNumber=22
Protocol=ssh
ProxyMethod=1
ProxyPassword=stored-secret
PublicKeyFile=C:\\Users\\operator\\id_router.ppk
`, "router");

    expect(result.candidates).toEqual([expect.objectContaining({
      name: "router",
      kind: "ssh",
      host: "router.example.test",
      port: 22,
      warnings: expect.arrayContaining([
        "PublicKeyFile 未导入；PortMate 不会直接读取 PuTTY .ppk 私钥文件",
        "ProxyPassword 未导入；请在会话设置中重新录入代理密码",
        "ProxyMethod=1 不受支持，未导入代理",
      ]),
    })]);
    expect(result.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("PublicKeyFile 未导入"),
      expect.stringContaining("ProxyPassword 未导入"),
      expect.stringContaining("ProxyMethod=1 不受支持"),
    ]));
  });

  it("retains only serial settings PortMate can represent", () => {
    const result = parsePuttySessions(`
Protocol=serial
SerialLine=/dev/ttyUSB0
SerialSpeed=115200
SerialStopHalfbits=3
SerialParity=3
SerialFlowControl=3
`, "bench");

    expect(result.candidates).toEqual([expect.objectContaining({
      kind: "serial",
      serial: { port: "/dev/ttyUSB0", baudRate: 115200 },
      warnings: expect.arrayContaining([
        "SerialStopHalfbits=3 表示 1.5 停止位，未导入",
        "SerialParity 的 mark 或 space 模式未导入",
        "SerialFlowControl 的 DSR/DTR 模式未导入",
      ]),
    })]);
  });

  it("skips default, unsupported, and incomplete sessions instead of producing unusable profiles", () => {
    const defaultResult = parsePuttySessions("HostName=default.example.test", "Default%20Settings");
    expect(defaultResult.candidates).toEqual([]);
    expect(defaultResult.warnings).toEqual(expect.arrayContaining([
      "PuTTY Default Settings 未作为独立会话导入",
    ]));

    const unsupportedResult = parsePuttySessions(`
Protocol=rlogin
HostName=legacy.example.test
PortNumber=513
`, "legacy");
    expect(unsupportedResult.candidates).toEqual([]);
    expect(unsupportedResult.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("Protocol=rlogin 不受支持"),
    ]));

    const invalidRawResult = parsePuttySessions(`
Protocol=raw
HostName=raw.example.test
`, "raw");
    expect(invalidRawResult.candidates).toEqual([]);
    expect(invalidRawResult.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("raw 会话缺少 PortNumber"),
    ]));
  });

  it("bounds source input before parsing it", () => {
    const result = parsePuttySessions("x".repeat(PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS + 1));

    expect(result.candidates).toEqual([]);
    expect(result.warnings).toEqual([]);
    expect(result.error).toContain("字符限制");
  });
});
