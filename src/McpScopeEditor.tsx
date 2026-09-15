import type { McpScope } from "./types";

const groups: { label: string; scopes: { id: McpScope; label: string; detail: string }[] }[] = [
  { label: "读取权限", scopes: [
    { id: "read-sessions", label: "会话状态", detail: "查看授权范围内的会话" },
    { id: "read-logs", label: "终端日志", detail: "读取与搜索会话日志" },
    { id: "read-transfers", label: "传输状态", detail: "查看文件传输进度" },
    { id: "read-tunnels", label: "隧道状态", detail: "查看可见的转发与代理" },
    { id: "read-scripts", label: "脚本工具", detail: "查看允许此客户端使用的脚本" },
    { id: "read-mcp", label: "Bridge 状态", detail: "查看 MCP 运行与配置信息" },
  ] },
  { label: "操作权限", scopes: [
    { id: "write-input", label: "发送指令", detail: "向授权会话发送输入" },
    { id: "transfer", label: "文件传输", detail: "操作传输，包含传输状态读取" },
    { id: "host-files", label: "主机文件访问", detail: "高风险：额外允许访问 PortMate 主机路径" },
    { id: "tunnel", label: "隧道与代理", detail: "操作转发与代理，包含隧道状态读取" },
    { id: "manage-sessions", label: "会话管理", detail: "管理授权范围内的会话连接" },
    { id: "run-scripts", label: "运行主机脚本", detail: "仅运行明确开放给此客户端的脚本" },
    { id: "manage-mcp", label: "Bridge 管理", detail: "高风险：允许管理 Bridge 运行状态" },
  ] },
];
export const allMcpScopes = groups.flatMap(group => group.scopes.map(scope => scope.id));

export default function McpScopeEditor({ scopes, disabled, onToggle }: {
  scopes: readonly McpScope[];
  disabled: boolean;
  onToggle: (scope: McpScope) => void;
}) {
  return <div className="mcp-scope-groups">{groups.map(group => (
    <fieldset key={group.label} className="mcp-check-grid">
      <legend>{group.label}</legend>
      {group.scopes.map(scope => <label key={scope.id} title={scope.detail}>
        <input type="checkbox" aria-label={scope.id} value={scope.id} disabled={disabled} checked={scopes.includes(scope.id)} onChange={() => onToggle(scope.id)} />
        <span>{scope.label}<code>{scope.id}</code></span>
      </label>)}
    </fieldset>
  ))}</div>;
}
