# PortMate 当前进度与下一阶段目标

审查日期：2026-07-13

本文档对照 [PLAN.md](./PLAN.md) 的最终目标、[README.md](./README.md) 的当前说明、以及当前源码实现，单独记录 PortMate 的实际完成度、缺口和下一阶段目标。

## 审查范围

本次审查覆盖当前仓库内的桌面端、共享核心库、MCP bridge 和项目说明：

- 桌面前端：`src/App.tsx`、`src/api.ts`、`src/types.ts`、`src/styles.css`、`src/sync-input-state.ts`。
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

但它还不是完整 WindTerm/Bitvise 替代品。主要差距集中在：portable-vault/keyring 的系统化跨平台故障矩阵、日志与命令关联、终端兼容性测试、跨协议深度健康检测、HTTP MCP 客户端矩阵、以及系统化跨平台测试。

## 当前实现快照

### 前端桌面工作台

状态：部分完成，可用但仍是 alpha UI。

已实现：

- WindTerm 风格主界面：顶部菜单、会话树、标签页、左侧文件管理器、右侧会话/历史、底部发送区、状态栏。
- 所有主要设置入口使用弹窗，而不是右侧抽屉。
- 新建会话、保存、保存并连接、关闭连接、重连入口已接通。
- 不同协议有不同设置分组：Shell、SSH、Tmux、Telnet、Tcp、Serial。
- `xterm.js` 终端渲染，FitAddon、SearchAddon、WebLinksAddon 已接入。
- 分屏布局有水平/垂直/关闭 pane 的基础实现；版本化 workspace snapshot 会统一保存 pane session binding、active session 和标签颜色，兼容迁移旧 localStorage key，并在 profile 列表变化后剔除失效 ID、收敛无效 split；同一 session 的多 pane 视图会保留，但启动连接目标只执行一次。
- `会话 -> 还原布局` 会重新读取并应用 snapshot；启动模式支持不连接、按上次 pane 或按指定列表顺序连接，自动去重/过滤失效会话并避免凭据弹窗并发覆盖。
- 搜索弹窗支持会话和已加载日志搜索。
- MCP grant 管理弹窗、Transfer/Tunnel/Tmux/Sysmon/Trigger 相关入口已存在。
- 同步输入会把输入按 FIFO 顺序发送到源 pane 和经过协议过滤的已连接 pane；支持按协议换行、0..5000 ms 目标间延迟、显式批量发送各应用一次的受限前后缀、失败/即时取消反馈和明显目标计数。普通 XTerm 键击及原生 bracketed 键盘粘贴保持无前后缀的流式输入，顶部菜单、上下文和中键粘贴走批量路径。源会话始终保留，重复 pane binding 只发送一次；设置持久化但开关每次启动默认关闭。
- 底部发送区、发送次数/间隔/目标、命令历史、Hex 字节发送已接通真实后端。
- 终端交互支持选择即复制、右键/中键粘贴。

主要缺口：

- WindTerm 级别的任意嵌套分屏和快捷键体系还不完整；当前持久化布局仍限定水平/垂直最多 4 pane。
- serialize/unicode/webgl/clipboard 等计划项没有完整接入。
- 自由输入、锁屏等 WindTerm 细节仍不完整。
- 很多全局偏好目前存在于前端 localStorage 或表单状态，没有全部驱动真实后端行为。

### 连接与传输

状态：核心协议可用，深度能力待补。

已实现：

- SSH PTY shell：`russh` 连接、PTY、resize、password/public-key/keyboard-interactive/ssh-agent、profile-vault 私钥、保存密码/口令。
- SSH/Tmux 后台重连每次尝试都会按 session ID 从 store 重新加载并规范化最新 profile；已保存的 endpoint、username、secretRef、identity、Jump Host 与 host-key 策略会用于下一次尝试。握手期间连接配置变化会废弃旧建立结果和旧失败诊断，关闭 reconnect 或把 profile 改为非 SSH transport 会终止 worker；runtime ID 代际校验覆盖 tunnel 恢复和 `Connected` 状态提交，避免已关闭 runtime 被旧任务重新标记为已连接。
- Shell：跨平台 PTY 基础能力，支持自定义程序、参数、cwd。
- Serial：端口枚举、波特率、数据位、停止位、校验、流控、DTR/RTS、Break、文本/Hex 字节发送、读写，活动串口会话可查看最近收发事件的时间戳、方向、Hex 和文本预览；profile 开启 reconnect 后，读线程断开会释放旧端口、进入 `Reconnecting`，并按 1 秒间隔后台重开。每次尝试都会重新加载最新 Profile，端口或线路参数变化会废弃旧尝试并改用新配置，pending/connected 阶段关闭 reconnect 都会收敛到 `Disconnected`；用户关闭或手动重连也会取消旧重连循环。
- Telnet/Raw TCP：socket 模式读写；Telnet 已有增量 IAC 选项协商、分片终端类型子协商、NVT `CR NUL`/CRLF 编解码、Hex/raw byte IAC 转义，以及 Telnet/Raw TCP loopback mock 回归覆盖；协商回复写失败会结束旧 transport 并进入统一断开/重连流程。
- TCP/Telnet：profile 开启 reconnect 后，远端断开会进入 `Reconnecting`，保留可取消的 runtime 占位并按 1 秒间隔后台重连；每次尝试前重新加载最新 Profile，host/port/协议在连接中变化会废弃旧连接并改用新配置，关闭 reconnect 或改成其他 transport 会移除占位并收敛到 `Disconnected`。活动 socket 断开时也读取最新 reconnect flag，不会因捕获的旧值错误进入重连；用户主动关闭或手动重连同样会取消旧循环。loopback 回归覆盖远端立即断开、runtime id 轮换、`Connected -> Reconnecting -> Connected`、断线后切换端口，以及 pending/connected 两种阶段关闭重连。
- Tmux：远端 `list-sessions`、`list-panes`、attach/new-session。
- SFTP：原生 subsystem 浏览、上传、下载、远端复制、递归建目录、递归删除。
- SCP：上传、下载、远端 `cp` 复制。
- X/Y/ZModem：in-band 传输，块级进度与取消已接入；ZModem 使用 `zmodem2`，自动远端传输使用 lrzsz 的 `rx`/`sx`、`rb`/`sb`、`rz`/`sz`，并通过随机 READY/DONE marker 隔离相邻传输尾部字节、在 SSH PTY 上切换 raw TTY。
- SSH tunnel：local、remote reverse、dynamic SOCKS5，桌面端可查看当前会话运行中的 tunnel、停止 tunnel、显示 active/total 连接数、双向字节计数和最后错误；local/dynamic 使用端口 0 时会回填实际监听端口；目标失败会记录错误，后续连接成功会清除 degraded 状态，监听器永久退出会从运行 registry 移除并禁用已保存配置；remote forward 每 15 秒通过远端 Linux `/proc/net/tcp`/`ss`、FreeBSD `sockstat`、macOS `lsof` 或成功执行的 `netstat -ltn` 被动核对监听端口，服务端撤销后会重发原 bind request 并记录恢复事件；存在但参数不兼容的探测工具会回退为 unsupported，不会把空输出误判为监听丢失；Stop 在服务端 cancel 拒绝/超时后仍会清理本地路由/runtime 并把 profile 置为 disabled；SSH channel 断开会移除该会话全部旧 tunnel runtime，自动重连成功后从最新 profile 按原 ID、标签和端口逐条恢复 enabled tunnel，单条恢复失败会保留期望状态、记录事件且不阻断会话和其他 tunnel。

主要缺口：

- Jump Host 后端连接链路已支持多跳，逐跳 host key 验证和 direct-tcpip 串接可用；会话设置可增删多跳 Jump Host，并可为每跳保存独立 password/passphrase secretRef、指定 identityRef、切换继承或自定义 host-key mode/alias/trust scope/rotation/IP 检查；目标 host-key 预扫描可经多跳链路执行；连接失败和扫描时可返回首个需要确认的 Jump Host host key，并通过同一确认弹窗逐跳信任后重连；目标会话临时输入的凭据不会覆盖跳板独立 secretRef。
- GSSAPI 标记为 unsupported。
- Runtime summary 已记录 `lastDisconnect`/`lastDisconnectReason`，SQLite mirror 同步保存，桌面会话工具栏会显示最近断开时间和原因；SSH/TCP/Telnet/Serial 自动重连已有初版，断线后会进入 `Reconnecting` 并后台重试；更深连接健康探测还不完整。
- Serial 的断线重开、最新 Profile 重载和过期尝试拒绝已接入；Hex/时间戳查看已接入活动会话侧栏，但仍缺更完整过滤、导出和独立串口分析窗口。
- SFTP 文件管理已是 local/remote 双栏并支持 Ctrl/Command 切换、Shift 连选、全选、批量删除，以及单项 rename/chmod/属性查看；多文件和完整目录可通过按钮或面板拖拽上传/下载，远端目录由 SFTP 递归枚举并保留空目录、跳过 symlink。内部批次和 Tauri 原生外部拖放共享 `fail/overwrite/skip/rename` 冲突策略，在目标修改前完成类型/路径冲突和超限检查，并跟踪整批 task 终态后刷新目标；local/SFTP/SCP 分块传输已有进度、速度、取消、失败重试、profile 级 B/s 限速和 `.portmate-part` 断点续传（local copy、SFTP upload/download/remote copy、SCP upload/download）；远端命令型复制已有源/目标大小标记、`.portmate-part` 续传、目标大小轮询进度和 channel 级取消；传输任务已改为后台执行并按 session 串行排队，弹窗已有当前会话全量队列视图和批量取消/重试入口。失败任务会显示远端错误、部分进度和失败时间，可复制完整诊断；受限长度的失败原因同时写入 session event，避免只有 `Failed` 状态而没有上下文。更广 SFTP/SCP 服务故障矩阵仍待补。
- ZModem 当前限制单文件小于 4 GiB；OpenSSH PTY 上的 lrzsz X/Y/Z 双向传输已有实测，静默 modem 等待可在取消后立即发送 CAN 并快速退出，transport 进入重连态会立即失败旧 worker；物理串口、OpenSSH 活动传输断线、批量和不同工具实现的兼容矩阵仍待补。

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
- 密钥管理器可以查看 PortMate host key trust store、按 scope/profile 过滤分组、导入/导出 known_hosts、删除 host key、批量删除/复制 host key 到选中 profile、编辑 host key 的 alias/host/port/scope/profile/label；Client Key 区域可搜索并按 profile/source 筛选分组全部 SSH/Tmux profile identity，批量复制到目标 profile、在各自 profile 中置顶或移除引用，被 Jump Host 使用的引用会标记并阻止移除；可从本地文件或粘贴内容导入 OpenSSH 私钥到 profile-vault，并单条或批量添加当前 ssh-agent identity；紧凑 identity inspector 可编辑 label/source/path/fingerprint，展示不可变 ID、Jump Host 和共享 secret 影响，并安全轮换 Vault 私钥或区分“只移除引用/同时清理未共享 secret”。
- 首次连接/host key 变更失败后会弹出专门确认窗口，展示 SHA-256 指纹/已保存指纹，并支持仅本次、加入 Profile、加入 Project、替换 Profile、拒绝和确认后重连。
- `AskEveryTime` 会在每次连接时要求显式确认，不再因永久 trust store 中已有 key 而直接放行；`TrustOnce` 只对精确匹配的 profile/alias/host/port/algorithm/fingerprint 生效。
- 多跳连接的 `TrustOnce` 会保留到整条 Jump Host 链成功建立后再消费，后续跳点确认失败不会导致前一跳重复确认。
- OpenSSH `known_hosts` 导入/导出会保留非 22 端口的 `[host]:port` 形式，并保持 `checkIp` 可匹配规范化 host。

主要缺口：

- 目标 host key 扫描已可经多跳 Jump Host 链路执行；多跳连接链路、每跳独立凭据/host-key 策略编辑和逐跳 host key 确认弹窗可用。
- Bitvise 风格 Host Key Manager / Client Key Manager 已有 Host Key 分组过滤/批量删除/批量复制、host key 字段编辑、Client Key 搜索/分组/批量复制/置顶/安全移除、私钥文件导入、ssh-agent 批量添加、identity 字段检查器和 Vault 私钥轮换。identity 更新/轮换/删除由后端按 immutable ID 原子持久化；共享 secret 不会被原地覆盖或误删，孤儿清理失败只返回 warning，不会留下悬空 Profile 引用。
- IOTA Stronghold portable vault 已接入：使用独立 Argon2id salt 派生 snapshot key，支持创建/解锁/锁定、`stronghold:` secretRef 路由和显式存储选择；自动模式优先 OS keyring，仅在 native 写入失败且 Stronghold 已解锁时 fallback。SQLite 仍只保存引用。已解锁 vault 可在验证当前密码后更换主密码；新密码至少 8 个字符且必须不同，snapshot 使用新派生 key 通过 Stronghold 临时文件提交，提交成功后才替换内存 provider，原 `stronghold:` 引用和 secret 内容保持不变，旧密码无法再解锁。解锁、保存与换密在进程内共用串行化边界；open/save/rekey 另使用跨进程 OS 文件锁并在写前比较已加载 snapshot 的 SHA-256 版本，stale 实例会被拒绝并要求重新解锁，不能用旧 provider 覆盖换密结果。SSH/Tmux 的目标密码、passphrase、Profile Vault 私钥和逐 Jump Host 凭据可按全部或单个 profile 在 native keyring 与 Stronghold 间双向迁移；同源引用只复制一次，Stronghold 批量写只提交一个 snapshot，native 写入会精确读回，预检 token 和 SessionStore revision 在任何目标写入前复核。Profile 采用 copy-on-write 一次提交，旧 secret 仅在全局无引用且没有建连中 reader 时清理。迁移先以 `synchronous=FULL` 写入不含 secret 正文/正文 hash 的 SQLite journal，目标写入只有在 Prepared 精确读回后才开始；Profile KV、mirror、revision 与 `profiles-committed` 是同一事务中的唯一 commit point。重启后按结构化 credential slot 投影分类：全 OLD 只回收未引用目标，全 NEW 在逐项验证目标及源/目标内容一致后继续源清理，混合/缺失/第三种投影或 provider 不可判定时保留两侧并冻结自动恢复。Key Manager 会在打开、vault 锁/解锁和迁移结束后刷新 pending 状态，提供显式“核对并恢复”，pending/corrupt journal 会阻止新的迁移及相关 Profile/secret/identity mutation。恢复面板还能原子导出 JSON + SHA-256 诊断，包含 before/current/after slot、provider 存在性、引用计数和内容一致性布尔值；secret 正文/正文 hash 以及无法验证的原始 corrupt payload 均不会进入文件。MCP token 始终排除在该范围外。

### 数据、日志与自动化

状态：结构化数据可用，日志体系还没达到最终目标。

已实现：

- SQLite `portmate-store.sqlite3` 为主存储，并保留 JSON 兼容导出。
- SessionStore 保存使用跨进程 sidecar 文件锁和绑定 `storeRevision + kv` 内容的 CAS；stale 实例、旧 writer 直接修改 kv、提交后无法验证以及损坏 SQLite 均拒绝继续覆盖。首次创建/旧 JSON 迁移在同一锁内完成，单纯启动第二实例不会旋转 revision。
- SQLite v3 mirror tables：profiles、runtimes、events、transfers、trusted_host_keys、mcp_grants、mcp_audit、timeline_marks、sysmon_snapshots，以及独立的 profile credential migration journal。
- SQLite mirror 在同一事务内更新完整 kv 快照；profiles/runtimes/transfers/keys/grants 等小型可变表重建，events/audit/timeline/sysmon 按主键增量插入并清理已裁剪项，避免日志增长后每次保存重复重写全部大表。
- 会话事件、屏幕文本、传输任务、host keys、MCP grants/audit、timeline、sysmon 都进入统一 store。
- terminal stream 可按 profile 设置追加写入 raw/text/jsonl 分片；入站 Raw 保存 SSH channel、PTY、Raw TCP/Telnet socket 和 Serial 解码前精确字节，出站 Raw 保存成功用户 text/bytes 经协议编码后的 wire、Telnet 协商回复和 modem 帧。每会话 lane 保证出站 transport/write/event 顺序；每个 `bytesRef` 精确绑定对应字节，但双向并发时不承诺共享分片的跨方向因果排序。写成功后的 SQLite/Raw/Text/JSONL 降级会通过事件 `loggingError` 报告，不会伪装成发送失败；二进制结构化事件只保存长度，不复制可逆 Hex。最终分片路径上的追加、读取和删除互斥；新 `bytesRef` 带 segment SHA-256，可拒绝删除重建/修改后的错误内容，同时兼容旧 path/offset/length 引用。新 profile 默认关闭日志和 Raw，UI 明示 Raw 不脱敏。
- direct、连接生命周期、trigger、重连、transfer、tunnel 等 system/control 事件通过 Core 的单槽 wake + 4,096 条有界 outbox 统一写入脱敏 Text/JSONL 并实时 emit；system sink 不创建或追加 Raw，正常运行中每个入队事件只发布一次，退出时会停用共享 notifier、drain 并 join worker。积压超限或 worker 断开会在 store event 写入 `loggingError`，不会无限增长或静默丢弃后续诊断。事件后补 annotations 会按内容变化更新 SQLite `events` mirror，不退化为重复插入全部历史。
- `工具 -> 日志管理` 可安全枚举 raw/txt/jsonl 分片，按路径和格式筛选、查看受限尾部预览（raw/非 UTF-8 使用 Hex）并批量清理；扫描、预览和删除均有数量/大小上限，symlink、路径穿越和非分片扩展不会进入操作范围。
- 日志管理可全文搜索磁盘 Text/JSONL 分片，包括已从有界 store events 裁剪的历史；支持全部或选中路径，结果带分片、行号、字节偏移和受限上下文，并明确报告命中/单文件/总扫描上限与 warning。Raw 保持 Hex 预览，不伪装成文本搜索。
- 日志管理可把最多 1,000 个、合计不超过 512 MiB 的选中 raw/txt/jsonl 分片流式归档为原子落盘的 `.tar.gz`，包内 manifest 记录逐文件 SHA-256，并生成 `.sha256` sidecar；源分片保留不删除，路径穿越、symlink、非法扩展和归档过程中截断的文件会被拒绝。
- 每个 profile 可配置 0..=3650 天自动保留期；旧配置默认关闭。应用启动时后台检查，持续写入时最多每小时复查一次，只按 profile 模板匹配并在删除前二次核对 mtime，随后清理空目录；启用保留期的自定义模板必须含 `{session}` 或 `{profile}`，避免误删共享路径。
- 日志管理可把选中会话导出为原子落盘的 `.tar.gz` 和 `.sha256` sidecar；包内含 bundle JSON、events JSONL、平台/store 诊断和逐文件 SHA-256 manifest。默认脱敏同时覆盖 event text 与 `summary.lastLine`，脱敏开启时强制排除 raw；只有显式关闭脱敏并启用 raw 后才按受限 `bytesRef` 读取片段。
- 触发器支持多个 contains/regex 规则和每条规则的有序多动作编辑；动作包括高亮、通知、时间线标记、本地命令、发送文本、自定义链接和 bell/chime/alert 声音。运行时视觉/声音效果通过 Tauri event 立即送达桌面，本地命令与发送文本保留后端 dispatch，并记录 system event/timeline 诊断。
- secret redaction 有核心测试。

主要缺口：

- 日志与命令关联、毫秒级分片文件，以及 bundle 的签名/自定义附件选择还需增强。
- 传输任务对 local/SFTP/SCP copy loop、远端命令型复制目标大小轮询和 X/Y/ZModem block loop 已有实时进度/速度与取消。

### MCP Surface

状态：stdio bridge 可用，基础 HTTP JSON-RPC 模式可用。

已实现：

- `portmate-mcp` stdio bridge，JSON-RPC lifecycle/tools/resources/prompts。
- MCP protocol version 使用 `2025-06-18`。
- Tools：`list_sessions`、`read_screen`、`tail_log`、`search_logs`、`send_text`、`send_key`、`run_command`、`open_session`、`close_session`、`start_transfer`、`create_tunnel`、`list_tmux_state`、`attach_tmux`、`export_session_bundle`。
- Resources：sessions、state、screen、log、timeline、sysmon、tmux、transfer。
- Prompts：diagnose、serial/SSH compare、repro report。
- 默认只读，写操作通过 MCP grant scope 控制。
- 桌面运行时通过本地 IPC 转发真实控制动作，IPC token 优先存 keyring。
- HTTP 模式通过 `--http` 或 `PORTMATE_MCP_HTTP=1` 启动，仅允许 loopback 绑定，校验 `Origin`，并要求 Bearer 或 `X-PortMate-MCP-Token`；HTTP token 优先来自 `PORTMATE_MCP_HTTP_TOKEN`，否则存入 OS keyring；支持 JSON-RPC POST、streamable-http JSON Accept 兼容、GET SSE 事件流和纯 SSE POST message 事件响应。
- MCP Bridge 弹窗已提供 HTTP endpoint、Origin、启动命令、tokenRef 展示，以及 keyring token 生成/轮换入口。

主要缺口：

- HTTP MCP 已补 `Accept: application/json, text/event-stream` streamable-http JSON 兼容回归，GET `text/event-stream` 基础事件流，以及纯 SSE POST 的 `message` 事件响应；更完整客户端矩阵仍待补。
- MCP 已区分 `resources/list` 实际资源与 `resources/templates/list` URI 模板，支持 `ping`、JSON-RPC batch/notification 语义；HTTP notification 返回无响应体的 `202 Accepted`。
- MCP 与桌面 IPC 都执行日志查询 `limit` 的 1..=1000 边界，日志搜索返回最近命中并按时间正序排列。
- 当 desktop IPC 不可用时，写工具已返回明确未执行错误；后续可考虑队列或离线计划。
- MCP 授权 UI 已有基础 grant 管理，但还缺更细的 per-tool/per-session 审计可视化和授权确认体验。

### 测试与验证

本次审查已执行并通过的基础回归命令：

```bash
cargo fmt --all -- --check
cargo test --workspace -- --test-threads=4
cargo clippy --workspace --all-targets -- -D warnings
npm test -- --run
npm run build
```

`npm run build` 当前有 Vite chunk size warning：主 JS chunk 约 793 kB，功能上不阻断构建，发布前可通过 code splitting 或调整 chunk 策略处理。

已有单元测试覆盖：

- Host key alias 隔离、同 alias key mismatch、多算法 host key。
- Store open/close、profile upsert、MCP write scope、send_text redaction/audit。
- Trigger contains/regex、七类动作前端字段往返、多动作后端 dispatch、自定义链接替换和声音/通知/高亮 runtime effect。
- 同步输入设置归一化、目标协议过滤、换行/显式批量发送前后缀变换、Telnet CRLF、FIFO 批次顺序、交互输入不重复包裹、部分失败和关闭后即时取消剩余目标。
- Secret redaction。
- JSON 风格凭据、完整 Bearer token 脱敏，以及 redacted session bundle。
- 运行时断线诊断跨 store reload 保留。
- 多跳 one-time host key 生命周期与 `AskEveryTime` 强制确认。
- MCP resource/template、ping、batch、notification、HTTP `202` 和日志 limit 协议边界。
- 隔离 OpenSSH 服务上的 TOFU、同地址 host key 变更阻断、`allowRotation` 后重新信任并保留轮换历史、公钥认证、PTY 命令、原生 SFTP 浏览/递归建目录/上传/rename/chmod/属性/远端复制/下载/递归删除、外部目录树递归上传（含空目录）、SFTP/SCP upload/download 与 SFTP remote-copy 的 `.portmate-part` 断点续传、限速 SFTP/SCP 上传取消后从 part 重试、SFTP/SCP 服务端拒写失败状态、传输中 SSH 断开后重连续传，以及 local/dynamic/remote reverse tunnel 的流量统计、目标拒绝、错误状态和原 tunnel 恢复。
- 三 OpenSSH 服务上的两跳 Jump Host direct-tcpip 链、三端独立公钥身份筛选、两跳/目标独立 TOFU 持久化、末端 PTY、第一跳连接拒绝、第二跳 direct-tcpip 拒绝、第一跳/第二跳/目标静默握手超时、第二跳错误 identity 与目标 identity 耗尽的逐端点诊断，以及第二跳 host key 变更诊断。
- 用户态 russh password/keyboard-interactive 跳板与两台独立 OpenSSH 公钥端点组成的两种混合认证链，以及第一跳错误密码诊断和三端 host key 持久化。
- 独立真实 `ssh-agent` 与 OpenSSH 服务上的 agent 禁用、未过滤 offer、`IdentitiesOnly` 空白名单、显式指纹白名单，以及错误指纹不能被相同 comment/path 绕过。
- OpenSSH PTY 上 lrzsz X/Y/ZModem 上传/下载、相邻协议 stale-byte 隔离、raw TTY 恢复，以及 XModem block padding 精确截断。
- OpenSSH `MaxAuthTries 2` 下错误 key 优先导致认证耗尽、逐 identity 错误聚合，以及正确 key 前置后的成功连接。
- `socat` 虚拟 PTY 上的串口二进制收发、断线后切换到最新端口、pending/connected 阶段关闭重连，以及设备不支持 DTR/RTS 时的兼容和拒绝边界。
- SOCKS5 no-auth 协商、domain target 解析、非法认证方式和命令错误回复。
- Client identity source/immutable ID 校验、重复 ID 拒绝、共享 secret 轮换隔离、Jump Host 删除阻断、全凭据引用计数，以及清理失败后已持久化 Profile 仍保持有效。
- Portable Stronghold Argon2id KDF 的密码/salt 绑定，以及 encrypted snapshot 无明文、错误主密码拒绝、正确密码重开、secret 写入/读取/删除、引用格式和 snapshot 缺失 salt 时不覆盖恢复线索的边界；主密码轮换覆盖错误当前密码/短密码/同密码拒绝、旧密码失效、新密码重开、secret 保留、旧密码重新解锁不覆盖新 provider、提交失败时旧 snapshot/provider 仍可用，以及第二个 stale 实例不能覆盖已轮换 snapshot。
- SSH/Tmux profile 凭据迁移覆盖五类 credential slot、SSH/Tmux 显式 scope、共享/legacy native alias、MCP HTTP/IPC 保留引用排除、全部读取预检、Stronghold 单批次提交/回滚/重开、native 精确读回、Profile 提交失败/不确定、源清理 warning、幂等目标、建连中源保留，以及会随迁移计划变化失效的预检 token。恢复测试覆盖 Prepared durable barrier、journal revision/CAS、Profile+journal 原子 checkpoint、结构化槽位 OLD/NEW/MIXED/缺失/交换分类、source missing、provider unavailable、内容 mismatch、Stronghold target unknown、cleanup checkpoint 失败和 corrupt journal 冻结；诊断回归验证结构化冲突证据、原子 JSON/checksum，以及 provider secret 和损坏 payload/state 均不泄漏。
- SessionStore CAS 覆盖 stale 第二实例、未更新 revision 的 kv 变更、损坏 SQLite 加载保护、重复加载不旋转 revision；SQLite mirror schema/trigger 的独立变化不制造逻辑快照冲突。
- 文件选择的单选/Ctrl/Command/Shift 状态转换、批次 `fail/overwrite/skip/rename` 冲突策略和路径逃逸拒绝，以及真实 OpenSSH SFTP 远端目录递归下载、空目录保留和冲突重命名。
- 传输状态本地化、生命周期消息过滤、失败 fallback 和可复制诊断文本，以及中文错误摘要的 UTF-8 安全截断。
- Remote tunnel listener 探测的 Linux `/proc`/`ss`、FreeBSD `sockstat`、macOS `lsof`、BSD `host.port` 和 unsupported 工具回退解析矩阵。
- 日志分片枚举/尾部 UTF-8 与 Hex 预览/批量删除的根目录约束、symlink 跳过、路径穿越拒绝、全量预验证和重复项去重，以及前端路径/格式筛选与全选合并。
- 解码前非 UTF-8 入站 raw、Telnet IAC/NVT 精确分片、48 线程共享路径追加的无重叠引用、带 SHA-256 的 v2 `bytesRef` 删除重建检测和旧引用兼容。
- Telnet 用户 text 的 CRLF wire、用户 bytes/modem 的 IAC doubling、协商 reply 的 outbound/control 无文本事件、每会话出站 lane、不可逆二进制结构化摘要、分片失败诊断，以及 transport 成功后 store 保存失败不触发重发且内存事件注解一致。
- system/control 事件通道覆盖 direct/open/close 生命周期，验证脱敏 Text/JSONL 各恰好一次且 Raw 不变、shutdown drain，并锁定 inbound JSONL 先于由它触发的 system 诊断；Core 回归覆盖 wake 合并、4,096 条 outbox 上限和 worker 断开后的持续显式降级。SQLite mirror 会更新同 ID 事件的后补 annotations，同时不重复插入未变化历史。
- `.tar.gz` session bundle 的原子落盘、逐文件 manifest SHA-256、archive sidecar 校验、脱敏/raw 互斥和 `bytesRef` 范围读取；回归同时覆盖此前遗漏的 `summary.lastLine` 敏感信息泄漏。
- 历史 Text/JSONL 分片全文搜索的大小写不敏感匹配、路径/行号/byte offset、全部/选中范围、raw 排除、查询长度、命中上限和路径穿越边界。
- 通用日志分片归档的流式读取、源文件保留、逐文件 manifest SHA-256、archive sidecar 校验、重复路径去重和路径穿越拒绝。
- profile 日志自动保留的旧配置兼容、模板归属约束、过期 mtime 删除、新分片和其他 profile 隔离，以及空日志根目录边界。

当前 Rust workspace 自动化测试总数为 161：`portmate` 125、`portmate-kdf` 1、`portmate-core` 24、`portmate-mcp` 11；`npm test` 另有 47 个前端 transfer/selection/presentation/log-shard/workspace/trigger/sync-input/secret-migration 单元测试。

主要缺口：

- 已有隔离 OpenSSH server、host key mismatch/`allowRotation`、MaxAuthTries/identity 顺序、真实 ssh-agent 策略/过滤和两跳 Jump Host 集成测试；第一/二跳连接拒绝、三段静默握手超时、逐跳独立 identity 拒绝、目标 identity 耗尽，以及 password/keyboard-interactive 到公钥端点的混合认证链均已覆盖。
- 已有 `socat` 虚拟串口 loopback 二进制收发和 PTY 消失后的自动重连测试，覆盖切换到最新端口路径、runtime ID 轮换、重连期间拒绝写入、恢复后的双向 I/O，以及 connected 阶段关闭 reconnect 后直接断开；真实硬件和 modem 测试矩阵仍待补。
- Telnet/Raw TCP 已有 loopback mock 测试覆盖跨 read 分片的 IAC/TTYPE 子协商、子协商 IAC 转义、NVT `CR NUL`/CRLF 与 EOF 孤立 CR、raw byte IAC 转义、Raw TCP 原样字节发送，以及断线自动重连状态恢复、重连中切换到最新端口、pending/connected 阶段关闭重连的收敛；BINARY/NAWS 等更完整 Telnet 选项和更广服务矩阵仍待补。
- 已有 OpenSSH SFTP 浏览/写操作/传输、SFTP/SCP 五条断点续传路径、SFTP/SCP 取消后 retry、服务端拒写失败状态、活动 SSH 断开后重连续传、lrzsz X/Y/ZModem 双向端到端、静默 XModem 快速取消/CAN 和 transport 重连态旧 worker 快速失败测试；SFTP/SCP 更广服务故障矩阵，以及 modem 物理串口/OpenSSH 活动传输断线/工具变体矩阵仍待补。
- 已有 OpenSSH local/dynamic/remote reverse tunnel 端到端、三种模式目标拒绝后原 tunnel 恢复、remote 失败 channel 主动关闭、服务端撤销 remote forward 后被动探测/原端口重建、重复 cancel 被拒后的本地强制收敛、SSH channel 结束时按 session 清理旧 runtime、自动重连后按原 ID/标签/端口重建和单条端口冲突失败隔离，以及 SOCKS5 错误协议 loopback 测试；`sockstat`/`lsof`/BSD netstat 解析与失败工具回退已有单元矩阵，真实 FreeBSD/macOS SSH 主机仍待纳入集成环境。
- 没有 Playwright UI/截图/交互回归。
- 没有 vttest/xterm 兼容性基线。

## 对照最终目标的完成度

| 目标域 | 当前状态 | 说明 |
| --- | --- | --- |
| 跨平台桌面框架 | 已实现 | Tauri v2 + React/TS + Rust 已成型。 |
| xterm 6 | 已实现 | `@xterm/xterm` 固定 `6.0.0`。 |
| WindTerm 风格工作台 | 部分实现 | 主布局和菜单、最多 4 pane 的水平/垂直布局、版本化 snapshot、pane/active/tab color 恢复、旧 key 迁移和启动会话策略已有；任意嵌套分屏和快捷键体系不足。 |
| 同步输入 | 已实现 | 多 pane 去重广播、额外目标协议过滤、协议感知换行、目标间延迟、显式批量发送前后缀、FIFO、失败/即时取消反馈、明显目标计数和启动默认关闭均已接入，并有前端状态回归。 |
| SSH | 部分实现 | PTY、密码、公钥、keyboard-interactive、ssh-agent、多跳 Jump Host 后端连接链路、每跳独立 secretRef/identityRef 和基础编辑可用；两跳 OpenSSH direct-tcpip、三端独立 identity、逐跳 TOFU、第一/二跳连接拒绝、第一/二跳及目标握手超时、逐端认证失败聚合、第二跳 key mismatch、password/keyboard-interactive 混合链，以及真实 ssh-agent 启用/禁用/过滤矩阵已端到端覆盖，GSSAPI 未完成。 |
| Host key 隔离 | 大部分实现 | profile alias、TOFU、mismatch block、known_hosts 导入导出、连接失败确认弹窗、一次性信任、多跳 Jump Host 目标扫描、多跳连接时逐跳验证、逐跳确认 UX、每跳自定义 host-key 策略已有；高级管理待补。 |
| Bitvise 风格密钥管理 | 大部分实现 | keyring/secretRef、Host Key Manager scope/profile 分组过滤和批量删除/复制、host key 字段编辑、Client Key profile/source 搜索分组、跨 profile 批量复制/置顶/安全移除、私钥文件/粘贴导入、Agent identity 单条/批量添加、identity 字段编辑、Vault 私钥轮换、共享 secret 生命周期保护、Argon2id + IOTA Stronghold portable vault/fallback/主密码轮换，以及带预检 token、durable SQLite journal、原子 commit point、跨重启显式恢复、冲突冻结和安全诊断导出的 SSH/Tmux profile 凭据双向迁移已有；跨平台 provider 故障矩阵待补。 |
| Shell/SSH/Telnet/TCP/Serial | 部分实现 | 基础连接读写、Telnet 增量协商/NVT CR 编解码/TTYPE/raw byte IAC 转义、Telnet/Raw TCP loopback、SSH/TCP/Telnet/Serial 重连加载最新 Profile 并拒绝过期尝试、TCP/Telnet/Serial pending/connected 阶段禁用收敛、虚拟串口切换最新端口自动重连、runtime 最近断开原因可见、break、DTR/RTS、hex 字节发送、串口最近收发 Hex/时间戳查看可用；Telnet 高级选项、深度健康探测和完整 Hex viewer 待补。 |
| Tmux | 部分实现 | list/attach 可用；pane sync 和更完整 tmux workflow 待补。 |
| SFTP/SCP | 部分实现 | 原生 SFTP 和 SCP、双栏、多选/连选/全选、批量删除、rename、chmod、属性查看、面板间及原生外部文件/目录树拖放、远端目录递归下载、空目录保留、安全批次规划、四种冲突策略、retry、速度、local/SFTP/SCP 分块进度与取消、profile 级限速、local/SFTP/SCP upload/download 断点续传、远端命令复制大小标记/目标大小轮询进度/取消和 `.portmate-part` 续传、后台串行队列调度、全量队列视图、批量取消/重试和失败诊断展示已有；真实 OpenSSH 递归上传/下载和冲突重命名已覆盖，更广服务故障矩阵待补。 |
| X/Y/ZModem | 部分实现 | 三者都有实现，块级进度与取消已接入；OpenSSH PTY + lrzsz 六方向传输、raw TTY、READY/DONE 门控、XModem 精确长度、静默对端取消后 CAN/worker 清理和 transport 重连态断线失败已覆盖，物理串口、OpenSSH 活动传输断线和工具变体矩阵待补。 |
| 隧道 | 大部分实现 | local/remote/dynamic、运行中列表、停止入口、连接数/字节/最后错误、监听器终止、Linux/FreeBSD/macOS remote forward 被动探测、撤销后重建、cancel 失败本地收敛、SSH 断线清理和重连后原规格恢复已接入；OpenSSH 三模式、撤销/恢复/停止、重建失败隔离和 SOCKS5 错误协议已覆盖，真实 BSD/macOS 主机和更广服务端矩阵待补。 |
| Sysmon | 部分实现 | 本机/远端 Linux 采样已有；进程、磁盘、网络细节待补。 |
| 日志 | 大部分实现 | 结构化 events/SQLite、双向精确 transport raw、Telnet reply/modem control、system Text/JSONL sink、每会话出站 lane、共享路径串行追加、SHA-256 v2 `bytesRef`、预览/筛选/搜索/清理/保留/归档和可选 raw 的脱敏 session bundle 已有；命令关联与毫秒级分片待补。 |
| 触发器 | 已实现 | 多条 contains/regex 规则、多动作编辑、高亮、通知、时间线、本地命令、发送文本、自定义链接和声音均有模型、运行时 dispatch 与回归覆盖。 |
| MCP stdio | 已实现 | bridge、tools/resources/prompts、grant scope、live IPC 已有。 |
| MCP HTTP | 部分实现 | `portmate-mcp --http` 支持 loopback JSON-RPC、Origin 校验、Bearer/X-Token、本地 keyring token、streamable-http JSON Accept 兼容回归、GET SSE 事件流和纯 SSE POST message 响应；桌面 UI 可展示配置并轮换 token；客户端矩阵待补。 |
| 测试体系 | 部分实现 | core 单测可用；集成、UI、终端兼容测试不足。 |

## 下一阶段目标

### P0：把 alpha 变成稳定可日用版本

1. Client identity 字段编辑、密钥轮换、引用计数生命周期管理、OS keyring 不可用时的 IOTA Stronghold portable vault/fallback、主密码轮换，以及带 durable journal/跨重启核对/安全 conflict 诊断导出的 SSH/Tmux profile 凭据双向批量迁移已完成；继续补 Windows/macOS/Linux 原生 keyring/Stronghold 故障注入矩阵。
2. Jump Host password/keyboard-interactive 混合认证、连接拒绝、三段握手超时与逐端 identity 失败诊断已覆盖。
3. remote forward 服务端撤销的被动探测/原端口重建、cancel 失败后的本地收敛，以及远端命令型传输失败详情、部分进度、事件摘要和复制诊断均已完成；继续扩展服务端故障矩阵。
4. 扩展端到端集成测试：SFTP/SCP 更广服务故障矩阵、Raw TCP/Telnet 更完整矩阵和 modem 的物理串口/OpenSSH 活动传输断线；虚拟串口重连、静默 modem 快速取消和 transport 重连态失败已覆盖。
5. 扩展自动重连、断线恢复和连接健康检测：SSH、TCP/Telnet 与 Serial 会在重试前加载最新 Profile，并在尝试完成时拒绝已过期配置；TCP/Telnet/Serial 在 pending/connected 阶段关闭重连均能立即或下次断线时收敛，runtime 最近断开时间/原因已可见；下一步补更深健康探测。

### P1：补齐 WindTerm/Bitvise 级工作流

1. 文件管理器多选、可配置冲突策略和远端目录递归下载已完成；继续扩展 SFTP/SCP 服务故障矩阵和跨平台路径边界。
2. pane session binding、启动自动连接、标签颜色和 workspace restore 已完成版本化基础实现；继续补任意嵌套分屏。
3. 同步输入正式化已完成：多 pane 去重广播、协议过滤、换行策略、延迟、显式批量发送前后缀、FIFO、失败/即时取消反馈和明显目标计数均已接入；为避免误广播，开关不跨启动保留。
4. 串口工具增强：完整 Hex viewer、收发过滤/导出、重连状态可视化。
5. 密钥管理器继续增强：portable vault 创建/解锁/锁定、主密码轮换、Client identity 字段编辑、密钥轮换、底层 secret 生命周期管理、SSH/Tmux profile 凭据双向批量迁移、migration journal、恢复/重载 UX 和人工 conflict 诊断导出已完成；下一步补跨平台 provider 回归。

### P2：日志、诊断和 MCP 产品化

1. append-only raw/text/jsonl 分片的安全枚举、受限预览、筛选、Text/JSONL 历史全文查询、批量清理 UI、通用归档、profile 自动保留期、双向精确 transport 字节/v2 引用、出站顺序和 system Text/JSONL sink 已完成；继续补命令关联与毫秒级分片。
2. `export_session_bundle` 的桌面 `.tar.gz` 交付包、逐文件/整包校验、平台/store 诊断、默认脱敏和显式 raw 策略已完成；继续补签名和自定义附件选择。
3. MCP HTTP 模式：补 streamable-http 客户端矩阵和更多客户端回归测试。
4. Sysmon 扩展：进程、磁盘、网络接口、远端平台兼容。
5. Playwright UI 回归、vttest/Unicode/鼠标/全屏程序兼容基线。

### P3：架构整理与发布准备

1. 拆分当前 `src-tauri/src/lib.rs`：transport、transfer、mcp、storage、security、terminal 模块化。
2. SQLite 大型追加表已改为增量写入并有 INSERT/DELETE 触发器回归；继续拆分存储模块并评估 kv/JSON 兼容快照的异步化。
3. Stronghold portable vault 已覆盖 OS keyring 不可用/禁用场景、主密码轮换、SSH/Tmux 凭据批量迁移和保守的跨重启恢复；继续把 journal/recovery 与 provider 适配从 `src-tauri/src/lib.rs` 拆为独立 security/storage 模块。
4. 增加 Windows/macOS/Linux 打包验证和权限说明。
5. 建立 release checklist：签名、更新日志、迁移测试、回滚策略。

## 建议的近期执行顺序

1. 集成测试环境加入真实 FreeBSD/macOS SSH tunnel 主机；跨平台探测命令与解析单元矩阵已完成。
2. 会话任意嵌套分屏；基础布局持久化、workspace restore 和启动会话策略已完成。
3. keyring/Stronghold 的 Windows/macOS/Linux 故障注入矩阵；durable migration journal、异常提交核对、重载 UX、双向迁移、跨进程 CAS 和 conflict 诊断导出已完成。
4. 更深连接健康探测和跨平台传输故障矩阵。

这个顺序优先补“真实终端工具的可靠性”和“会话控制的安全边界”，比继续堆 UI 设置项更能降低后续返工。
