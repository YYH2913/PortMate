import { t } from "./i18n";
import type { SessionProfile } from "./types";

export const protocolTabs = ["Shell", "SSH", "Tmux", "Telnet", "Tcp", "Serial"] as const;
export const MAX_SESSION_PROFILE_NAME_CHARACTERS = 128;
export const MAX_SESSION_PROFILE_GROUP_CHARACTERS = 256;
export const MAX_SESSION_PROFILE_TAGS = 32;
export const MAX_SESSION_PROFILE_TAG_CHARACTERS = 64;
export const MAX_SESSION_PROFILE_TAG_INPUT_CHARACTERS = MAX_SESSION_PROFILE_TAGS * (MAX_SESSION_PROFILE_TAG_CHARACTERS + 2);

export type ProtocolTab = (typeof protocolTabs)[number];
export type SessionTreeNode = { label: string; children?: readonly string[] };
export type QuickConnectField = "target" | "port" | "baudRate";
export type QuickConnectIssue = { field: QuickConnectField; message: string };

const sharedSessionTreeAfterProtocol: readonly SessionTreeNode[] = [
  { label: "terminal" },
  { label: "logs" },
  { label: "transfers" },
  { label: "triggers" },
];

const sshAuthTree: readonly string[] = ["public-key", "password", "ssh-agent", "proxy", "verification"];

export const sessionSettingTrees: Record<ProtocolTab, readonly SessionTreeNode[]> = {
  Shell: [{ label: "session" }, { label: "Shell" }, ...sharedSessionTreeAfterProtocol],
  SSH: [{ label: "session" }, { label: "SSH", children: sshAuthTree }, ...sharedSessionTreeAfterProtocol],
  Tmux: [{ label: "session" }, { label: "Tmux", children: sshAuthTree }, ...sharedSessionTreeAfterProtocol],
  Telnet: [{ label: "session" }, { label: "Telnet", children: ["proxy"] }, ...sharedSessionTreeAfterProtocol],
  Tcp: [{ label: "session" }, { label: "Tcp", children: ["proxy"] }, ...sharedSessionTreeAfterProtocol],
  Serial: [{ label: "session" }, { label: "serial" }, ...sharedSessionTreeAfterProtocol],
};

export function flattenSessionTree(tree: readonly SessionTreeNode[]): string[] {
  return tree.flatMap((item) => (item.children ? [item.label, ...item.children] : [item.label]));
}

export function protocolSettingsSection(protocol: ProtocolTab): string {
  return protocol === "Serial" ? "serial" : protocol;
}

export function sessionSectionLabel(section: string): string {
  return t(section);
}

export function filterSessionTree(
  tree: readonly SessionTreeNode[],
  query: string,
  labelOf: (section: string) => string = sessionSectionLabel,
): SessionTreeNode[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return tree.map((node) => ({ ...node, children: node.children ? [...node.children] : undefined }));
  return tree.flatMap((node) => {
    const selfMatches = labelOf(node.label).toLocaleLowerCase().includes(needle);
    const matchingChildren = (node.children ?? []).filter((child) => (
      selfMatches || labelOf(child).toLocaleLowerCase().includes(needle)
    ));
    if (selfMatches) {
      return [{ label: node.label, children: node.children ? [...node.children] : undefined }];
    }
    if (matchingChildren.length) {
      return [{ label: node.label, children: matchingChildren }];
    }
    return [];
  });
}

export function validateQuickConnectProfile(
  profile: Pick<SessionProfile, "connection">,
): { valid: boolean; issues: QuickConnectIssue[] } {
  const { connection } = profile;
  const issues: QuickConnectIssue[] = [];

  if (connection.kind === "ssh" || connection.kind === "tmux") {
    if (!connection.endpoint.host.trim()) {
      issues.push({ field: "target", message: t("enter-a-host") });
    }
    if (!validNetworkPort(connection.endpoint.port)) {
      issues.push({ field: "port", message: t("port-must-be-between-1-and-65535") });
    }
  } else if (connection.kind === "telnet" || connection.kind === "tcp") {
    if (!connection.host.trim()) {
      issues.push({ field: "target", message: t("enter-a-host") });
    }
    if (!validNetworkPort(connection.port)) {
      issues.push({ field: "port", message: t("port-must-be-between-1-and-65535") });
    }
  } else if (connection.kind === "serial") {
    if (!connection.port.trim()) {
      issues.push({ field: "target", message: t("select-a-serial-port") });
    }
    if (!Number.isInteger(connection.baudRate) || connection.baudRate < 1 || connection.baudRate > 4_294_967_295) {
      issues.push({ field: "baudRate", message: t("baud-rate-must-be-a-valid-positive-integer") });
    }
  }

  return { valid: issues.length === 0, issues };
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
  fallbackName = t("unnamed-session"),
): Pick<SessionProfile, "name" | "group" | "tags"> {
  const name = boundedTrimmedSessionMetadata(value.name, MAX_SESSION_PROFILE_NAME_CHARACTERS)
    || boundedTrimmedSessionMetadata(fallbackName, MAX_SESSION_PROFILE_NAME_CHARACTERS)
    || t("unnamed-session");
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

function validNetworkPort(value: number): boolean {
  return Number.isInteger(value) && value >= 1 && value <= 65_535;
}
