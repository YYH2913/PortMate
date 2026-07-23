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
      proxy: {
        kind: "socks5",
        host: "socks.example.test",
        port: 1081,
        username: "relay",
      },
      warnings: [],
    }]);
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
