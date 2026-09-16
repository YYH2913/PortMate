import { describe, expect, it } from "vitest";
import { t } from "./i18n";
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
  validateQuickConnectProfile,
} from "./session-settings-state";
import {
  createSerialConnection,
  createShellConnection,
  createSshConnection,
  createTcpConnection,
} from "./session-profile-helpers";

const sharedPages = ["session", "terminal", "logs", "triggers", "transfers"];

describe("session settings navigation", () => {
  it("keeps one route for each real profile capability", () => {
    expect(protocolTabs).toEqual(["Shell", "SSH", "Tmux", "Telnet", "Tcp", "Serial"]);
    expect(flattenSessionTree(sessionSettingTrees.Shell)).toEqual([...sharedPages, "Shell"]);
    expect(flattenSessionTree(sessionSettingTrees.SSH)).toEqual([
      ...sharedPages,
      "SSH",
      "proxy",
      "verification",
      "ssh-agent",
      "password",
      "public-key",
    ]);
    expect(flattenSessionTree(sessionSettingTrees.Tmux)).toEqual([
      ...sharedPages,
      "Tmux",
      "proxy",
      "verification",
      "ssh-agent",
      "password",
      "public-key",
    ]);
    expect(flattenSessionTree(sessionSettingTrees.Telnet)).toEqual([...sharedPages, "Telnet", "proxy"]);
    expect(flattenSessionTree(sessionSettingTrees.Tcp)).toEqual([...sharedPages, "Tcp", "proxy"]);
    expect(flattenSessionTree(sessionSettingTrees.Serial)).toEqual([...sharedPages, "serial"]);
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
