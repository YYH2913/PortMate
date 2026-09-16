import { describe, expect, it } from "vitest";
import { workspacePointerAfter, workspacePointerDropZone } from "./use-workspace-pointer-drag";

describe("workspace pointer insertion side", () => {
  it("uses the tab midpoint in LTR and reverses the insertion side in RTL", () => {
    for (const [x, after] of [[99, false], [100, false], [149.9, false], [150, true], [199, true], [201, true]] as const) {
      expect(workspacePointerAfter(x, 100, 100, false), `LTR at ${x}`).toBe(after);
      expect(workspacePointerAfter(x, 100, 100, true), `RTL at ${x}`).toBe(!after);
    }
  });

  it("keeps fractional and narrow tabs' midpoint precise", () => {
    expect(workspacePointerAfter(10.49, 10.25, 0.5, false)).toBe(false);
    expect(workspacePointerAfter(10.5, 10.25, 0.5, false)).toBe(true);
    expect(workspacePointerAfter(10.49, 10.25, 0.5, true)).toBe(true);
    expect(workspacePointerAfter(10.5, 10.25, 0.5, true)).toBe(false);
    expect(workspacePointerAfter(10, 10, 0, false)).toBe(true);
    expect(workspacePointerAfter(10, 10, 0, true)).toBe(false);
  });
});

describe("workspace pointer split zone", () => {
  const rect = { left: 100, top: 200, width: 100, height: 100 };

  it.each([
    [100, 250, "left"],
    [124, 250, "left"],
    [124.01, 250, "center"],
    [150, 250, "center"],
    [175.99, 250, "center"],
    [176, 250, "right"],
    [200, 250, "right"],
    [150, 200, "up"],
    [150, 224, "up"],
    [150, 224.01, "center"],
    [150, 275.99, "center"],
    [150, 276, "down"],
    [150, 300, "down"],
  ] as const)("maps (%s, %s) to physical %s with an inclusive 24%% edge", (x, y, expected) => {
    expect(workspacePointerDropZone(rect, x, y)).toBe(expected);
  });

  it("keeps a usable center and all four edges in a small non-square pane", () => {
    const small = { left: 11, top: 23, width: 10, height: 4 };
    expect(workspacePointerDropZone(small, 16, 25)).toBe("center");
    expect(workspacePointerDropZone(small, 11.5, 25)).toBe("left");
    expect(workspacePointerDropZone(small, 20.5, 25)).toBe("right");
    expect(workspacePointerDropZone(small, 16, 23.2)).toBe("up");
    expect(workspacePointerDropZone(small, 16, 26.8)).toBe("down");
  });

  it("selects the nearest proportional edge at corners with stable ties", () => {
    expect(workspacePointerDropZone(rect, 101, 203)).toBe("left");
    expect(workspacePointerDropZone(rect, 103, 201)).toBe("up");
    expect(workspacePointerDropZone(rect, 199, 297)).toBe("right");
    expect(workspacePointerDropZone(rect, 197, 299)).toBe("down");
    expect(workspacePointerDropZone(rect, 100, 200)).toBe("left");
    expect(workspacePointerDropZone(rect, 200, 200)).toBe("right");
    expect(workspacePointerDropZone(rect, 100, 300)).toBe("left");
    expect(workspacePointerDropZone(rect, 200, 300)).toBe("right");
  });

  it("clamps points outside a pane and handles collapsed dimensions without NaN", () => {
    expect(workspacePointerDropZone(rect, -100, 250)).toBe("left");
    expect(workspacePointerDropZone(rect, 300, 250)).toBe("right");
    expect(workspacePointerDropZone(rect, 150, -100)).toBe("up");
    expect(workspacePointerDropZone(rect, 150, 500)).toBe("down");
    expect(workspacePointerDropZone({ ...rect, width: 0 }, 100, 250)).toBe("left");
    expect(workspacePointerDropZone({ ...rect, height: 0 }, 150, 200)).toBe("up");
    expect(workspacePointerDropZone({ ...rect, width: 0, height: 0 }, 100, 200)).toBe("left");
  });
});
