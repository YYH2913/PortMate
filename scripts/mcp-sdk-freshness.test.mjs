import { describe, expect, it, vi } from "vitest";
import {
  assertLatestCovered,
  compareStableVersions,
  fetchTextWithRetries,
  newestStableVersion,
  normalizeStableVersion,
  parseLatestSdkVersion,
  proxyOptionsFromEnvironment,
  readBoundedResponseText,
} from "./mcp-sdk-freshness.mjs";

describe("MCP SDK freshness audit", () => {
  it("normalizes and orders stable semantic versions", () => {
    expect(normalizeStableVersion("v1.30.0")).toBe("1.30.0");
    expect(normalizeStableVersion("1.30.0-rc.1")).toBeNull();
    expect(normalizeStableVersion("01.30.0")).toBeNull();
    expect(normalizeStableVersion("99999999999999999999.0.0")).toBeNull();
    expect(compareStableVersions("2.0.0", "1.30.0")).toBeGreaterThan(0);
    expect(newestStableVersion(["v1.7.0-pre.1", "v1.6.1", "v1.7.0"])).toBe("1.7.0");
  });

  it("parses every official registry response shape", () => {
    expect(parseLatestSdkVersion("npm", JSON.stringify({ version: "1.30.0" }))).toBe("1.30.0");
    expect(parseLatestSdkVersion("pypi", JSON.stringify({ info: { version: "2.0.0" } }))).toBe("2.0.0");
    expect(parseLatestSdkVersion("go-proxy", "v1.6.1\nv1.7.0-pre.1\nv1.7.0\n")).toBe("1.7.0");
    expect(parseLatestSdkVersion("crates-io", JSON.stringify({
      crate: { max_stable_version: "3.0.0", max_version: "3.1.0-rc.1" },
    }))).toBe("3.0.0");
    expect(parseLatestSdkVersion("rubygems", JSON.stringify({ version: "1.0.0" }))).toBe("1.0.0");
    expect(parseLatestSdkVersion("maven", "<metadata><versioning><release>2.0.0</release></versioning></metadata>"))
      .toBe("2.0.0");
    expect(parseLatestSdkVersion("nuget", JSON.stringify({
      versions: ["1.4.1", "2.0.0-rc.2", "2.0.0"],
    }))).toBe("2.0.0");
    expect(parseLatestSdkVersion("github-release", JSON.stringify({ tag_name: "v0.12.1" }))).toBe("0.12.1");
  });

  it("reports a stale matrix with the registry and newest covered versions", () => {
    const source = {
      label: "Python",
      matrixFile: "mcp-python-client-versions.json",
    };
    expect(() => assertLatestCovered(source, ["1.29.0", "2.0.0"], "2.1.0")).toThrow(
      "Python: registry latest 2.1.0 is absent from scripts/mcp-python-client-versions.json; newest covered version is 2.0.0",
    );
  });

  it("retries transient registry failures within the configured bound", async () => {
    const fetchImpl = vi.fn()
      .mockResolvedValueOnce(new Response("busy", { status: 503 }))
      .mockResolvedValueOnce(new Response("ok", { status: 200 }));

    await expect(fetchTextWithRetries("https://registry.example.test/latest", {
      fetchImpl,
      attempts: 2,
      timeoutMs: 1_000,
    })).resolves.toBe("ok");
    expect(fetchImpl).toHaveBeenCalledTimes(2);
  });

  it("bounds declared and streamed registry response bodies", async () => {
    const declared = new Response("small", { headers: { "content-length": "2048" } });
    await expect(readBoundedResponseText(declared, 1024)).rejects.toThrow(
      "Registry response declares 2048 bytes, exceeding the 1024-byte limit",
    );

    const streamed = new Response("x".repeat(1025));
    await expect(readBoundedResponseText(streamed, 1024)).rejects.toThrow(
      "Registry response exceeds the 1024-byte limit",
    );
  });

  it("uses lowercase proxy variables before their uppercase aliases", () => {
    expect(proxyOptionsFromEnvironment({
      http_proxy: "http://lower-http",
      HTTP_PROXY: "http://upper-http",
      https_proxy: "http://lower-https",
      HTTPS_PROXY: "http://upper-https",
      no_proxy: "localhost",
      NO_PROXY: "example.test",
    })).toEqual({
      httpProxy: "http://lower-http",
      httpsProxy: "http://lower-https",
      noProxy: "localhost",
    });
  });
});
