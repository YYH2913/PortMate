import { describe, expect, it } from "vitest";
import {
  defaultMcpHttpOrigins,
  defaultMcpHttpSettings,
  formatMcpHttpOrigins,
  isNonLoopbackMcpHost,
  mcpHttpListenPreset,
  parseMcpHttpOrigins,
} from "./mcp-http-state";

describe("MCP HTTP settings", () => {
  it("uses a loopback listener and matching browser origins by default", () => {
    expect(defaultMcpHttpSettings()).toEqual({
      listenHost: "127.0.0.1",
      port: 8787,
      allowedOrigins: ["http://127.0.0.1:8787", "http://localhost:8787"],
      clientId: "portmate-local",
      trusted: false,
      allowRemote: false,
    });
    expect(defaultMcpHttpOrigins(9000)).toEqual(["http://127.0.0.1:9000", "http://localhost:9000"]);
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
