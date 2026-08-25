# PortMate MCP API

本文档说明当前 PortMate MCP Bridge 对外暴露的能力、调用方式、参数和权限边界。

> 以运行时 tools/list 返回的 schema 为准。工具定义位于
> crates/portmate-core/src/mcp.rs；本文档解释调用语义和常用限制。

## 1. 连接方式

PortMate MCP Bridge 支持 stdio 和 Streamable HTTP，两者使用同一套 JSON-RPC/MCP 工具。

### 1.1 stdio

    {
      "mcpServers": {
        "portmate": {
          "command": "/absolute/path/to/portmate-mcp",
          "args": ["--stdio"],
          "env": {
            "PORTMATE_STORE_PATH": "/absolute/path/to/portmate-store.sqlite3",
            "PORTMATE_MCP_CLIENT_ID": "my-mcp-client"
          }
        }
      }
    }

Windows 将 command 和 PORTMATE_STORE_PATH 替换为 Windows 绝对路径。路径必须指向
运行中的 PortMate 使用的 Store。stdio Bridge 会在每个 JSON-RPC 请求前重新读取 Store
和桌面 IPC endpoint，桌面端重启后通常不需要重启 MCP 客户端。

### 1.2 Streamable HTTP

    PORTMATE_STORE_PATH=/path/to/portmate-store.sqlite3 \\
    PORTMATE_MCP_HTTP=1 \\
    PORTMATE_MCP_HTTP_ADDR=127.0.0.1:8787 \\
    PORTMATE_MCP_CLIENT_ID=my-mcp-client \\
    PORTMATE_MCP_HTTP_TOKEN='<token>' \\
    portmate-mcp --http

请求端点是 POST /mcp。每个请求必须提供 Authorization: Bearer token，或
X-PortMate-MCP-Token: token。

| 变量 | 作用 |
| --- | --- |
| PORTMATE_MCP_HTTP_ADDR | 监听地址，默认 127.0.0.1:8787 |
| PORTMATE_MCP_HTTP_ALLOW_REMOTE=1 | 允许非回环监听；绑定 0.0.0.0 前必须设置 |
| PORTMATE_MCP_HTTP_ORIGINS | 逗号分隔的 Origin allowlist |
| PORTMATE_MCP_HTTP_TOKEN | HTTP Token；未设置时从内部密钥存储读取或生成 |
| PORTMATE_MCP_CLIENT_ID | 当前 MCP Client ID，默认 portmate-local |
| PORTMATE_MCP_TRUSTED=1 | 仅在 Store 尚无任何 grant 时启用可信 bootstrap 写入；已有 grant 后仍严格按 scope、会话范围和审批执行 |
| PORTMATE_MCP_PARENT_PID | 托管 sidecar 的父进程 ID；父进程退出时 sidecar 自动退出 |

HTTP 没有内置 TLS。远程使用时应放在可信网络或正确配置的 TLS 反向代理后面。
HTTP listener 最多同时处理 64 个连接；单个 JSON-RPC envelope 最大约 6 MiB。

CC Switch 的 HTTP 单服务器配置可以直接从桌面端 MCP Bridge 的 HTTP 页面生成；该 JSON
内含 Bearer Token，必须按密码处理，不能提交到仓库或共享日志。

CC Switch 扁平配置示例：

    {
      "portmate": {
        "type": "http",
        "url": "http://192.168.33.222:8787/mcp",
        "headers": { "Authorization": "Bearer <PortMate 生成的 token>" },
        "tool_timeout_sec": 180
      }
    }

该编辑器格式不包含外层 mcpServers；普通 MCP Host 通常仍需要 mcpServers 包装。

### 1.3 JSON-RPC

    {
      "jsonrpc": "2.0",
      "id": 1,
      "method": "tools/call",
      "params": { "name": "list_sessions", "arguments": {} }
    }

常用方法：initialize、tools/list、tools/call、resources/list、resources/read、
resources/templates/list、prompts/list、prompts/get。

支持的 MCP protocolVersion 为 2024-11-05、2025-03-26 和 2025-06-18；未提供时使用
2025-06-18。HTTP POST 使用 Content-Type: application/json，并可通过
MCP-Protocol-Version header 显式选择版本。

tools/call 成功或业务失败均返回 MCP CallToolResult：content 是 text 内容数组，
isError 表示工具是否执行失败。大部分结构化结果以 JSON 字符串放在 content[0].text
中，客户端应先检查 isError，再按 JSON 解析 text。JSON-RPC response 最大 64 MiB。

## 2. 授权模型

每个 MCP Client 由 PORTMATE_MCP_CLIENT_ID 标识。桌面端 工具 -> MCP Bridge -> 授权
中的 grant 决定 scope、允许的 session、到期时间、撤销状态和是否逐次确认写操作。

新建授权默认不允许访问任何会话。界面中的“允许会话”有三个明确模式：
“不授权会话”、“全部会话”和“仅选中会话”。选择单个或多个会话后，
`list_sessions`、无 `sessionId` 的日志/传输查询会返回授权范围内的子集，不会因为
授权只包含一个会话而整体失败；需要具体会话的工具仍必须提交已选中的 `sessionId`。

| Scope | 能力 |
| --- | --- |
| read-sessions | 读取会话摘要和运行状态 |
| read-logs | 读取屏幕、日志、Tmux 状态和诊断包 |
| read-transfers | 读取传输任务；transfer 隐含此权限 |
| read-tunnels | 列出隧道/代理；tunnel 隐含此权限 |
| read-scripts | 列出 MCP 脚本；run-scripts 隐含此权限 |
| read-mcp | 读取 Bridge 和托管 HTTP 状态 |
| write-input | 发送文本、字节、按键、命令、Break 或 Tmux attach |
| transfer | 开始、追加、取消、重试文件传输 |
| tunnel | 创建、停止隧道，以及 TCP/UDP 数据面请求 |
| manage-sessions | 会话生命周期保留权限；当前公开工具不使用 |
| run-scripts | 执行已保存且开放给 MCP 的脚本 |
| manage-mcp | 重启托管 MCP HTTP sidecar |

所有写操作都经过授权后二次校验并进入 MCP 审计。读取结果会移除密码、密钥引用和
敏感本地路径。MCP 不能提交 SSH 密码、私钥、passphrase 或桌面凭据 handle。

## 3. 工具总览

当前 tools/list 暴露 31 个工具。只读对应 readOnlyHint=true；写入对应 false。

| 工具 | 类型 | Scope | 作用 |
| --- | --- | --- | --- |
| list_sessions | 只读 | read-sessions | 列出授权可见会话 |
| mcp_bridge_status | 只读 | read-mcp | Bridge、Store、IPC、HTTP 状态 |
| reload_mcp | 只读 | read-mcp | 重新加载 Bridge 来源 |
| restart_mcp | 写入 | manage-mcp | 重启 HTTP sidecar |
| read_screen | 只读 | read-logs | 读取终端屏幕 |
| tail_log | 只读 | read-logs | 读取最近日志 |
| search_logs | 只读 | read-logs | 搜索日志 |
| send_text | 写入 | write-input | 发送原样文本 |
| send_bytes | 写入 | write-input | 发送 Base64/Hex 原始字节 |
| send_key | 写入 | write-input | 发送受限按键序列 |
| serial_send_break | 写入 | write-input | 发送串口硬件 Break |
| run_command | 写入 | write-input | 发送带协议换行的命令 |
| run_local_command | 写入 | write-input | 在已有 Shell PTY 执行命令 |
| list_custom_scripts | 只读 | read-scripts | 列出脚本摘要 |
| run_custom_script | 写入 | run-scripts | 执行已保存脚本 |
| list_transfers | 只读 | read-transfers | 列出传输任务 |
| get_transfer | 只读 | read-transfers | 查询任务 |
| start_transfer | 写入 | transfer | 启动 SFTP/SCP/TFTP/XModem/YModem/ZModem |
| begin_content_upload | 写入 | transfer | 开始大文件分片暂存 |
| append_content_upload | 写入 | transfer | 追加 Base64 分片 |
| cancel_content_upload | 写入 | transfer | 删除未完成分片上传 |
| cancel_transfer | 写入 | transfer | 取消任务 |
| retry_transfer | 写入 | transfer | 重试任务 |
| create_tunnel | 写入 | tunnel | 创建 SSH 转发或 PortMate 主机代理 |
| list_tunnels | 只读 | read-tunnels | 列出转发和 SOCKS5 代理 |
| stop_tunnel | 写入 | tunnel | 停止转发或代理 |
| tunnel_request | 写入 | tunnel | 发送一次 TCP 请求/响应 |
| udp_request | 写入 | tunnel | 发送一个 UDP 数据报并等待响应 |
| list_tmux_state | 只读 | read-logs | 读取 Tmux 状态 |
| attach_tmux | 写入 | write-input | attach/switch 到 Tmux target |
| export_session_bundle | 只读 | read-logs | 导出脱敏诊断包 |

除非另有说明，sessionId、transferId、tunnelId 最多 128 字节。sessionId 来自
list_sessions；其余 ID 使用对应创建/查询工具的返回值，不应自行猜测。

## 4. 会话、日志和输入

### list_sessions、mcp_bridge_status、reload_mcp、restart_mcp

前 3 个不需要参数。list_sessions 返回脱敏会话摘要；mcp_bridge_status 返回 Bridge
transport、Store、桌面 IPC 和托管 HTTP 状态；reload_mcp 只刷新当前 Bridge，不重启
PortMate。restart_mcp 不需要参数，但需要 manage-mcp；托管 HTTP sidecar 不能在自己的
请求中重启自己，请从 stdio Bridge 或桌面端执行。

export_session_bundle 使用与 read_screen 相同的 sessionId 参数，返回脱敏的会话元数据、
日志和诊断信息；不会包含凭据、私钥或未批准的本地路径。

### read_screen

    { "sessionId": "edge-router" }

必需参数：sessionId。返回当前屏幕文本，可能为空，不是原始字节流。

### tail_log

    { "sessionId": "edge-router", "limit": 100 }

必需 sessionId；limit 可选，默认 100，自动限制为 1-1000。

### search_logs

    { "query": "authentication failed", "sessionId": "edge-router", "limit": 50 }

必需 query；sessionId 可选；limit 默认 100，范围 1-1000。查询不会执行命令。

### send_text

    { "sessionId": "edge-router", "text": "show version" }

必需 sessionId、text。原样写入，不自动追加换行；Telnet 可能按协议转换线上字节。

### send_bytes

    { "sessionId": "board-uart", "encoding": "hex", "data": "55 aa 00 ff" }

必需 sessionId、encoding、data。encoding 只能是 base64 或 hex；解码后最多 4 MiB，
不追加换行。适用于串口帧、Bootloader 数据和 Raw TCP/Telnet 字节；审计只记录摘要。

### send_key

    { "sessionId": "edge-router", "key": "ctrl+c" }

必需 sessionId、key。支持 enter/return、lf、tab、backspace、delete、escape、方向键、
home、end、page-up、page-down、insert、f1-f12、space 和受限 ctrl+字母等形式，不能提交
任意 Escape 字符串。

### serial_send_break

    { "sessionId": "board-uart" }

目标必须是已连接且驱动支持 Break 的 Serial 会话。它不是文本、Enter 或 Ctrl-C。

### run_command 和 run_local_command

两者必需 sessionId、command：

    { "sessionId": "edge-router", "command": "show version" }

run_command 根据会话类型发送到 SSH/Tmux/Shell/Telnet/TCP，并追加协议终止符。
run_local_command 只允许已有的 PortMate Shell Profile 和 live PTY；MCP 不能选择 shell
程序、argv 或 cwd。

## 5. 自定义脚本和 Tmux

### list_custom_scripts

    { "sessionId": "edge-router" }

只返回 id、name、description、updatedAt 等摘要。脚本必须在桌面端保存、开启“开放给
MCP”并允许当前会话；正文不会通过 MCP 返回。

### run_custom_script

    { "sessionId": "edge-router", "scriptId": "550e8400-e29b-41d4-a716-446655440000" }

需要 run-scripts，同时满足脚本 MCP 开关、会话范围、版本未变化和写入审批。MCP 只能
选择已有脚本，不能上传或覆盖脚本正文。

### list_tmux_state

    { "sessionId": "edge-router" }

目标应为已连接 SSH/Tmux 会话，返回脱敏的 session/pane 状态。

### attach_tmux

    { "sessionId": "edge-router", "target": "main" }

必需 sessionId、target。target 会经过非空、无控制字符和长度限制校验，并转换为受限
的 switch-client/attach/new-session 回退命令；不能提交任意 shell 命令。

## 6. 文件传输

通用参数：protocol 只能是 sftp、scp、tftp、xmodem、ymodem、zmodem。传输是异步的，
使用 get_transfer 轮询 status、bytesDone、bytesTotal、message 和结束时间。list_transfers
接受可选 sessionId、limit（默认 100，范围 1-1000）；get_transfer、cancel_transfer、
retry_transfer 只需 transferId。返回的源/目标路径会脱敏，纯 local-to-local 复制不暴露。
SFTP/SCP 的非本地端点只支持 SSH/Tmux Profile；TFTP 和 X/Y/ZModem 还必须在该 Profile
的传输设置中启用，并需要可用的交互会话。

### start_transfer

三种互斥形式：

1. sessionId + protocol + source + destination；
2. sessionId + protocol + fileName + contentBase64 + destination；
3. 只传 uploadId（完成分片后）。

桌面路径示例：

    {
      "sessionId": "edge-router",
      "protocol": "sftp",
      "source": "/home/operator/firmware.bin",
      "destination": "remote:/tmp/firmware.bin"
    }

虚拟 MCP 文件示例：

    {
      "sessionId": "board-uart",
      "protocol": "ymodem",
      "source": { "kind": "mcp", "fileName": "firmware.bin", "contentBase64": "AAECAwQF" },
      "destination": "load:loady"
    }

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| sessionId | string | 传输所属会话；uploadId 最终启动时省略 |
| protocol | enum | 六种传输协议之一 |
| source | string/object | 桌面路径，或 kind=mcp、fileName、contentBase64 |
| fileName | string | 旧版内联来源文件名 |
| contentBase64 | string | 旧版内联 Base64，解码后最多 4 MiB |
| destination | string/object | 本地、remote:/ssh:、load: 或结构化 TFTP |
| uploadId | UUID | begin_content_upload 完成后使用 |

SFTP/SCP 下载可以交换两端，例如 source=remote:/var/log/messages、destination 为本地
路径。Modem 接收端使用 load:loadx、load:loady 或 load:loadz，并支持受限地址/速率参数。

### 结构化 TFTP destination

    {
      "kind": "tftpboot",
      "deviceIp": "192.168.255.1",
      "serverIp": "192.168.255.2",
      "bindHost": "192.168.255.2",
      "bindPort": 0,
      "address": "0x81800000",
      "fileName": "firmware.bin",
      "timeoutSeconds": 120
    }

必需 kind=tftpboot、deviceIp（IPv4）。可选 address、fileName、serverIp、bindHost、
bindPort、timeoutSeconds。address 默认 ${loadaddr}；bindPort=0 自动选择；schema 默认
端口 69，非特权环境建议显式传 0；timeoutSeconds 最小 5 秒且无应用层固定最大值。

PortMate 启动一次性 TFTP 服务，临时设置 ipaddr、serverip、tftpdstp 并执行 tftpboot，
完成后自动关闭，绝不执行 saveenv。服务只接受指定 deviceIp 的 RRQ，只提供选定文件名。
旧式 load:tftpboot?deviceIp=... 仍支持，但新客户端应使用结构化对象。

### 大文件分片

begin_content_upload 必需：sessionId、protocol、fileName、sizeBytes、sha256、destination。
sizeBytes 最大 512 MiB，sha256 为 64 位小写 SHA-256；活动上传合计最大 1 GiB，上传 24
小时后过期。

append_content_upload 必需 uploadId、offset、contentBase64。每个分片解码后最多 4 MiB，
offset 必须等于上次返回的 nextOffset。完成后调用 start_transfer，只传 uploadId。
cancel_content_upload 只需 uploadId。分片暂存不重复触发最终传输审批。

## 7. 隧道、代理和数据面

### create_tunnel

必需 mode、bindHost、bindPort。可选参数如下：

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| mode | local/remote/dynamic | TCP 固定转发、SSH 远端转发或 SOCKS5 |
| bindHost | string | 监听地址 |
| bindPort | integer | 0-65535；0 自动选择 |
| egress | ssh/portmate-host | 出站边界 |
| sessionId | string | SSH/Tmux egress 必需；主机路由省略 |
| targetHost | string | 固定模式必需；dynamic 不填 |
| targetPort | integer | 固定模式 1-65535；dynamic 不填或 0 |
| routeRules | array | dynamic 允许目标，最多 64 条 |
| allowRemoteBind | boolean | 默认 false；非回环主机 listener 必须显式 true |
| label | string | 可选显示/审计标签，最多 128 字符 |

SSH local 示例：

    {
      "egress": "ssh", "sessionId": "edge-router", "mode": "local",
      "bindHost": "127.0.0.1", "bindPort": 15432,
      "targetHost": "db.internal", "targetPort": 5432
    }

PortMate 主机 dynamic 示例：

    {
      "egress": "portmate-host", "mode": "dynamic",
      "bindHost": "127.0.0.1", "bindPort": 0,
      "routeRules": [
        { "host": "192.168.33.0/24", "port": null },
        { "host": "service.internal", "port": 443 }
      ]
    }

egress 未填写时按是否存在 sessionId 推断：有 sessionId 为 ssh，否则为 portmate-host。
SSH egress 需要已连接、已授权的 SSH/Tmux session；portmate-host 不需要 sessionId，
其 local/dynamic 出站来自运行 PortMate 的主机。主机 dynamic 至少需要一条 routeRules；
规则项为 host + port（null 表示任意端口），host 可为域名、*.example.com、IP 或 CIDR。
SSH dynamic 空规则表示允许所有目标。当前 listener 全部是 TCP 语义：没有持久 UDP
association、SOCKS5 UDP ASSOCIATE、组播、广播、DTLS 或 QUIC 会话管理。

### list_tunnels 和 stop_tunnel

list_tunnels 参数均可选 sessionId、egress。egress=ssh 时必须有 sessionId；
egress=portmate-host 时必须省略 sessionId，只列当前 Client 的主机路由。

stop_tunnel 参数为 tunnelId。它关闭当前 Client/授权会话可见的 TCP forward 或 SOCKS5
proxy，不修改操作系统路由表。

### tunnel_request

必需 tunnelId、encoding（base64/hex）、data。可选 targetHost、targetPort、timeoutMs、
maxResponseBytes、closeWrite：

| 参数 | 默认/范围 | 说明 |
| --- | --- | --- |
| timeoutMs | 10000，100-30000 | 请求/响应超时 |
| maxResponseBytes | 4 MiB，1-4 MiB | 最大响应 |
| closeWrite | true | 写完后半关闭请求方向 |
| targetHost/targetPort | dynamic 必需 | 固定路由禁止覆盖目标 |

请求和响应均为一次性 TCP 数据面，不是持久文件流。返回 sentBytes、receivedBytes、
responseBase64、truncated、timedOut。

### udp_request

必需 tunnelId、encoding、data；dynamic 路由还需 targetHost、targetPort；timeoutMs 默认
10000，范围 100-30000。单个 datagram 最大 65507 字节，等待一个响应 datagram 后返回。
它只能承载单个 TFTP/QUIC/DTLS 数据包，不实现协议状态机、持久 UDP 关联或 SOCKS5 UDP
ASSOCIATE。完整 TFTP 使用 start_transfer。

## 8. Resources

通过 resources/list 可见的资源受当前 Client scope 和 session 范围过滤；使用
resources/read 并传 { "uri": "..." }。

| URI | Scope | 内容 |
| --- | --- | --- |
| portmate://sessions | read-sessions | 会话摘要 |
| portmate://sessions/{id}/state | read-sessions | 单会话状态 |
| portmate://sessions/{id}/screen | read-logs | 终端屏幕 |
| portmate://sessions/{id}/log | read-logs | 最近最多 200 行 JSONL |
| portmate://sessions/{id}/timeline | read-logs | 时间线标记 |
| portmate://sessions/{id}/sysmon | read-logs | Sysmon 快照 |
| portmate://sessions/{id}/tmux | read-logs | Tmux 状态 |
| portmate://transfers/{id} | read-transfers | 传输状态 |

ID 会进行 percent-encoding；不要手动拼接未编码的 /、?、# 或 %。

## 9. Prompts

通过 prompts/list 发现，prompts/get 时传 arguments.sessionId。三种 Prompt 都是只读
诊断辅助，不会执行写操作：

| Prompt | 用途 |
| --- | --- |
| diagnose_session | 使用终端快照生成诊断上下文 |
| compare_serial_and_ssh | 对比串口/SSH 行为 |
| prepare_repro_report | 使用日志、时间线、传输和审计信息准备复现报告 |

## 10. 安全和异步边界

- 当前公开 MCP 工具只使用已保存的会话；不会从 MCP 创建或关闭会话。
- run_local_command 只使用用户预先保存的 Shell Profile，不接受 MCP 提交的程序路径、argv 或 cwd。
- run_custom_script 不能从 MCP 上传任意脚本；正文由桌面端保存并单独控制 MCP 开关。
- 内容传输会在桌面端私有目录暂存，完成或取消后清理；文件字节不进入 MCP 审计。
- tunnel listener 不等于操作系统路由；PortMate 退出后主机路由停止且不会恢复。
- TFTP、Modem、文件传输和隧道可能占用 session outbound lane；同一会话终端输入会等待，避免数据交错。
- HTTP Token、桌面 IPC Token、SSH 密码、私钥和 passphrase 不应写入文档、日志、Issue 或共享配置。

传输和隧道通常异步：保存返回的 transferId/tunnelId，随后用 get_transfer/list_tunnels
轮询最终 status、bytesDone、message 或 error。失败时 MCP 返回 isError=true 和脱敏错误文本。

## 11. 已移除的旧工具名

以下旧名称不会出现在 tools/list 中，也不能再调用：

- open_session、close_session：当前 MCP 不暴露会话生命周期工具。
- create_host_route、list_host_routes、stop_host_route：统一使用 create_tunnel、
  list_tunnels、stop_tunnel，并通过 egress=portmate-host 选择主机路由。
- tftp、start_content_transfer、start_content_upload_transfer：统一使用 start_transfer；
  小文件使用虚拟 MCP source，大文件使用 begin_content_upload、append_content_upload
  后再用 uploadId 启动。
