import { describe, expect, it } from "vitest";
import { t } from "./i18n";
import {
  filterSessionTree,
  flattenSessionTree,
  MAX_SESSION_PROFILE_GROUP_CHARACTERS,
  MAX_SESSION_PROFILE_NAME_CHARACTERS,
  MAX_SESSION_PROFILE_TAG_CHARACTERS,
  MAX_SESSION_PROFILE_TAGS,
  normalizeSessionMetadataText,
  normalizeSessionProfileMetadata,
  protocolSettingsSection,
  protocolTabs,
  removeJumpSecretDraftIndex,
  sessionSettingTrees,
  validateQuickConnectProfile,
} from "./session-settings-state";
import {
  createSerialConnection,
  createShellConnection,
  createSshConnection,
  createTcpConnection,
} from "./session-profile-helpers";

const sharedPagesAfterProtocol = ["terminal", "logs", "transfers", "triggers"];
const sshAuthPages = ["public-key", "password", "ssh-agent", "proxy", "verification"];

describe("session settings navigation", () => {
  it("keeps one route for each real profile capability", () => {
    expect(protocolTabs).toEqual(["Shell", "SSH", "Tmux", "Telnet", "Tcp", "Serial"]);
    expect(flattenSessionTree(sessionSettingTrees.Shell)).toEqual(["session", "Shell", ...sharedPagesAfterProtocol]);
    expect(flattenSessionTree(sessionSettingTrees.SSH)).toEqual([
      "session",
      "SSH",
      ...sshAuthPages,
      ...sharedPagesAfterProtocol,
    ]);
    expect(flattenSessionTree(sessionSettingTrees.Tmux)).toEqual([
      "session",
      "Tmux",
      ...sshAuthPages,
      ...sharedPagesAfterProtocol,
    ]);
    expect(flattenSessionTree(sessionSettingTrees.Telnet)).toEqual(["session", "Telnet", "proxy", ...sharedPagesAfterProtocol]);
    expect(flattenSessionTree(sessionSettingTrees.Tcp)).toEqual(["session", "Tcp", "proxy", ...sharedPagesAfterProtocol]);
    expect(flattenSessionTree(sessionSettingTrees.Serial)).toEqual(["session", "serial", ...sharedPagesAfterProtocol]);
    expect(protocolSettingsSection("Serial")).toBe("serial");
    expect(protocolSettingsSection("SSH")).toBe("SSH");
  });

  it("filters the session tree by visible labels without dropping parents", () => {
    const tree = sessionSettingTrees.SSH;
    const labels: Record<string, string> = {
      session: "会话",
      terminal: "终端",
      logs: "日志",
      triggers: "触发器",
      transfers: "传输",
      SSH: "SSH",
      proxy: "代理",
      verification: "验证",
      "ssh-agent": "代理人",
      password: "密码",
      "public-key": "公钥",
    };
    const labelOf = (section: string) => labels[section] ?? section;

    expect(flattenSessionTree(filterSessionTree(tree, "公钥", labelOf))).toEqual(["SSH", "public-key"]);
    expect(flattenSessionTree(filterSessionTree(tree, "ssh", labelOf))).toEqual([
      "SSH",
      "public-key",
      "password",
      "ssh-agent",
      "proxy",
      "verification",
    ]);
    expect(filterSessionTree(tree, "不存在", labelOf)).toEqual([]);
  });

  it("does not expose duplicate or non-runtime settings", () => {
    const removedPages = [
      "Bell",
      "模式",
      "键盘",
      "security",
      "窗口",
      "select-2",
      "automation",
      "processes",
      "connect",
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

  it("requires a usable target before a network or serial draft can connect", () => {
    const ssh = createSshConnection();
    expect(validateQuickConnectProfile({ connection: ssh })).toEqual({
      valid: false,
      issues: [{ field: "target", message: t("enter-a-host") }],
    });
    expect(validateQuickConnectProfile({
      connection: { ...ssh, endpoint: { host: "router.local", port: 65_536 } },
    }).issues).toEqual([{ field: "port", message: t("port-must-be-between-1-and-65535") }]);

    const tcp = createTcpConnection("tcp");
    expect(validateQuickConnectProfile({ connection: tcp }).issues.map((issue) => issue.field))
      .toEqual(["target", "port"]);
    expect(validateQuickConnectProfile({
      connection: { ...tcp, host: "10.0.0.5", port: 443 },
    }).valid).toBe(true);

    const serial = createSerialConnection();
    expect(validateQuickConnectProfile({ connection: serial }).issues)
      .toEqual([{ field: "target", message: t("select-a-serial-port") }]);
    expect(validateQuickConnectProfile({
      connection: { ...serial, port: "/dev/ttyUSB0", baudRate: Number.NaN },
    }).issues).toEqual([{ field: "baudRate", message: t("baud-rate-must-be-a-valid-positive-integer") }]);
  });

  it("allows a local shell draft to connect through the platform default shell", () => {
    expect(validateQuickConnectProfile({ connection: createShellConnection() }))
      .toEqual({ valid: true, issues: [] });
  });
});
