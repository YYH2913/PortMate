import { describe, expect, it, vi } from "vitest";
import { normalizeTerminalWebLink, openIsolatedWebLink, openTerminalWebLink } from "./terminal-web-link";

describe("terminal web links", () => {
  it("normalizes only HTTP and HTTPS destinations", () => {
    expect(normalizeTerminalWebLink("HTTPS://example.test/a b")).toBe("https://example.test/a%20b");
    expect(normalizeTerminalWebLink("javascript:alert(1)")).toBeNull();
    expect(normalizeTerminalWebLink("file:///tmp/secret")).toBeNull();
    expect(normalizeTerminalWebLink("not a URL")).toBeNull();
  });

  it("isolates the popup without interrupting XTerm mouse-state cleanup", () => {
    const event = {
      preventDefault: vi.fn(),
    };
    const popup = { opener: {} };
    const openWindow = vi.fn(() => popup);

    expect(openTerminalWebLink(event, "https://example.test/path?q=portmate", openWindow)).toBe(true);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(openWindow).toHaveBeenCalledWith(
      "https://example.test/path?q=portmate",
      "_blank",
      "noopener,noreferrer",
    );
    expect(popup.opener).toBeNull();
  });

  it("does not consume or open rejected destinations", () => {
    const event = {
      preventDefault: vi.fn(),
    };
    const openWindow = vi.fn(() => null);

    expect(openTerminalWebLink(event, "ftp://example.test/file", openWindow)).toBe(false);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(openWindow).not.toHaveBeenCalled();
  });

  it("rejects unsafe standalone links and contains opener failures", () => {
    const openWindow = vi.fn(() => {
      throw new Error("popup unavailable");
    });

    expect(openIsolatedWebLink("javascript:alert(1)", openWindow)).toBe(false);
    expect(openWindow).not.toHaveBeenCalled();
    expect(openIsolatedWebLink("https://example.test/", openWindow)).toBe(false);
    expect(openWindow).toHaveBeenCalledOnce();
  });
});
