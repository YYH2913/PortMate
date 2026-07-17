export const protocolTabs = ["Shell", "SSH", "Tmux", "Telnet", "Tcp", "Serial"] as const;

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
