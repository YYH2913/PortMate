import { describe, expect, it, vi } from "vitest";
import type { ClipboardSelectionType } from "@xterm/addon-clipboard";
import { createWriteOnlyClipboardProvider } from "./terminal-clipboard";

const systemClipboard = "c" as ClipboardSelectionType;

describe("terminal clipboard provider", () => {
  it("allows OSC 52 writes but never exposes clipboard contents", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const provider = createWriteOnlyClipboardProvider({ writeText });

    expect(await provider.readText(systemClipboard)).toBe("");
    await provider.writeText(systemClipboard, "copied by terminal");
    expect(writeText).toHaveBeenCalledWith("copied by terminal");
  });

  it("absorbs unavailable or rejected clipboard writes", async () => {
    const rejected = createWriteOnlyClipboardProvider({ writeText: vi.fn().mockRejectedValue(new Error("denied")) });
    const throwing = createWriteOnlyClipboardProvider({ writeText: vi.fn().mockImplementation(() => { throw new Error("denied"); }) });
    await expect(rejected.writeText(systemClipboard, "text")).resolves.toBeUndefined();
    expect(throwing.writeText(systemClipboard, "text")).toBeUndefined();
    expect(createWriteOnlyClipboardProvider(undefined).writeText(systemClipboard, "text")).toBeUndefined();
  });
});
