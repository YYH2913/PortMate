import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { SessionSummary } from "./types";
import { SessionContextMenu } from "./ContextMenus";

const originalWindow = globalThis.window;
const session = {
  profile: { id: "session-a", name: "Primary" },
} as SessionSummary;

beforeAll(() => {
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: { innerWidth: 1440, innerHeight: 900 },
  });
});

afterAll(() => {
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: originalWindow,
  });
});

describe("session context menu", () => {
  it("renders one direct command for each supported action without permanent placeholders", () => {
    const html = renderMenu(false);
    expect(html).toContain("开启同步输入(S)");
    expect(html).toContain("水平拆分视图(H)");
    expect(html).toContain("垂直拆分视图(V)");
    expect(html).toContain("移动视图到分组(M)");
    expect(html).not.toContain("同步输入已开启");
    expect(html).not.toContain("复制SSH通道");
    expect(html).not.toContain("拆分为(S)");
    expect(html).not.toContain("选择分组...");
  });

  it("turns the synchronized-input row into the inverse action when enabled", () => {
    const html = renderMenu(true);
    expect(html).toContain("关闭同步输入(S)");
    expect(html).not.toContain("开启同步输入(S)");
  });
});

function renderMenu(syncInput: boolean): string {
  return renderToStaticMarkup(
    <SessionContextMenu
      state={{ x: 100, y: 100, sessionId: session.profile.id }}
      active={session}
      syncInput={syncInput}
      colors={[]}
      onAction={() => {}}
      onColor={() => {}}
    />,
  );
}
