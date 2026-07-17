import { describe, expect, it } from "vitest";
import { flattenSessionTree, protocolTabs, sessionSettingTrees } from "./session-settings-state";

const sharedPages = ["会话", "终端", "日志", "触发器", "传输"];

describe("session settings navigation", () => {
  it("keeps one route for each real profile capability", () => {
    expect(protocolTabs).toEqual(["Shell", "SSH", "Tmux", "Telnet", "Tcp", "Serial"]);
    expect(flattenSessionTree(sessionSettingTrees.Shell)).toEqual([...sharedPages, "Shell"]);
    expect(flattenSessionTree(sessionSettingTrees.SSH)).toEqual([
      ...sharedPages,
      "SSH",
      "代理",
      "验证",
      "代理人",
      "密码",
      "公钥",
    ]);
    expect(flattenSessionTree(sessionSettingTrees.Tmux)).toEqual([
      ...sharedPages,
      "Tmux",
      "代理",
      "验证",
      "代理人",
      "密码",
      "公钥",
    ]);
    expect(flattenSessionTree(sessionSettingTrees.Telnet)).toEqual([...sharedPages, "Telnet", "代理"]);
    expect(flattenSessionTree(sessionSettingTrees.Tcp)).toEqual([...sharedPages, "Tcp", "代理"]);
    expect(flattenSessionTree(sessionSettingTrees.Serial)).toEqual([...sharedPages, "串口"]);
  });

  it("does not expose duplicate or non-runtime settings", () => {
    const removedPages = [
      "Bell",
      "模式",
      "键盘",
      "安全",
      "窗口",
      "选择",
      "自动化",
      "进程",
      "连接",
      "协议",
      "密钥交换",
      "MAC 哈希",
      "X11",
      "SFTP",
      "X/Y/Z Modem",
    ];

    for (const tab of protocolTabs) {
      const pages = flattenSessionTree(sessionSettingTrees[tab]);
      expect(new Set(pages).size).toBe(pages.length);
      expect(pages).not.toEqual(expect.arrayContaining(removedPages));
    }
  });
});
