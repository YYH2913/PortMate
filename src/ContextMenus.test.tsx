import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { SessionSummary } from "./types";
import { SessionContextMenu, TerminalContextMenu } from "./ContextMenus";

const originalWindow = globalThis.window;
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
    expect(html).toContain("删除会话 Profile");
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

  it("matches reconnect and disconnect availability to the runtime state", () => {
    const disconnected = renderMenu(false, "disconnected");
    const connected = renderMenu(false, "connected");
    const reconnecting = renderMenu(false, "reconnecting");
    expect(buttonMarkup(disconnected, "断开会话(C)")).toContain("disabled");
    expect(buttonMarkup(connected, "断开会话(C)")).not.toContain("disabled");
    expect(buttonMarkup(disconnected, "重新连接会话(R)")).not.toContain("disabled");
    expect(buttonMarkup(reconnecting, "重新连接会话(R)")).toContain("disabled");
    expect(buttonMarkup(reconnecting, "断开会话(C)")).not.toContain("disabled");
  });

  it("exposes online search and both terminal text export destinations", () => {
    const html = renderToStaticMarkup(
      <TerminalContextMenu
        state={{ x: 100, y: 100, alternate: false, hasSelection: true }}
        onAction={() => {}}
      />,
    );
    expect(html).toContain("在线搜索");
    expect(html).toContain("导出终端文本");
    expect(html).toContain("导出终端文本到...");
  });
});

function renderMenu(syncInput: boolean, status: SessionSummary["runtime"]["status"] = "connected"): string {
  const session = {
    profile: { id: "session-a", name: "Primary" },
    runtime: { status },
  } as SessionSummary;
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

function buttonMarkup(html: string, label: string): string {
  const labelIndex = html.indexOf(label);
  expect(labelIndex).toBeGreaterThanOrEqual(0);
  const start = html.lastIndexOf("<button", labelIndex);
  const end = html.indexOf("</button>", labelIndex);
  return html.slice(start, end + "</button>".length);
}
