import { describe, expect, it } from "vitest";
import {
  ccSwitchServerIdForGrant,
  defaultMcpHttpOrigins,
  defaultMcpHttpSettings,
  formatCcSwitchMcpJson,
  formatMcpHttpOrigins,
  isNonLoopbackMcpHost,
  mcpHttpListenPreset,
  mcpHttpClientEndpoint,
  parseMcpHttpOrigins,
} from "./mcp-http-state";

describe("MCP HTTP settings", () => {
  it("uses a loopback listener and matching browser origins by default", () => {
    expect(defaultMcpHttpSettings()).toEqual({
      listenHost: "127.0.0.1",
      clientHost: "127.0.0.1",
      port: 8787,
      allowedOrigins: ["http://127.0.0.1:8787", "http://localhost:8787"],
      clientId: "portmate-local",
      trusted: false,
      allowRemote: false,
    });
    expect(defaultMcpHttpOrigins(9000)).toEqual(["http://127.0.0.1:9000", "http://localhost:9000"]);
  });

  it("generates the flat CC Switch server JSON with an inline bearer token", () => {
    const json = formatCcSwitchMcpJson({
      clientHost: "192.168.33.222",
      port: 8787,
    }, { token: "portmate-test-token" });
    expect(JSON.parse(json)).toEqual({
      portmate: {
        type: "http",
        url: "http://192.168.33.222:8787/mcp",
        headers: {
          Authorization: "Bearer portmate-test-token",
        },
        tool_timeout_sec: 180,
      },
    });
    expect(json).not.toContain("mcpServers");
    expect(formatCcSwitchMcpJson({ clientHost: "192.168.33.222", port: 8787 })).toBe("");
  });

  it("derives stable import IDs for saved grants", () => {
    expect(ccSwitchServerIdForGrant("ops-console")).toBe("portmate-ops-console");
    expect(ccSwitchServerIdForGrant("  Lab Operator / Router  ")).toBe("portmate-lab-operator-router");
    expect(ccSwitchServerIdForGrant("中文授权")).toBe("portmate");
    expect(ccSwitchServerIdForGrant("x".repeat(128))).toHaveLength(64);
  });

  it("formats IPv6 client endpoints and rejects listener wildcard addresses", () => {
    expect(mcpHttpClientEndpoint({ clientHost: "[2001:db8::42]", port: 9088 }))
      .toBe("http://[2001:db8::42]:9088/mcp");
    expect(formatCcSwitchMcpJson(
      { clientHost: "mcp.example.test", port: 9088 },
      { serverId: "portmate-lab", token: "lab-token", toolTimeoutSeconds: 240 },
    )).toContain('"Authorization": "Bearer lab-token"');
    expect(mcpHttpClientEndpoint({ clientHost: "0.0.0.0", port: 8787 })).toBeNull();
    expect(formatCcSwitchMcpJson({ clientHost: "::", port: 8787 })).toBe("");
  });

  it("normalizes origin input and classifies wildcard listeners", () => {
    const origins = parseMcpHttpOrigins("http://localhost:8787, https://console.example.test\nhttp://localhost:8787");
    expect(origins).toEqual(["http://localhost:8787", "https://console.example.test"]);
    expect(formatMcpHttpOrigins(origins)).toBe("http://localhost:8787\nhttps://console.example.test");
    expect(isNonLoopbackMcpHost("127.4.3.2")).toBe(false);
    expect(isNonLoopbackMcpHost("::1")).toBe(false);
    expect(isNonLoopbackMcpHost("0.0.0.0")).toBe(true);
    expect(isNonLoopbackMcpHost("::")).toBe(true);
    expect(mcpHttpListenPreset("0.0.0.0")).toBe("0.0.0.0");
    expect(mcpHttpListenPreset(" [::1] ")).toBe("::1");
    expect(mcpHttpListenPreset("192.0.2.10")).toBe("custom");
  });
});
