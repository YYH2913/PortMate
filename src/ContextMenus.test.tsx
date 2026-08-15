import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { SessionSummary } from "./types";
import { SessionContextMenu, TerminalContextMenu } from "./ContextMenus";
import WorkspaceViewContextMenu from "./WorkspaceViewContextMenu";
import type { WorkspaceView } from "./workspace-state";

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

  it("locks Profile mutation actions while a shortcut save is pending", () => {
    const html = renderMenu(false, "connected", true);
    for (const label of [
      "重命名会话(R)",
      "保存会话(S)",
      "移动视图到分组(M)",
      "会话设置...(S)",
      "删除会话 Profile",
    ]) {
      expect(buttonMarkup(html, label)).toContain("disabled");
    }
    expect(buttonMarkup(html, "复制会话(D)")).not.toContain("disabled");
  });

  it("locks workspace-view Profile actions while a shortcut save is pending", () => {
    const view = {
      id: "view-a",
      sessionId: "session-a",
      title: "Primary",
      color: "",
      keyMode: "remote",
    } as WorkspaceView;
    const html = renderToStaticMarkup(
      <WorkspaceViewContextMenu
        state={{ x: 100, y: 100 }}
        view={view}
        sessionStatus="connected"
        profileBusy
        exportBusy
        label="Primary"
        colors={[]}
        canDuplicate
        canClose
        canCloseOther={false}
        canCloseRight={false}
        canMove={false}
        canMoveToNewGroup={false}
        canDetach={false}
        canClosePane={false}
        canMerge={false}
        canSwap={{ up: false, down: false, left: false, right: false }}
        canZoom={false}
        canReopen={false}
        onColor={() => {}}
        onDuplicate={() => {}}
        onRename={() => {}}
        onAction={() => {}}
      />,
    );
    expect(buttonMarkup(html, "保存会话配置")).toContain("disabled");
    expect(buttonMarkup(html, "会话设置...")).toContain("disabled");
    expect(buttonMarkup(html, "导出终端文本")).toContain("disabled");
    expect(buttonMarkup(html, "导出终端文本到...")).toContain("disabled");
    expect(buttonMarkup(html, "导出选中文本")).toContain("disabled");
    expect(buttonMarkup(html, "复制会话名称")).not.toContain("disabled");
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

  it("locks every terminal text export while one export is pending", () => {
    const html = renderToStaticMarkup(
      <TerminalContextMenu
        state={{ x: 100, y: 100, alternate: false, hasSelection: true }}
        exportBusy
        onAction={() => {}}
      />,
    );
    for (const label of ["导出终端文本", "导出终端文本到...", "导出选中文本"]) {
      expect(buttonMarkup(html, label)).toContain("disabled");
    }
    expect(buttonMarkup(html, "在线搜索")).not.toContain("disabled");
  });
});

function renderMenu(
  syncInput: boolean,
  status: SessionSummary["runtime"]["status"] = "connected",
  profileBusy = false,
): string {
  const session = {
    profile: { id: "session-a", name: "Primary" },
    runtime: { status },
  } as SessionSummary;
  return renderToStaticMarkup(
    <SessionContextMenu
      state={{ x: 100, y: 100, sessionId: session.profile.id }}
      active={session}
      profileBusy={profileBusy}
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
