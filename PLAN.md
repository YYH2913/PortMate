# PortMate 全功能终端与 MCP 会话控制规划

## Summary
- 从空仓库新建跨平台桌面应用：`Tauri v2 + React/TypeScript + Rust`，目标 Windows/macOS/Linux。
- 功能参考 WindTerm 与 Bitvise：SSH/SFTP/SCP/Shell/Telnet/Raw TCP/Serial/Tmux、标签页/分屏/会话树、日志、搜索、高亮、触发器、隧道、同步输入、Sysmon、文件传输。
- PortMate 的核心差异是“会话级身份与信任隔离”：同 IP、同端口、不同设备/系统/密钥时，不依赖系统 `~/.ssh/known_hosts` 的单一记录，避免 OpenSSH 常见的 host key 冲突。
- 内置本地 MCP 会话控制层，让外部 AI/MCP Host 读取当前会话、日志、时间线，并在授权白名单内发送控制动作；桌面端本身不内置 AI 助手。

## SSH Auth And Host Key Design
- 区分两类“密钥”：
  - **Host key**：服务器身份公钥。OpenSSH 会把它存进 `known_hosts`，同 IP/域名对应的 host key 变化会触发拒绝访问。
  - **Client identity key**：客户端登录私钥。错误私钥、agent 中密钥过多、未限制身份时，可能导致认证失败或触发服务器 `MaxAuthTries`。
- 参考 Bitvise：
  - 支持 `Host key manager` 和 `Client key manager`。
  - host key 可存到用户级信任库，也可复制到具体 profile；client keypair 也可存到 profile。
  - unattended/CLI 场景可显式传 `hostKeyFile/hostKeyFp/keypairFile`，避免依赖当前系统用户的全局状态。
- 参考 WindTerm：
  - 支持 SSH agent、agent forwarding、OneKey 自动登录，OneKey 可包含 password、public-key、keyboard-interactive。
  - 使用 master password 保护保存的自动登录信息。
- PortMate 设计：
  - 每个 SSH `SessionProfile` 都有独立 `hostKeyPolicy`、`trustedHostKeys[]`、`identityPolicy`、`identityRefs[]`、`agentPolicy`。
  - 默认不写系统 `~/.ssh/known_hosts`，使用 PortMate 自己的 host key store；可选择导入/导出 OpenSSH known_hosts，但不会自动污染系统记录。
  - 支持 `hostKeyAlias` 概念：同一个 `192.168.1.10:22` 可以按 `deviceId/profileId/labSlot` 存多份 host key，适配嵌入式板卡、虚拟机重装、NAT、跳板后多主机场景。
  - 支持同一 profile 绑定多种 host key algorithm 的可信 host key，避免服务器选择不同算法时误判。
  - 首次连接必须展示 SHA-256 fingerprint；用户确认后存入该 profile 或项目级信任域。
  - host key 变更时默认阻断，并给出“替换设备/系统重装/疑似 MITM”三种处理路径：临时信任一次、追加为同 profile 新 host key、替换旧 host key。
  - client identity 默认 `IdentitiesOnly` 行为：只尝试 profile 指定私钥；agent 默认可选但不抢先遍历全部 key。
  - 每个 profile 可配置认证顺序：`publickey -> keyboard-interactive -> password`，并记录实际成功方式供下次优先使用。
  - 密码、passphrase、private key 内容、MCP token 进入系统 keychain/Stronghold；SQLite 只保存引用和 metadata。Portable Stronghold 主密码轮换必须验证当前密码并通过临时文件提交 encrypted snapshot；snapshot open/save/rekey 还必须通过跨进程文件锁和版本校验拒绝 stale provider 覆盖。SSH/Tmux profile 凭据在 keychain/Stronghold 间的批量迁移采用 copy-on-write 引用切换：预检 token 必须绑定准确迁移计划并在写 secret 前验证 SessionStore revision，同一源引用只复制一次，全部目标写入验证和 profile 持久化成功后再清理全局未引用的旧 secret；MCP token 不进入该迁移范围。迁移的 SQLite journal 必须先以 `synchronous=FULL` 提交并精确读回，且不得保存 secret 正文或正文 hash；Profile KV/mirror/revision 与 `profiles-committed` checkpoint 必须在同一事务。该 checkpoint 是唯一 commit point：之前只能回收未引用目标，之后只能在目标可读且源/目标一致时继续源清理；OLD/NEW 以结构化 credential slot 投影判定，MIXED/缺失/第三状态一律保留两侧并转人工核对，provider unavailable 则阻塞本次恢复并保留 journal 供重试。active/corrupt journal 必须冻结新的迁移和相关凭据 mutation，恢复动作必须可跨重启幂等重试。人工诊断导出只能包含引用、slot 投影、provider 可用性/存在性、引用计数和内容是否一致的布尔值，不得导出 secret 正文、正文 hash 或无法验证的原始 journal payload。

## Architecture
- Rust 后端分为 `core`、`transport`、`terminal`、`transfer`、`mcp`、`storage`、`security` 模块；React 前端负责工作台 UI、终端渲染、布局和交互。
- 会话模型统一为 `SessionProfile -> SessionRuntime -> SessionEvent`：所有 SSH、串口、Shell、Telnet、Raw TCP、Tmux pane 都进入同一事件总线，UI 和 MCP 共享同一套命令入口。
- 终端渲染采用 `xterm.js 6` 及 WebGL、search、serialize、unicode、web-links、clipboard、fit 等插件；以 `xterm-256color` 为主兼容目标，并用 vttest/真实程序回归测试跟踪 vt100/vt220/vt340/vt420/vt520 兼容缺口。
- 本地数据存储采用 SQLite + append-only 文件日志：配置、布局、历史命令、触发器、传输任务、MCP 授权写入 SQLite；会话原始流、文本流、时间线、审计日志写入按会话分片文件。

## Key Implementation Changes
- 连接能力：
  - SSH 使用 `russh`/`russh-sftp` 实现交互式 PTY、密码/公钥/keyboard-interactive/agent、Jump Host、端口转发、SFTP/SCP。
  - Shell 使用跨平台 PTY；Windows 支持 PowerShell/CMD/WSL，macOS/Linux 支持默认 shell 和自定义 shell。
  - Serial 支持端口枚举、波特率/数据位/停止位/校验/流控、断线重连、DTR/RTS、Break、十六进制发送/查看。
  - Telnet、Raw TCP、Tmux 集成作为独立 transport；Tmux 先实现 attach/session list/pane sync。
- WindTerm 风格工作台：
  - 会话树、标签页、任意分屏、多工作区窗口、布局恢复、启动自动打开会话、标签颜色、主题、字体、透明度、快捷键、全局搜索。
  - 同步输入支持多 pane 广播，显示明显状态条；可按会话类型过滤换行、延迟、前后缀。
  - 终端交互支持选择即复制、右键/中键粘贴、在线链接识别、搜索打开标签、锁屏、焦点模式。
- 文件传输：
  - SFTP/SCP 提供本地/远端双栏文件管理、上传下载、删除、重命名、新建目录/空文件、队列、失败重试、进度和速度。
  - X/Y/ZModem 进入 v1：支持串口和 SSH 终端内触发，传输任务纳入统一队列和日志。
- 日志、诊断和自动化：
  - 手动/自动会话日志，支持 raw/text/jsonl 三种输出，带毫秒时间戳、来源 session、方向、pane、命令关联。
  - 触发器支持 regex/substring，动作包括高亮、发送文本、运行本地命令、弹通知、播放声音、添加时间线标记、生成自定义链接。
  - Sysmon 支持本机和 SSH 远端 CPU、内存、磁盘、网络、uptime、进程概览；结果进入侧栏和 MCP 资源。

## MCP Surface
- 内置 `portmate-mcp` stdio bridge，供 Claude Desktop、Cursor、Codex 等 MCP Host 启动；bridge 通过本地认证 IPC 连接正在运行的 PortMate 桌面应用。
- MCP 使用官方 `2025-06-18` 基线：JSON-RPC lifecycle、tools、resources、prompts；HTTP 模式默认绑定 `127.0.0.1`，显式启用后可绑定非回环地址，所有监听地址都强制校验 Origin 和 Bearer token。
- Resources：`portmate://sessions`、`portmate://sessions/{id}/state`、`screen`、`log`、`timeline`、`sysmon`、`transfers/{id}`。
- Tools：`list_sessions`、`read_screen`、`tail_log`、`search_logs`、`send_text`、`send_key`、`run_command`、`open_session`、`close_session`、`start_transfer`、`create_tunnel`、`export_session_bundle`。
- 权限策略：默认只读；用户把 MCP client 加入信任白名单后，可按 scope 开启写入、传输、隧道、关闭会话等能力；所有 MCP 写操作记录审计日志并在 UI 显示来源 client。

## Test Plan
- SSH 专项测试：同 IP 不同 host key、同 host 多算法 host key、profile 级 host key alias、host key rotation、agent 禁用/启用、指定 identity 顺序、MaxAuthTries 场景。
- 单元测试：配置 schema、权限判定、触发器匹配、日志分片、时间线关联、secret redaction、Stronghold 主密码换密成功/失败回退、旧密码失效与 `secretRef` 保持，以及 profile 凭据迁移的共享引用、范围隔离、MCP token 排除、批量提交与故障回滚。迁移恢复还需覆盖 Prepared barrier、Profile+journal 原子 commit、revision/CAS、OLD/NEW/MIXED/缺失/槽位交换、source/target missing、内容 mismatch、provider unavailable、Stronghold commit unknown、逐项删除失败、重复恢复和 corrupt/未知版本 journal 冻结；诊断导出需覆盖结构化冲突证据、文件校验和 secret/raw-corrupt-payload 不泄漏。
- 集成测试：虚拟串口 loopback、测试 SSH server、Telnet/Raw TCP mock、SFTP/SCP/X/Y/ZModem 传输、端口转发。
- 终端测试：xterm 兼容序列、Unicode、鼠标、复制粘贴、全屏程序、vttest 基线、长日志性能。
- MCP 测试：初始化、tools/resources/prompts、白名单授权、拒绝未授权写入、stdio bridge、HTTP Origin/token。

## Assumptions And References
- Bitvise 行为参考官方文档：[public key authentication](https://bitvise.com/getting-started-public-key-bitvise)、[first connection and host key verification](https://bitvise.com/getting-started-connect-first-time)、[host key handling/rotation](https://bitvise.com/ssh-server-guide-host-keys)、[unattended profile/key usage](https://bitvise.com/ssh-client-unattended)。
- WindTerm 行为参考官方文档：[SSH OneKey](https://kingtoolbox.github.io/2023/06/07/onekey_ssh_onekey/)、[SSH Agent](https://kingtoolbox.github.io/2020/08/22/ssh_agent/)、[Master Password](https://kingtoolbox.github.io/2021/03/11/protection-master-password/)。
- OpenSSH 冲突处理参考：[ssh_config HostKeyAlias/IdentitiesOnly/IdentityFile](https://man.openbsd.org/ssh_config.5) 与 [known_hosts behavior](https://docs.oracle.com/en/operating-systems/oracle-linux/openssh/openssh-WorkingWithknownhosts.html)。
