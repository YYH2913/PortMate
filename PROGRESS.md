# PortMate 当前进度与下一阶段目标

审查日期：2026-05-18

本文档对照 [PLAN.md](./PLAN.md) 的最终目标、[README.md](./README.md) 的当前说明、以及当前源码实现，单独记录 PortMate 的实际完成度、缺口和下一阶段目标。

## 审查范围

本次审查覆盖当前仓库内的桌面端、共享核心库、MCP bridge 和项目说明：

- 桌面前端：`src/App.tsx`、`src/api.ts`、`src/types.ts`、`src/styles.css`。
- Tauri 后端：`src-tauri/src/lib.rs`、`src-tauri/Cargo.toml`。
- 共享核心：`crates/portmate-core/src/models.rs`、`store.rs`、`host_keys.rs`、`mcp.rs`、`triggers.rs`、`redaction.rs`。
- MCP stdio bridge：`crates/portmate-mcp/src/main.rs`。
- 项目目标和使用说明：`PLAN.md`、`README.md`、`package.json`、workspace `Cargo.toml`。

判断标准是“源码中是否已有真实可执行路径”。只有 UI 菜单或数据模型存在但未接入真实行为的能力，在本文中按“部分实现”或“未完成”记录。

## 总体结论

PortMate 当前已经从“规划原型”推进到“可运行的 alpha 桌面终端工作台”。核心链路已经成立：

- Tauri v2 + React/TypeScript + Rust 桌面应用可构建运行。
- `@xterm/xterm` 已固定为 `6.0.0`。
- SSH、Shell PTY、Serial、Telnet、Raw TCP、Tmux attach/list、SFTP/SCP、X/Y/ZModem、SSH tunnel、Sysmon、触发器、MCP stdio bridge 都已有实际实现。
- SSH host key 已实现 profile 级隔离，不写系统 `known_hosts`，能覆盖“同 IP/端口不同设备/私钥”的核心场景。
- 私钥、可选密码/私钥口令、MCP live IPC token 已接入 OS keyring，SQLite/文件只保存 `secretRef` 或 `tokenRef`。

但它还不是完整 WindTerm/Bitvise 替代品。主要差距集中在：Jump Host、真正完整的传输队列体验、终端兼容性测试、append-only 日志体系、自动重连/队列控制、HTTP MCP、以及系统化集成测试。

## 当前实现快照

### 前端桌面工作台

状态：部分完成，可用但仍是 alpha UI。

已实现：

- WindTerm 风格主界面：顶部菜单、会话树、标签页、左侧文件管理器、右侧会话/历史、底部发送区、状态栏。
- 所有主要设置入口使用弹窗，而不是右侧抽屉。
- 新建会话、保存、保存并连接、关闭连接、重连入口已接通。
- 不同协议有不同设置分组：Shell、SSH、Tmux、Telnet、Tcp、Serial。
- `xterm.js` 终端渲染，FitAddon、SearchAddon、WebLinksAddon 已接入。
- 分屏布局有水平/垂直/关闭 pane 的基础实现。
- 搜索弹窗支持会话和已加载日志搜索。
- MCP grant 管理弹窗、Transfer/Tunnel/Tmux/Sysmon/Trigger 相关入口已存在。
- 同步输入、底部发送区、发送次数/间隔/目标、命令历史、Hex 字节发送已接通真实后端。
- 终端交互支持选择即复制、右键/中键粘贴。

主要缺口：

- WindTerm 级别的任意分屏、布局持久化、启动自动打开会话、标签颜色实际应用、快捷键体系还不完整。
- serialize/unicode/webgl/clipboard 等计划项没有完整接入。
- 自由输入、锁屏等 WindTerm 细节仍不完整。
- 很多全局偏好目前存在于前端 localStorage 或表单状态，没有全部驱动真实后端行为。

### 连接与传输

状态：核心协议可用，深度能力待补。

已实现：

- SSH PTY shell：`russh` 连接、PTY、resize、password/public-key/keyboard-interactive/ssh-agent、profile-vault 私钥、保存密码/口令。
- Shell：跨平台 PTY 基础能力，支持自定义程序、参数、cwd。
- Serial：端口枚举、波特率、数据位、停止位、校验、流控、DTR/RTS、Break、文本/Hex 字节发送、读写。
- Telnet/Raw TCP：socket 模式读写；Telnet 已有最小 IAC 选项协商、终端类型响应和换行编码。
- Tmux：远端 `list-sessions`、`list-panes`、attach/new-session。
- SFTP：原生 subsystem 浏览、上传、下载、远端复制、递归建目录、递归删除。
- SCP：上传、下载、远端 `cp` 复制。
- X/Y/ZModem：in-band 传输；ZModem 使用 `zmodem2`，远端需要 `rz`/`sz`。
- SSH tunnel：local、remote reverse、dynamic SOCKS5。

主要缺口：

- Jump Host 模型存在，但链路未实现。
- GSSAPI 标记为 unsupported。
- 自动重连、断线恢复、连接健康检测还不完整。
- Serial 的 Hex 查看、收发时间戳、自动重连还没有达到专业串口工具级别。
- SFTP 文件管理已是 local/remote 双栏并支持 rename/chmod；属性、断点续传、取消、限速、失败重试、实时速度还缺。
- ZModem 当前限制单文件小于 4 GiB，批量和复杂 rz/sz 兼容矩阵还需要实测。

### SSH 身份与 Host Key 隔离

状态：核心设计已落地，交互还需强化。

已实现：

- `SessionProfile` 持有 `hostKeyPolicy`、`trustedHostKeys[]`、`identityPolicy`、`identityRefs[]`、`agentPolicy`。
- 默认不写系统 `~/.ssh/known_hosts`。
- 支持 `hostKeyAlias`/profile alias，同 IP 同端口不同 profile 不冲突。
- 支持同一 alias 下多 host key algorithm。
- Host key mismatch 默认阻断。
- TOFU 模式会保存到 profile 级 trust。
- client identity 默认按 `IdentitiesOnly` 思路，只尝试 profile/keyring 指定 key，不遍历系统 agent。
- 当 profile 配置 `Profile + Agent` 或显式 agent identity 时，后端会读取本机 ssh-agent 并按 `offerMode` 顺序尝试认证；未关闭 `IdentitiesOnly` 时不会无界遍历全部 agent key。
- 认证成功方法会记录到 `lastSuccessful`。
- 私钥、密码、passphrase、MCP IPC token 进入 OS keyring，SQLite 只保存引用。
- OpenSSH `known_hosts` 导入/导出已接入 Tauri command 和密钥管理器弹窗。
- 密钥管理器可以查看 PortMate host key trust store，并列出当前 ssh-agent 中的 client identities 和 SHA-256 指纹。

主要缺口：

- 首次连接/host key 变更时的专门 fingerprint 确认弹窗还不完整；后端有 `evaluate_host_key`/`apply_host_key_decision`，但 UI 没有完整三路径处理流程。
- Bitvise 风格 Host Key Manager / Client Key Manager 已有基础弹窗，但还缺完整的分组、编辑、复制到 profile、导入私钥文件等高级管理体验。
- Stronghold 作为 OS keyring 不可用时的可移植 fallback 尚未实现。

### 数据、日志与自动化

状态：结构化数据可用，日志体系还没达到最终目标。

已实现：

- SQLite `portmate-store.sqlite3` 为主存储，并保留 JSON 兼容导出。
- SQLite v2 mirror tables：profiles、runtimes、events、transfers、trusted_host_keys、mcp_grants、mcp_audit、timeline_marks、sysmon_snapshots。
- 会话事件、屏幕文本、传输任务、host keys、MCP grants/audit、timeline、sysmon 都进入统一 store。
- 触发器支持 contains/regex，动作包括高亮、通知、时间线标记、本地命令、发送文本。
- secret redaction 有核心测试。

主要缺口：

- 最终目标中的 append-only raw/text/jsonl 分片日志还没完整实现；当前主要是 store events 和 SQLite mirror。
- `bytes_ref`/原始字节流持久化没有完整落地。
- 日志与命令关联、毫秒级分片文件、导出 bundle 的完整离线包还需增强。
- 触发器动作中的播放声音、自定义链接等还不完整。
- 传输任务进度主要是完成后更新，不是全程实时进度/速度。

### MCP Surface

状态：stdio bridge 可用，HTTP 模式未实现。

已实现：

- `portmate-mcp` stdio bridge，JSON-RPC lifecycle/tools/resources/prompts。
- MCP protocol version 使用 `2025-06-18`。
- Tools：`list_sessions`、`read_screen`、`tail_log`、`search_logs`、`send_text`、`send_key`、`run_command`、`open_session`、`close_session`、`start_transfer`、`create_tunnel`、`list_tmux_state`、`attach_tmux`、`export_session_bundle`。
- Resources：sessions、state、screen、log、timeline、sysmon、tmux、transfer。
- Prompts：diagnose、serial/SSH compare、repro report。
- 默认只读，写操作通过 MCP grant scope 控制。
- 桌面运行时通过本地 IPC 转发真实控制动作，IPC token 优先存 keyring。

主要缺口：

- HTTP MCP 模式未实现，因此也没有 HTTP Origin 校验路径。
- 当 desktop IPC 不可用时，部分写工具返回 accepted 文案但不会真正执行；后续应改成明确错误、队列或离线计划。
- MCP 授权 UI 已有基础 grant 管理，但还缺更细的 per-tool/per-session 审计可视化和授权确认体验。

### 测试与验证

本次审查已执行并通过的基础回归命令：

```bash
cargo fmt --all -- --check
cargo check -p portmate -p portmate-mcp
cargo test -p portmate --lib
cargo test -p portmate-core -p portmate-mcp
npm run build
```

`npm run build` 当前有 Vite chunk size warning：主 JS chunk 约 639 kB，功能上不阻断构建，发布前可通过 code splitting 或调整 chunk 策略处理。

已有单元测试覆盖：

- Host key alias 隔离、同 alias key mismatch、多算法 host key。
- Store open/close、profile upsert、MCP write scope、send_text redaction/audit。
- Trigger contains/regex。
- Secret redaction。

主要缺口：

- 没有自动化 SSH server 集成测试。
- 没有虚拟串口 loopback 测试。
- 没有 Telnet/Raw TCP mock 测试。
- 没有 SFTP/SCP/X/Y/ZModem 端到端测试。
- 没有 tunnel 端到端测试。
- 没有 Playwright UI/截图/交互回归。
- 没有 vttest/xterm 兼容性基线。

## 对照最终目标的完成度

| 目标域 | 当前状态 | 说明 |
| --- | --- | --- |
| 跨平台桌面框架 | 已实现 | Tauri v2 + React/TS + Rust 已成型。 |
| xterm 6 | 已实现 | `@xterm/xterm` 固定 `6.0.0`。 |
| WindTerm 风格工作台 | 部分实现 | 主布局和菜单已有，深层交互/快捷键/布局持久化不足。 |
| SSH | 部分实现 | PTY、密码、公钥、keyboard-interactive、ssh-agent 可用；Jump Host、GSSAPI 未完成。 |
| Host key 隔离 | 大部分实现 | profile alias、TOFU、mismatch block、known_hosts 导入导出已有；连接时确认弹窗不足。 |
| Bitvise 风格密钥管理 | 部分实现 | keyring/secretRef、Host Key Manager、Agent identity 列表已有；完整 manager 未完成。 |
| Shell/Telnet/TCP/Serial | 部分实现 | 基础连接读写、Telnet 协商、break、DTR/RTS、hex 字节发送可用；重连、Hex viewer 待补。 |
| Tmux | 部分实现 | list/attach 可用；pane sync 和更完整 tmux workflow 待补。 |
| SFTP/SCP | 部分实现 | 原生 SFTP 和 SCP 已有；双栏、rename、retry、cancel、速度待补。 |
| X/Y/ZModem | 部分实现 | 三者都有实现；需要真实 rz/sz/串口矩阵测试。 |
| 隧道 | 部分实现 | local/remote/dynamic 已有；管理/停止/监控体验待补。 |
| Sysmon | 部分实现 | 本机/远端 Linux 采样已有；进程、磁盘、网络细节待补。 |
| 日志 | 部分实现 | 结构化 events/SQLite 已有；append-only raw/text/jsonl 分片待补。 |
| 触发器 | 部分实现 | 匹配和主要动作已有；声音、自定义链接等待补。 |
| MCP stdio | 已实现 | bridge、tools/resources/prompts、grant scope、live IPC 已有。 |
| MCP HTTP | 未实现 | 127.0.0.1 + Origin/token 模式待做。 |
| 测试体系 | 部分实现 | core 单测可用；集成、UI、终端兼容测试不足。 |

## 下一阶段目标

### P0：把 alpha 变成稳定可日用版本

1. 完整补 Jump Host。
2. 补 host key 首次连接/变更弹窗：展示 SHA-256 fingerprint，并提供临时信任、追加、替换、拒绝。
3. 为 SFTP/SCP/Modem/tunnel 增加真实进度、取消、失败状态和错误可视化。
4. 增加端到端集成测试：测试 SSH server、SFTP server、TCP/Telnet mock、虚拟串口 loopback、rz/sz。
5. 完成自动重连、断线恢复和连接健康检测。

### P1：补齐 WindTerm/Bitvise 级工作流

1. 文件管理器继续增强：拖拽、属性、断点续传、取消/重试、队列控制和速度统计。
2. 会话布局持久化：任意分屏、pane session binding、启动自动打开、标签颜色、workspace restore。
3. 同步输入正式化：多 pane 广播、过滤协议、换行策略、延迟、前后缀、明显状态条。
4. 串口工具增强：Hex viewer、收发时间戳、断线自动重连。
5. 密钥管理器增强：Host Key Manager、Client Key Manager 的编辑、复制到 profile、私钥导入和批量操作。

### P2：日志、诊断和 MCP 产品化

1. 落地 append-only raw/text/jsonl 分片日志，`bytes_ref` 指向真实字节片段。
2. `export_session_bundle` 生成完整可交付包：profile metadata、screen、timeline、audit、sysmon、transfers、日志片段。
3. MCP HTTP 模式：仅绑定 `127.0.0.1`，校验 Origin，本地 token/keyring，提供配置入口。
4. Sysmon 扩展：进程、磁盘、网络接口、远端平台兼容。
5. Playwright UI 回归、vttest/Unicode/鼠标/全屏程序兼容基线。

### P3：架构整理与发布准备

1. 拆分当前 `src-tauri/src/lib.rs`：transport、transfer、mcp、storage、security、terminal 模块化。
2. 将 SQLite mirror 从全量 delete/reinsert 优化为增量写入或 append-only event store。
3. 引入 Stronghold-style portable vault，覆盖 OS keyring 不可用/禁用场景。
4. 增加 Windows/macOS/Linux 打包验证和权限说明。
5. 建立 release checklist：签名、更新日志、迁移测试、回滚策略。

## 建议的近期执行顺序

1. Jump Host。
2. Host key 确认弹窗。
3. 传输队列取消/重试/速度。
4. 集成测试环境。
5. append-only 日志和 session bundle。

这个顺序优先补“真实终端工具的可靠性”和“会话控制的安全边界”，比继续堆 UI 设置项更能降低后续返工。
