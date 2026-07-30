import type { SessionProfile } from "./types";

export const protocolTabs = ["Shell", "SSH", "Tmux", "Telnet", "Tcp", "Serial"] as const;
export const MAX_SESSION_PROFILE_NAME_CHARACTERS = 128;
export const MAX_SESSION_PROFILE_GROUP_CHARACTERS = 256;
export const MAX_SESSION_PROFILE_TAGS = 32;
export const MAX_SESSION_PROFILE_TAG_CHARACTERS = 64;
export const MAX_SESSION_PROFILE_TAG_INPUT_CHARACTERS = MAX_SESSION_PROFILE_TAGS * (MAX_SESSION_PROFILE_TAG_CHARACTERS + 2);

export type ProtocolTab = (typeof protocolTabs)[number];
export type SessionTreeNode = { label: string; children?: readonly string[] };

const sharedSessionTree: readonly SessionTreeNode[] = [
  { label: "会话" },
  { label: "终端", children: ["日志"] },
  { label: "触发器" },
  { label: "传输" },
];

export const sessionSettingTrees: Record<ProtocolTab, readonly SessionTreeNode[]> = {
  Shell: [...sharedSessionTree, { label: "Shell" }],
  SSH: [...sharedSessionTree, { label: "SSH", children: ["代理", "验证", "代理人", "密码", "公钥"] }],
  Tmux: [...sharedSessionTree, { label: "Tmux", children: ["代理", "验证", "代理人", "密码", "公钥"] }],
  Telnet: [...sharedSessionTree, { label: "Telnet", children: ["代理"] }],
  Tcp: [...sharedSessionTree, { label: "Tcp", children: ["代理"] }],
  Serial: [...sharedSessionTree, { label: "串口" }],
};

export function flattenSessionTree(tree: readonly SessionTreeNode[]): string[] {
  return tree.flatMap((item) => (item.children ? [item.label, ...item.children] : [item.label]));
}

export function normalizeSessionMetadataText(value: unknown, maxCharacters: number): string {
  if (typeof value !== "string" || maxCharacters <= 0) return "";
  return Array.from(value)
    .filter((character) => !/[\u0000-\u001f\u007f-\u009f]/.test(character))
    .slice(0, maxCharacters)
    .join("");
}

export function normalizeSessionProfileMetadata(
  value: Pick<SessionProfile, "name" | "group" | "tags">,
  fallbackName = "未命名会话",
): Pick<SessionProfile, "name" | "group" | "tags"> {
  const name = boundedTrimmedSessionMetadata(value.name, MAX_SESSION_PROFILE_NAME_CHARACTERS)
    || boundedTrimmedSessionMetadata(fallbackName, MAX_SESSION_PROFILE_NAME_CHARACTERS)
    || "未命名会话";
  const group = boundedTrimmedSessionMetadata(value.group, MAX_SESSION_PROFILE_GROUP_CHARACTERS);
  const tags: string[] = [];
  const seen = new Set<string>();
  for (const rawTag of Array.isArray(value.tags) ? value.tags : []) {
    const tag = boundedTrimmedSessionMetadata(rawTag, MAX_SESSION_PROFILE_TAG_CHARACTERS);
    if (!tag || seen.has(tag)) continue;
    seen.add(tag);
    tags.push(tag);
    if (tags.length >= MAX_SESSION_PROFILE_TAGS) break;
  }
  return { name, group, tags };
}

export function removeJumpSecretDraftIndex(
  drafts: Readonly<Record<string, string>>,
  removedIndex: number,
): Record<string, string> {
  const next: Record<string, string> = {};
  for (const [key, value] of Object.entries(drafts)) {
    const match = /^(\d+):(passwordSecretRef|passphraseSecretRef)$/.exec(key);
    if (!match) continue;
    const index = Number(match[1]);
    if (index === removedIndex) continue;
    next[`${index > removedIndex ? index - 1 : index}:${match[2]}`] = value;
  }
  return next;
}

function boundedTrimmedSessionMetadata(value: unknown, maxCharacters: number): string {
  if (typeof value !== "string") return "";
  const clean = Array.from(value)
    .filter((character) => !/[\u0000-\u001f\u007f-\u009f]/.test(character))
    .join("")
    .trim();
  return Array.from(clean).slice(0, maxCharacters).join("").trim();
}
