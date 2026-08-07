import { describe, expect, it, vi } from "vitest";
import { normalizeTerminalWebLink, openTerminalWebLink } from "./terminal-web-link";

describe("terminal web links", () => {
  it("normalizes only HTTP and HTTPS destinations", () => {
    expect(normalizeTerminalWebLink("HTTPS://example.test/a b")).toBe("https://example.test/a%20b");
    expect(normalizeTerminalWebLink("javascript:alert(1)")).toBeNull();
    expect(normalizeTerminalWebLink("file:///tmp/secret")).toBeNull();
    expect(normalizeTerminalWebLink("not a URL")).toBeNull();
  });

  it("isolates the popup and consumes the terminal mouse event", () => {
    const event = {
      preventDefault: vi.fn(),
      stopImmediatePropagation: vi.fn(),
    };
    const popup = { opener: {} };
    const openWindow = vi.fn(() => popup);

    expect(openTerminalWebLink(event, "https://example.test/path?q=portmate", openWindow)).toBe(true);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();
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
      stopImmediatePropagation: vi.fn(),
    };
    const openWindow = vi.fn(() => null);

    expect(openTerminalWebLink(event, "ftp://example.test/file", openWindow)).toBe(false);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(event.stopImmediatePropagation).not.toHaveBeenCalled();
    expect(openWindow).not.toHaveBeenCalled();
  });
});
