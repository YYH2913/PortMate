import { describe, expect, it } from "vitest";
import {
  flattenSessionTree,
  MAX_SESSION_PROFILE_GROUP_CHARACTERS,
  MAX_SESSION_PROFILE_NAME_CHARACTERS,
  MAX_SESSION_PROFILE_TAG_CHARACTERS,
  MAX_SESSION_PROFILE_TAGS,
  normalizeSessionMetadataText,
  normalizeSessionProfileMetadata,
  protocolTabs,
  removeJumpSecretDraftIndex,
  sessionSettingTrees,
} from "./session-settings-state";

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

  it("bounds display metadata by Unicode characters and strips controls", () => {
    expect(normalizeSessionMetadataText(` ${"😀".repeat(MAX_SESSION_PROFILE_NAME_CHARACTERS + 1)}\n`, MAX_SESSION_PROFILE_NAME_CHARACTERS))
      .toBe(` ${"😀".repeat(MAX_SESSION_PROFILE_NAME_CHARACTERS - 1)}`);

    const normalized = normalizeSessionProfileMetadata({
      name: "  Router\u0000\n  ",
      group: ` Lab\u0085${"g".repeat(MAX_SESSION_PROFILE_GROUP_CHARACTERS)} `,
      tags: [
        " edge ",
        "edge",
        ` ${"t".repeat(MAX_SESSION_PROFILE_TAG_CHARACTERS + 2)} `,
        ...Array.from({ length: MAX_SESSION_PROFILE_TAGS + 4 }, (_, index) => `tag-${index}`),
      ],
    });

    expect(normalized.name).toBe("Router");
    expect(Array.from(normalized.group)).toHaveLength(MAX_SESSION_PROFILE_GROUP_CHARACTERS);
    expect(normalized.tags).toHaveLength(MAX_SESSION_PROFILE_TAGS);
    expect(normalized.tags[0]).toBe("edge");
    expect(normalized.tags[1]).toBe("t".repeat(MAX_SESSION_PROFILE_TAG_CHARACTERS));
    expect(new Set(normalized.tags).size).toBe(normalized.tags.length);
  });

  it("uses a bounded fallback for an empty or control-only name", () => {
    expect(normalizeSessionProfileMetadata({
      name: "\u0000\n",
      group: "",
      tags: [],
    }, ` ${"界".repeat(MAX_SESSION_PROFILE_NAME_CHARACTERS + 2)} `).name)
      .toBe("界".repeat(MAX_SESSION_PROFILE_NAME_CHARACTERS));
  });

  it("drops removed jump secrets and reindexes later drafts", () => {
    expect(removeJumpSecretDraftIndex({
      "0:passwordSecretRef": "first-password",
      "0:passphraseSecretRef": "first-passphrase",
      "1:passwordSecretRef": "second-password",
      "2:passphraseSecretRef": "third-passphrase",
      malformed: "ignored",
    }, 0)).toEqual({
      "0:passwordSecretRef": "second-password",
      "1:passphraseSecretRef": "third-passphrase",
    });
  });
});
