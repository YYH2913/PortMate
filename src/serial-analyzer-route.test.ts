import { describe, expect, it } from "vitest";
import { buildSerialAnalyzerPath, parseSerialAnalyzerRequest } from "./serial-analyzer-route";

describe("serial analyzer route", () => {
  it("round-trips validated analyzer window identities", () => {
    const request = { windowId: "serial-analyzer-1", ownerWindowId: "workspace-1", sessionId: "serial/profile" };
    const path = buildSerialAnalyzerPath(request);
    expect(parseSerialAnalyzerRequest(new URL(path, "http://localhost").search)).toEqual(request);
  });

  it("rejects malformed labels and control characters", () => {
    expect(parseSerialAnalyzerRequest("?serialAnalyzer=1&windowId=bad%2Fid&sessionId=serial")).toBeNull();
    expect(parseSerialAnalyzerRequest("?serialAnalyzer=1&windowId=good&ownerWindowId=bad%2Fid&sessionId=serial")).toBeNull();
    expect(parseSerialAnalyzerRequest("?serialAnalyzer=1&windowId=good&sessionId=x%0A")).toBeNull();
    expect(parseSerialAnalyzerRequest("?windowId=good&sessionId=serial")).toBeNull();
  });

  it("keeps legacy analyzer links attached to the main workspace", () => {
    expect(parseSerialAnalyzerRequest("?serialAnalyzer=1&windowId=good&sessionId=serial"))
      .toMatchObject({ ownerWindowId: "main" });
  });
});
