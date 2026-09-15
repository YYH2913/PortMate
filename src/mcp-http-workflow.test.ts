import { describe, expect, it, vi } from "vitest";
import { prepareMcpHttpAccess } from "./mcp-http-workflow";
import type { McpInvoke } from "./mcp-http-workflow";
import { defaultMcpHttpSettings } from "./mcp-http-state";
import type { McpHttpConfig } from "./types";

const settings = { ...defaultMcpHttpSettings(), clientId: "new-client" };
const config = { ...settings, tokenAvailable: false } as McpHttpConfig;

describe("MCP HTTP connection workflow", () => {
  it("saves the binding before generating a token and uses canonical access", async () => {
    const invoke = vi.fn().mockResolvedValueOnce(config)
      .mockResolvedValueOnce({ config, token: null })
      .mockResolvedValueOnce({ config: { ...config, tokenAvailable: true }, token: "new-token" });
    const access = await prepareMcpHttpAccess(invoke as McpInvoke, settings, true, true);
    expect(invoke.mock.calls.map(call => call[0])).toEqual([
      "save_mcp_http_settings", "mcp_http_access_config", "rotate_mcp_http_token",
    ]);
    expect(access?.token).toBe("new-token");
    expect(invoke.mock.calls[0][1]).toEqual({ settings });
  });
  it("never rotates merely to save settings", async () => {
    const invoke = vi.fn().mockResolvedValueOnce(config).mockResolvedValueOnce({ config, token: null });
    await prepareMcpHttpAccess(invoke as McpInvoke, settings, true, false);
    expect(invoke).toHaveBeenCalledTimes(2);
  });
  it("keeps the existing canonical token when starting unchanged settings", async () => {
    const invoke = vi.fn().mockResolvedValue({ config, token: "canonical-token" });
    expect((await prepareMcpHttpAccess(invoke as McpInvoke, settings, false, true))?.token).toBe("canonical-token");
    expect(invoke).toHaveBeenCalledExactlyOnceWith("mcp_http_access_config", {});
  });
  it("does not rotate or start after a failed save", async () => {
    const invoke = vi.fn().mockRejectedValueOnce(new Error("disk full"));
    await expect(prepareMcpHttpAccess(invoke as McpInvoke, settings, true, true)).rejects.toThrow("disk full");
    expect(invoke).toHaveBeenCalledTimes(1);
  });
  it("stops the workflow when its window closes during a save", async () => {
    let current = true;
    const invoke = vi.fn().mockImplementation(async () => { current = false; return config; });
    expect(await prepareMcpHttpAccess(invoke as McpInvoke, settings, true, true, () => current)).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(1);
  });
  it("does not continue from a failed access or token read", async () => {
    const invoke = vi.fn().mockRejectedValueOnce(new Error("keyring unavailable"));
    await expect(prepareMcpHttpAccess(invoke as McpInvoke, settings, false, true)).rejects.toThrow("keyring unavailable");
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
