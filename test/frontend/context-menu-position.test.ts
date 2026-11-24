import { describe, expect, it } from "vitest";
import { computeContextMenuPosition, computeSubmenuPosition } from "../../src/context-menu-position";

describe("context menu viewport positioning", () => {
  it("flips left and up when opening near bottom-right viewport edge", () => {
    const pos = computeContextMenuPosition(1400, 860, 260, 320, 1440, 900);
    expect(pos.left).toBe(1140);
    expect(pos.top).toBe(540);
  });

  it("clamps within safe viewport margins when menu is taller than available room on either side", () => {
    const pos = computeContextMenuPosition(100, 200, 310, 560, 400, 600);
    expect(pos.left).toBe(82);
    expect(pos.top).toBe(32);
  });

  it("flips submenus horizontally when overflowing right edge in LTR and left edge in RTL", () => {
    const ltrFlipped = computeSubmenuPosition(
      { left: 1180, right: 1420, top: 200, bottom: 228 },
      200,
      180,
      1440,
      900,
      false,
    );
    expect(ltrFlipped.left).toBe(980);
    expect(ltrFlipped.top).toBe(195);

    const rtlFlipped = computeSubmenuPosition(
      { left: 20, right: 260, top: 800, bottom: 828 },
      200,
      180,
      1440,
      900,
      true,
    );
    expect(rtlFlipped.left).toBe(260);
    expect(rtlFlipped.top).toBe(712);
  });
});
