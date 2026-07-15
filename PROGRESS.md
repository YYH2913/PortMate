# PortMate 当前进度与下一阶段目标

审查日期：2026-07-15

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
- `@xterm/xterm` 已固定为 `6.0.0`，Unicode 11、Serialize、Clipboard 和 WebGL 兼容插件已按验证版本固定。
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
- `xterm.js` 终端渲染已接入 FitAddon、SearchAddon、WebLinksAddon、Unicode11Addon、SerializeAddon、ClipboardAddon 和按需加载的 WebglAddon。`编辑 -> 查找`、`Ctrl/Cmd+F` 和 WindTerm 终端默认键 `Ctrl+Shift+F` 会在当前焦点 pane 打开增量查找条，支持前后匹配、计数、大小写、整词和正则模式；WebGL 初始化失败或 context loss 会回退 DOM renderer；OSC 52 只允许写系统剪贴板，远端读取固定返回空内容。
- 分屏布局使用 v4 递归树，每个 view 有独立 ID、可选别名/颜色和 session binding；同一 session 可在同组拥有多个视图，复制/重命名/着色不会新建连接或修改 Profile，关闭/恢复、精确排序、跨组定点拖放、四方向拆分、整组合并及独立窗口往返都按 view ID 处理。每个 pane 是最多 32 个 view 的有序分组，拖放显示前/后插入位置，跨组目标满载时保持原树不变；WindTerm 2.7 的 12 种预设颜色及清除入口可从 view 右键菜单或窗口菜单使用，并随复制、重载、关闭恢复和独立窗口往返保留。v1 平铺 snapshot、v2 单视图树、v3 session-list 分组和更早 localStorage key 会原位迁移并修复重复 node/view ID。布局支持任意组合的水平/垂直嵌套、关闭叶子后自动折叠父 split，以及最多 16 pane/8 层的明确边界；splitter 可鼠标/触控拖动、方向键调节、Home/End 跳到 15/85% 边界并双击复位，比例会随 group view 列表、active pane/view 和标签颜色一起持久化。WindTerm 2.7 默认键位中的 `Alt+方向键` 几何焦点导航、`Alt+-`/`Alt+\` 向下/向右拆分、Shift 变体向上/向左拆分、`Alt+X` 关闭 active pane 和 `Alt+Z` pane zoom 已接入，并只在 XTerm 工作区持有键盘焦点时拦截；active pane 与真实 XTerm textarea 焦点保持同步。终端设置的快捷键命令表支持录入一段或两段 Ctrl/Alt/Shift/Meta 组合、显式禁用、单项/全部恢复默认和重复/前缀冲突保存阻断；两段 chord 可共享首段，1.2 秒超时、Escape 或错误后缀会清理等待状态并隔离按键，v1 单组合 keymap 自动迁移至 v2，损坏字段回退默认，歧义绑定不会随机执行。pane 标题栏或窗口菜单可把 active view 移至受限 Tauri WebviewWindow；只有创建成功才更新原树，非空来源分组继续保留，独立窗口保留 view ID/别名/颜色并使用同一后端 session，可刷新、连接/断开且不改变主布局，并能返回原分组；浏览器预览使用同源 popup 与校验后的 postMessage。zoom 会隐藏 sibling branch/splitter 但保持其他 XTerm 挂载，方向焦点会连同 zoom 一起移动。窗口菜单还支持把完整 active pane 与上下左右的几何邻居交换；水平拆分对应右侧视图、垂直拆分对应下方视图。profile 列表变化后会剔除失效 session view、收敛空分支。同一 session 的多 view 只产生一个启动连接目标。
- WindTerm `Split View To Group` 的上/下/左/右四个方向已接入窗口菜单；只会从至少含两个 view 的来源分组移出活动 view，达到 16 pane/8 层边界时保持原树不变。
- pane 标签与窗口菜单支持关闭活动/其他/右侧 view，以及当前进程内最近 32 条有界关闭历史的重新打开；关闭 view 不断开后端 session，空 group 自动折叠，最后一个工作区 view 受保护，原 group 消失时恢复到活动非满 group。顶层 session 右键菜单已改为明确的“断开会话”语义。
- WindTerm 锁屏已从占位入口改为真实状态：`模式 -> 锁屏`、状态栏按钮和主/独立终端内的 `Ctrl+Alt+L`/macOS `Meta+Alt+L` 共用从首帧起不透明、焦点封闭的全屏遮罩，不断开会话或停止输出；安全设置可启用启动锁屏和默认 30 分钟、边界 `1..=1440` 分钟的空闲锁屏。只含原因/时间的 v1 本地 marker 让刷新、重启和 detached window 都保持遮罩，存在但损坏的 marker 会保持锁定并由主窗口修复；独立窗口会禁用终端输入并返回主窗口解锁。存在 Portable Vault 时会先锁定 Stronghold、用主密码验证并恢复锁前 provider 状态，当前窗口会话内的刷新也保留该恢复状态；错误消息不暴露后端路径。未配置 vault 或浏览器预览时明确降级为无认证的隐私遮罩。
- WindTerm FreeType 风格自由输入已接入 `模式 -> 自由输入`：只有焦点 pane 打开本地编辑器，草稿按 Unicode 字符限制为 32,768，Enter 原子提交、Shift+Enter 换行、Escape 取消、Ctrl/Meta+Shift+X 剪切选区；提交时统一终端回车并追加一次执行回车，会话切换会清理未提交草稿。自由输入与终端查找互斥，不触发工作区快捷键，并在启用同步输入时复用目标过滤、协议换行、延迟及批量前后缀。
- WindTerm Quick Commands 已接入 `工具 -> 快速命令` 管理器与 `查看 -> 快捷栏`：支持最多 64 条命令的增删改、上下排序、插入文本/追加回车执行两种模式和显式保存/取消；名称与命令正文分别按 Unicode 字符限制为 64/8,192，v1 localStorage 会迁移旧 `{name,text}` 数组并修复非法/重复 ID。调用复用同步输入的原子 FIFO、协议换行、目标、延迟及批量前后缀，执行型命令写入有界历史；Quick Commands 不进入加密凭据 provider，禁止保存密码、token 或私钥。
- WindTerm OneKeys 已接入 `工具 -> OneKeys` 和 `Ctrl+T Ctrl+K`/macOS `Meta+T Meta+K`：Account/SSH 凭据按最多 64 条管理，支持用户名、密码、私钥口令、自动/native/Portable Stronghold Secret 存储、兼容会话绑定、显式保存/删除，以及向当前已连接且已绑定的会话手动发送用户名/密码/口令。持久化摘要只暴露 Secret 是否存在，不返回 Secret 引用或正文；发送走每会话出站 lane，并记录无可读正文的 `one-key` control event。OneKeys 与 localStorage Quick Commands 保持独立。
- `查看` 菜单中的资源管理器、文件管理器、会话、历史命令、发送、快捷栏和状态栏已从无效提示改为带勾选态的真实开关；四个 dock 标题栏和发送面板使用可聚焦的关闭/设置按钮。单侧只剩一个 pane 时自动占满，整侧、发送区或状态栏隐藏后空间会完整归还终端，并与 Quick Bar 正确组合。六项 pane/bar 状态使用有版本的本地快照跨重启恢复，损坏字段独立回退为显示；移动端的响应式隐藏不覆盖桌面选择。`模式 -> 专注模式`、顶部按钮和 WindTerm `Alt+Enter` 会临时隐藏这些区域但不改写持久化选择，退出精确恢复；快捷键只在 XTerm 工作区生效，同步输入开启时强制保留状态栏风险提示。
- `会话 -> 还原布局` 会重新读取并应用 snapshot；启动模式支持不连接、按上次 pane 或按指定列表顺序连接，自动去重/过滤失效会话并避免凭据弹窗并发覆盖。
- 搜索弹窗支持会话和已加载日志搜索。
- MCP grant 管理弹窗、Transfer/Tunnel/Tmux/Trigger 相关入口已存在；Sysmon 已从单行通知升级为 CPU/内存/负载/吞吐概览与进程/磁盘/网络/趋势四标签工作窗口，趋势可切换 CPU/内存利用率与 RX/TX 速率，并提供当前会话可启停、立即采样后每 10 秒刷新的紧凑工具栏 applet。
- 同步输入会把输入按 FIFO 顺序发送到源 pane 和经过协议过滤的已连接 pane；支持按协议换行、0..5000 ms 目标间延迟、显式批量发送各应用一次的受限前后缀、失败/即时取消反馈和明显目标计数。普通 XTerm 键击及原生 bracketed 键盘粘贴保持无前后缀的流式输入，顶部菜单、上下文和中键粘贴走批量路径。源会话始终保留，重复 pane binding 只发送一次；设置持久化但开关每次启动默认关闭。
- 底部发送区、发送次数/间隔/目标、命令历史、Hex 字节发送已接通真实后端。
- 终端交互支持选择即复制、右键/中键粘贴。

主要缺口：

- WindTerm 的 view 精确排序/跨 group 定点拖放和完整 group 合并、逐 view 标签颜色、pane 独立窗口/返回、最多两段的 chord keymap、默认分屏创建、方向焦点移动、关闭、交换、zoom、比例调整和恢复已可用。
- 终端视图切换使用最多 32 个会话、单项 2 MiB、2,000 行 scrollback 的进程内 LRU 序列化缓存，不把屏幕内容写入 localStorage 或磁盘；仍缺 vttest、鼠标协议和全屏程序兼容基线。
- OneKeys 管理、加密 Secret 生命周期、会话绑定和手动发送已完成；仍缺自动识别登录提示并补全、SSH 登录弹窗选择、公钥复用和 keyboard-interactive 集成。
- WindTerm Local/Remote/Normal/Command 键盘模式需要本地导航和命令键表，当前未实现；菜单点击会给出明确缺口，不再误报为已显示视图。
- 很多全局偏好目前存在于前端 localStorage 或表单状态，没有全部驱动真实后端行为。

### 连接与传输

状态：核心协议可用，深度能力待补。

已实现：

- SSH PTY shell：`russh` 连接、PTY、resize、password/public-key/keyboard-interactive/ssh-agent、profile-vault 私钥、保存密码/口令。
- SSH/Tmux/TCP/Telnet 的代理设置已接入真实 transport：每个 Profile 可选择 HTTP CONNECT 或 SOCKS5，并可使用 HTTP Basic 或 SOCKS5 username/password 认证；SOCKS5 使用 domain target，避免在本机解析目标域名。代理密码以事务式 Profile 保存流程写入 native keyring/Stronghold，只持久化 `secretRef`，共享引用计数、孤儿清理、迁移 journal 与恢复投影覆盖四类 Profile。SSH host-key 扫描和正式连接走同一路径；存在 Jump Host 时代理只承载第一条物理连接，后续跳点继续通过 `direct-tcpip`。
- SSH/Tmux 的重连延迟和协议 KeepAlive 已由 Profile 持久化；延迟范围 100-60,000 ms、默认 1,000 ms，等待期间每 100 ms 重读最新 Profile，因此修改延迟或关闭 reconnect 会影响下一次尝试。KeepAlive 可独立开关并配置 1-3,600 秒探测间隔和 1-20 次未响应上限，默认 30/3；正式会话和整条 Jump Host 链使用同一组 russh client 参数。后台重连每次尝试都会按 session ID 从 store 重新加载并规范化最新 profile；已保存的 endpoint、username、secretRef、identity、Jump Host、host-key 策略与健康参数会用于下一次尝试。握手期间连接配置变化会废弃旧建立结果和旧失败诊断，关闭 reconnect 会终止 worker；runtime ID 代际校验覆盖 tunnel 恢复和 `Connected` 状态提交，避免已关闭 runtime 被旧任务重新标记为已连接。
- Shell：跨平台 PTY 基础能力，支持自定义程序、参数、cwd。
- Profile 在 `Connecting`/`Connected`/`Reconnecting` 状态下禁止直接切换协议类型，必须先关闭会话；同协议内的设置仍可保存。Core runtime 会保留真实 `activeTransport` 直到断开或新 transport 确实启动，避免新协议编码、状态和旧 registry 连接交叉。
- Serial：端口枚举、波特率、数据位、停止位、校验、流控、DTR/RTS、Break、文本/Hex 字节发送、读写；每 Profile 保留最多 512 帧/1 MiB 的进程内精确 RX/TX 原始字节捕获，侧栏支持方向、Hex、ASCII 筛选、显式清空，以及把当前可见帧原子导出为未脱敏 JSONL + SHA-256 sidecar。捕获跨自动重连并在断线后保留，但默认不持久化；单帧超过 64 KiB 时明确显示 captured/original 长度。Profile 可配置 100-60,000 ms 自动重连延迟（默认 1,000 ms），等待期间每 100 ms 重读最新 Profile，因此缩短延迟或关闭 reconnect 会及时生效。可选 1-86,400 秒接收空闲超时默认关闭/60 秒，只观察 RX 且不会向设备注入通用 heartbeat；适合预期持续上报的设备。读错误或空闲超时会保留精确断线原因。每次尝试都会重新加载最新 Profile，端口、线路或健康参数变化会废弃旧尝试并改用新配置，pending/connected 阶段关闭 reconnect 都会收敛到 `Disconnected`；用户关闭或手动重连也会取消旧重连循环。
- Telnet/Raw TCP：socket 模式读写；Telnet 已有增量 IAC 选项协商、Profile TERMINAL-TYPE、方向独立的 BINARY、NAWS 初始/持续 resize、NVT `CR NUL`/CRLF 编解码和 Hex/raw byte IAC 转义。BINARY/NAWS 可按 Profile 关闭，旧 Profile 默认开启；协商状态按 runtime 隔离并在重连时重置，协商回复写失败会结束旧 transport 并进入统一断开/重连流程。
- TCP/Telnet：profile 开启 reconnect 后，远端断开会进入 `Reconnecting`，保留可取消的 runtime 占位并按 Profile 延迟（100-60,000 ms，默认 1,000 ms）后台重连；等待期间每 100 ms 检查最新配置，因此修改延迟或关闭 reconnect 会影响下一次尝试。每次连接前重新加载最新 Profile，host/port/协议/代理/健康参数及终端类型/尺寸变化会废弃旧连接并改用新配置，关闭 reconnect 会移除占位并收敛到 `Disconnected`。socket 的 OS keepalive 开关、idle、probe interval 和 retry 均由 Profile 持久化，默认 30/10/3；平台支持相应参数时无需注入协议字节即可检测半开连接。loopback 回归覆盖自定义/关闭内核 keepalive、远端立即断开、runtime id 轮换、`Connected -> Reconnecting -> Connected`、断线后切换端口与缩短延迟、代理端点切换，以及 pending/connected 两种阶段关闭重连。
- Tmux：远端 `list-sessions`、`list-panes`、attach/new-session。
- SFTP：原生 subsystem 浏览、上传、下载、远端复制、递归建目录、递归删除。
- SCP：上传、下载、远端 `cp` 复制。
- X/Y/ZModem：in-band 传输，块级进度与取消已接入；ZModem 使用 `zmodem2`，自动远端传输使用 lrzsz 的 `rx`/`sx`、`rb`/`sb`、`rz`/`sz`，并通过随机 READY/DONE marker 隔离相邻传输尾部字节、在 SSH PTY 上切换 raw TTY。X/YModem sender 会在收到 NAK 或等待 ACK 超时后重发数据块和 EOT，重试次数有界。
- SSH tunnel：local、remote reverse、dynamic SOCKS5，桌面端可查看当前会话运行中的 tunnel、停止 tunnel、显示 active/total 连接数、双向字节计数和最后错误；local/dynamic 使用端口 0 时会回填实际监听端口；目标失败会记录错误，后续连接成功会清除 degraded 状态，监听器永久退出会从运行 registry 移除并禁用已保存配置；remote forward 每 15 秒通过远端 Linux `/proc/net/tcp`/`ss`、FreeBSD `sockstat`、macOS `lsof` 或成功执行的 `netstat -ltn` 被动核对监听端口，服务端撤销后会重发原 bind request 并记录恢复事件；存在但参数不兼容的探测工具会回退为 unsupported，不会把空输出误判为监听丢失；Stop 在服务端 cancel 拒绝/超时后仍会清理本地路由/runtime 并把 profile 置为 disabled；SSH channel 断开会移除该会话全部旧 tunnel runtime，自动重连成功后从最新 profile 按原 ID、标签和端口逐条恢复 enabled tunnel，单条恢复失败会保留期望状态、记录事件且不阻断会话和其他 tunnel。

主要缺口：

- Jump Host 后端连接链路已支持多跳，逐跳 host key 验证和 direct-tcpip 串接可用；会话设置可增删多跳 Jump Host，并可为每跳保存独立 password/passphrase secretRef、指定 identityRef、切换继承或自定义 host-key mode/alias/trust scope/rotation/IP 检查；目标 host-key 预扫描可经多跳链路执行；连接失败和扫描时可返回首个需要确认的 Jump Host host key，并通过同一确认弹窗逐跳信任后重连；目标会话临时输入的凭据不会覆盖跳板独立 secretRef。
- GSSAPI 标记为 unsupported。
- Runtime summary 已记录 `lastDisconnect`/`lastDisconnectReason`，SQLite mirror 同步保存，桌面会话工具栏会显示最近断开时间和原因；SSH/Tmux 的重连延迟/协议 KeepAlive、TCP/Telnet 的 OS keepalive/重连延迟，以及 Serial 的重连延迟/接收空闲阈值均可按 Profile 配置，断线后会进入 `Reconnecting` 并后台重试；更广故障矩阵和更深连接健康诊断还不完整。
- Serial 的断线重开、最新 Profile 重载和过期尝试拒绝已接入；精确 Hex/ASCII、时间戳、方向过滤、增量查询、清空与 JSONL 导出已接入会话侧栏，但仍缺独立串口分析窗口、帧协议解析和更大数据集工作流。
- SFTP 文件管理已是 local/remote 双栏并支持 Ctrl/Command 切换、Shift 连选、全选、批量删除，以及单项 rename/chmod/属性查看；多文件和完整目录可通过按钮或面板拖拽上传/下载，远端目录由 SFTP 递归枚举并保留空目录、跳过 symlink。内部批次和 Tauri 原生外部拖放共享 `fail/overwrite/skip/rename` 冲突策略，在目标修改前完成类型/路径冲突和超限检查，并跟踪整批 task 终态后刷新目标；local/SFTP/SCP 分块传输已有进度、速度、取消、失败重试、profile 级 B/s 限速和 `.portmate-part` 断点续传（local copy、SFTP upload/download/remote copy、SCP upload/download），限速等待会让出 async runtime 并每 100 ms 检查取消；远端命令型复制已有源/目标大小标记、`.portmate-part` 续传、目标大小轮询进度和 channel 级取消；传输任务已改为后台执行并按 session 串行排队，弹窗已有当前会话全量队列视图和批量取消/重试入口。失败任务会显示远端错误、部分进度和失败时间，可复制完整诊断；受限长度的失败原因同时写入 session event，避免只有 `Failed` 状态而没有上下文。更广 SFTP/SCP 服务故障矩阵仍待补。
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
- SQLite v4 mirror tables：profiles、runtimes、events、transfers、trusted_host_keys、mcp_grants、mcp_audit、timeline_marks、带结构化 `details_json` 的 sysmon_snapshots，以及独立的 profile credential migration journal；v3 Sysmon 表会原位增加默认空 details 列。
- SQLite mirror 在同一事务内更新完整 kv 快照；profiles/runtimes/transfers/keys/grants 等小型可变表重建，events/audit/timeline/sysmon 按主键增量插入并清理已裁剪项，避免日志增长后每次保存重复重写全部大表。
- 会话事件、屏幕文本、传输任务、host keys、MCP grants/audit、timeline、sysmon 都进入统一 store。events 每会话保留目标 5,000 条并使用 512 条实时批量裁剪余量；审计按 session/global scope 保留目标 5,000 条、timeline 每会话 2,000 条、Sysmon 每会话 1,024 条，并使用 128 条余量；终态 transfer 每会话精确保留 1,000 条，queued/running 永不淘汰。桌面和独立 MCP 加载旧快照时会把每个 event/history scope 收敛到精确上限并重建 event count cache，防止未继续产生日志的旧会话绕过边界，也避免 KV JSON、内存和 mirror 同步成本无限增长。
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
- stdio 每条 newline-delimited JSON payload 上限为 1 MiB（不含 `LF/CRLF`）；超限行仅保留 `limit + 2` 字节并有界丢弃到换行，返回 JSON-RPC parse error 后可继续处理下一条消息，不会无界分配或协议失步。
- stdio/HTTP 共用的 JSON-RPC envelope 会保留显式 null ID，拒绝对象/数组/布尔 ID 和非结构化 `params`；batch 在任何 tool dispatch 前限制为 128 项，避免小请求放大为无界调用与响应。
- stdio/HTTP JSON-RPC 响应与 SSE JSON 数据共用 64 MiB 有界 writer，写入将在追加越界前停止。单响应超限会返回保留原 ID 的 `-32603`；batch 或 SSE 状态超限会以不含原大 payload 的受限错误替换，避免日志、screen 或状态数据造成无界二次序列化和输出。
- MCP protocol version 使用 `2025-06-18`。
- Tools：`list_sessions`、`read_screen`、`tail_log`、`search_logs`、`send_text`、`send_key`、`run_command`、`open_session`、`close_session`、`start_transfer`、`create_tunnel`、`list_tmux_state`、`attach_tmux`、`export_session_bundle`。
- Resources：sessions、state、screen、log、timeline、sysmon、tmux、transfer。
- Prompts：diagnose、serial/SSH compare、repro report。
- 默认只读，写操作通过 MCP grant scope 控制。
- live desktop Store 是写授权的最终来源；bridge 不再用可能陈旧的本地快照提前拒绝。通过 IPC token 认证的写尝试会按 client ID、真实 tool、session、scope 和 `invalid`/`denied`/`authorized`/`succeeded`/`failed` 结果持久化审计，授权记录在副作用发生前先提交；审计不复制原始参数、命令文本、密码、passphrase 或路径正文。MCP 出站事件的 actor 也使用真实 client ID，不再误记为 `desktop-user` 或额外生成 `send_text` 审计。显式 grant 按配置 scope 生效，`PORTMATE_MCP_TRUSTED=1` 仅额外允许空 grant Store 的本地开发 bootstrap。
- 桌面 MCP IPC 请求体上限为 1 MiB，完整读取和响应写出各有 5 秒超时；超限、慢速未完成、JSON 无效和 token 无效的请求都在命令分发前拒绝，不进入审计，避免未认证本地进程无限占用任务或内存。
- `portmate-ipc.json` 通过同目录私有临时文件同步落盘后原子替换；Unix 最终权限强制为 `0600`，包括 keyring 不可用时含明文 token 的 fallback。替换不会跟随既有 symlink，失败会保留上一个完整 endpoint，并回收本次未发布的 keyring token。
- bridge 仅加载普通、≤64 KiB 且 Unix 无 group/world 权限的 endpoint 文件；`storePath` 必须匹配当前 Store，地址必须是 loopback `SocketAddr`，keyring 引用必须属于 `keychain:ipc-*` 专用命名空间且不能与 inline token 并存。bridge 到桌面的请求/响应分别限制为 1 MiB/64 MiB，并设置 3 秒连接、5 秒写入和 120 秒总响应 deadline，避免篡改 endpoint 触发任意远端连接、读取其他 keyring 记录或无界等待/分配。
- 长运行 stdio bridge 会在每个 JSON-RPC envelope 前重新加载最新有效 Store 快照和原子发布的 endpoint；桌面重启、IPC token/address 轮换无需重启 bridge，endpoint 删除会立即清空 live forwarding，Store 暂时不可读时则保留最后一次有效只读快照。endpoint 正常缺失不会反复输出错误日志。
- 桌面运行时通过本地 IPC 转发真实控制动作，IPC token 优先存 keyring。
- HTTP 模式通过 `--http` 或 `PORTMATE_MCP_HTTP=1` 启动，仅允许 loopback 绑定，校验 `Origin`，并要求 Bearer 或 `X-PortMate-MCP-Token`；HTTP token 优先来自 `PORTMATE_MCP_HTTP_TOKEN`，否则存入 OS keyring；支持 JSON-RPC POST、streamable-http JSON Accept 兼容、GET SSE 事件流和纯 SSE POST message 事件响应。POST 必须使用 `application/json`（允许 charset 参数），显式 `MCP-Protocol-Version` 必须匹配 `2025-06-18`，该版本头已加入 CORS preflight allow-list。
- HTTP bridge 最多同时保留 64 个连接（含长连接 SSE）；完整请求有不可被 trickle byte 延长的 5 秒总 deadline，每次普通/SSE 写入有 5 秒 socket timeout，超额连接立即返回 `503`，普通 HTTP/1.1 响应显式关闭连接，避免未认证本地进程无限占用线程。请求头通过 `httparse` 严格解析并限制为 64 KiB/128 项；重复的 framing/认证单值头、不支持的 `Transfer-Encoding`、畸形头和声明 body 后的额外字节均在 JSON-RPC 分发前拒绝。重复 `Accept` 会按列表合并，Bearer scheme 不区分大小写，`q=0` 媒体类型不会被误选。
- MCP Bridge 弹窗已提供 HTTP endpoint、Origin、启动命令、tokenRef 展示，以及 keyring token 生成/轮换入口。

主要缺口：

- HTTP MCP 已补 `Accept: application/json, text/event-stream` streamable-http JSON 兼容回归，GET `text/event-stream` 基础事件流、纯 SSE POST 的 `message` 事件响应，以及 Content-Type/协议版本/CORS preflight/严格 framing/重复头/quality value 回归；更完整客户端矩阵仍待补。
- MCP 已区分 `resources/list` 实际资源与 `resources/templates/list` URI 模板，支持 `ping`、有界 JSON-RPC batch/notification 语义；HTTP notification 返回无响应体的 `202 Accepted`。
- MCP 与桌面 IPC 都执行日志查询 `limit` 的 1..=1000 边界，日志搜索返回最近命中并按时间正序排列。
- 当 desktop IPC 不可用时，写工具已返回明确未执行错误；后续可考虑队列或离线计划。
- MCP 授权 UI 已有基础 grant 管理并展示逐 tool/client 的最近审计，记录本身包含 session/scope；还缺授权确认体验以及按 session/scope 的筛选、详情和导出。

### 测试与验证

本次审查已执行并通过的基础回归命令：

```bash
cargo fmt --all -- --check
cargo test --workspace -- --test-threads=4
cargo clippy --workspace --all-targets -- -D warnings
npm test -- --run
npm run build
```

`npm run build` 已把应用壳、Quick Command 管理器、xterm core、WebGL 和 CSS 拆为真实 lazy chunk：主 JS 约 493 kB、Quick Command 管理器约 4.4 kB、终端 core JS 约 439 kB、WebGL JS 约 120 kB、主 CSS 约 97 kB、终端 CSS 约 4 kB；此前约 805 kB 的单 chunk warning 已消失，未通过抬高阈值隐藏问题。浏览器回归已验证 Unicode 11、write-only OSC 52、WebGL/DOM fallback、进程内屏幕恢复、当前 pane 查找和自由输入，v4 view 复制/别名/着色/重载/关闭恢复、同组排序、跨组定点拖放、独立窗口颜色往返，以及手动/快捷键/启动/空闲锁屏、刷新恢复、detached 同步遮罩、焦点封闭和桌面/移动端几何。Quick Commands 另覆盖首次懒加载、增改排序、取消隔离、显式保存、Quick Bar 显隐/刷新恢复、插入/执行历史差异，以及 1440x900 与 390x844 无溢出布局；面板回归覆盖标题栏/菜单两种关闭入口、勾选态、单 pane 填充、全 dock/sender/status 聚焦布局、Quick Bar 组合、刷新恢复及移动端响应式隔离。专注模式另覆盖顶部按钮/精确 `Alt+Enter`、不改写持久化状态、退出恢复，以及同步输入保留 active 状态栏；工作台布局不塌陷。

已有单元测试覆盖：

- Host key alias 隔离、同 alias key mismatch、多算法 host key。
- Store open/close、profile upsert、MCP write scope、send_text redaction/audit。
- Trigger contains/regex、七类动作前端字段往返、多动作后端 dispatch、自定义链接替换和声音/通知/高亮 runtime effect。
- 同步输入设置归一化、目标协议过滤、换行/显式批量发送前后缀变换、Telnet CRLF、FIFO 批次顺序、交互输入不重复包裹、部分失败和关闭后即时取消剩余目标。
- 终端序列化缓存的 UTF-8 字节上限、LRU 淘汰、事件 ID 上限、空屏恢复和防御性复制，以及 OSC 52 只写剪贴板、拒绝远端读取、权限 Promise 拒绝/同步异常降级。
- 当前终端查找的标准/WindTerm 快捷键识别、选中文本单行化和 UTF-16 长度边界、结果/溢出/非法表达式状态，以及菜单到焦点 pane 的事件分发。
- 自由输入的 Unicode 字符上限、跨平台换行原子提交、空/空格草稿、可编辑选区剪切和菜单到焦点 pane 的事件分发。
- Quick Commands 的旧数组迁移、Unicode 名称/正文边界、NUL 清理、无效/重复 ID 修复、64 条上限、插入/执行 payload 和不可变上下排序。
- 工作区 pane/bar 显示状态的默认值、v1/旧直存快照恢复、逐字段损坏修复、幂等设置和不可变切换，以及不修改原状态的专注布局派生、同步输入状态栏保留和精确 `Alt+Enter` 识别。
- workspace v1/v2/v3→v4 迁移、重复 node/view ID 修复、同 session 多 view、独立别名/颜色、精确激活/复制/关闭/排序/定点移动/拆分/合并、失效 session 收敛和带 view 身份/颜色的 detach route 校验。
- 锁屏超时 `1..=1440` 归一化、绝对空闲 deadline、精确 WindTerm/Linux/macOS 快捷键，以及版本化跨窗口 marker 对损坏值的保守锁定与修复。
- Secret redaction。
- JSON 风格凭据、完整 Bearer token 脱敏，以及 redacted session bundle。
- 运行时断线诊断跨 store reload 保留。
- 多跳 one-time host key 生命周期与 `AskEveryTime` 强制确认。
- MCP resource/template、ping、batch、notification、HTTP `202`、日志 limit、stdio 超限恢复、慢请求总 deadline、连接限额拒绝/permit 释放，以及 desktop endpoint 的文件/地址/Store/tokenRef 校验和 IPC 请求响应上限。
- 隔离 OpenSSH 服务上的 TOFU、同地址 host key 变更阻断、`allowRotation` 后重新信任并保留轮换历史、公钥认证、PTY 命令、原生 SFTP 浏览/递归建目录/上传/rename/chmod/属性/远端复制/下载/递归删除、外部目录树递归上传（含空目录）、SFTP/SCP upload/download 与 SFTP remote-copy 的 `.portmate-part` 断点续传、限速 SFTP/SCP 上传取消后从 part 重试、SFTP/SCP 服务端拒写失败状态、传输中 SSH 断开后重连续传，以及 local/dynamic/remote reverse tunnel 的流量统计、目标拒绝、错误状态和原 tunnel 恢复。
- 三 OpenSSH 服务上的两跳 Jump Host direct-tcpip 链、三端独立公钥身份筛选、两跳/目标独立 TOFU 持久化、末端 PTY、第一跳连接拒绝、第二跳 direct-tcpip 拒绝、第一跳/第二跳/目标静默握手超时、第二跳错误 identity 与目标 identity 耗尽的逐端点诊断，以及第二跳 host key 变更诊断。
- 用户态 russh password/keyboard-interactive 跳板与两台独立 OpenSSH 公钥端点组成的两种混合认证链，以及第一跳错误密码诊断和三端 host key 持久化。
- HTTP CONNECT/SOCKS5 对 TCP/Telnet 的真实转发、拒绝响应、无认证限制、目标域名交给代理解析、关闭代理时忽略残留端点、空代理端点和换行注入拒绝；混合认证 Jump Host 矩阵还覆盖经代理执行目标 host-key 扫描及 password/keyboard-interactive 正式连接。
- SSH 重连延迟/KeepAlive 的旧 Profile 默认值、非法阈值夹紧、KeepAlive 开关到 russh `keepalive_interval` 的映射、健康参数变化触发最新 Profile 重连代次更新，以及真实 OpenSSH 断线等待期间从 5,000 ms 缩短到 100 ms 后恢复 session 和原 tunnel。
- 独立真实 `ssh-agent` 与 OpenSSH 服务上的 agent 禁用、未过滤 offer、`IdentitiesOnly` 空白名单、显式指纹白名单，以及错误指纹不能被相同 comment/path 绕过。
- OpenSSH PTY 上 lrzsz X/Y/ZModem 上传/下载、相邻协议 stale-byte 隔离、raw TTY 恢复、XModem block padding 精确截断，以及 TCP loopback 下数据块/EOT 首个 ACK 丢失后的精确重传。
- 传输限速等待不阻塞 async runtime，并能在 100 ms 轮询周期内响应取消。
- OpenSSH `MaxAuthTries 2` 下错误 key 优先导致认证耗尽、逐 identity 错误聚合，以及正确 key 前置后的成功连接。
- `socat` 虚拟 PTY 上的串口二进制收发、非 UTF-8 RX/TX 精确内存捕获、无探测字节的接收空闲超时、断线后切换到最新端口并动态缩短重连延迟、pending/connected 阶段关闭重连，以及设备不支持 DTR/RTS 时的兼容和拒绝边界。串口捕获单测覆盖 512 帧/1 MiB 环形边界、大帧截断标记、增量 reset、选中帧 JSONL 原子导出和 SHA-256 sidecar。
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
- Sysmon 旧摘要快照兼容、Linux/macOS/FreeBSD CPU/内存/负载解析、Windows PowerShell/CIM 编码命令与 marker JSON 解析、Top 进程排序与 8 条边界、磁盘解析/挂载点去重与 16 条边界、Linux `/proc/net/dev`、macOS/FreeBSD `netstat -ibn` 和 Windows 性能计数器的每接口速率/重复行去重及 32 条边界、完整远端输出、真实本机 Linux `/proc`/`ps`/`df` 采样、本机 macOS/Windows 异步采样调度，以及本机命令非零退出/超时/4 MiB stdout/64 KiB stderr 边界、SQLite v3→v4 details 迁移和默认 120、允许 `1..=240` 的会话历史查询、时间戳去重排序、刷新即时归并及 CPU/内存/RX/TX 趋势量程。
- Tmux、远端 tunnel 健康探测和 Sysmon 共用的 SSH exec 捕获分别限制 stdout 4 MiB、stderr 64 KiB；精确上限可接受，越界分片会在写入前整体拒绝并保持已有缓冲区不变。

当前 Rust workspace 自动化测试总数为 222：`portmate` 160、`portmate-kdf` 1、`portmate-core` 34、`portmate-mcp` 27；`npm test` 另有 23 个文件、139 个前端 transfer/selection/presentation/log-shard/workspace/workspace-hotkey/workspace-panel/screen-lock/detached-pane/trigger/sync-input/terminal-state/terminal-search/free-input/quick-command/OneKey-shortcut/clipboard/secret-migration/SSH-health/TCP-health/Serial-health/Serial-capture/proxy/Sysmon-history 单元测试。

主要缺口：

- 已有隔离 OpenSSH server、host key mismatch/`allowRotation`、MaxAuthTries/identity 顺序、真实 ssh-agent 策略/过滤和两跳 Jump Host 集成测试；第一/二跳连接拒绝、三段静默握手超时、逐跳独立 identity 拒绝、目标 identity 耗尽，以及 password/keyboard-interactive 到公钥端点的混合认证链均已覆盖。
- 已有 `socat` 虚拟串口 loopback 二进制收发、非 UTF-8 精确 RX/TX 捕获、无探测字节的接收空闲超时和 PTY 消失后的自动重连测试，覆盖切换到最新端口路径、等待期间从 2,500 ms 缩短到 200 ms、runtime ID 轮换、精确读错误诊断、重连期间拒绝写入、恢复后的双向 I/O，以及 connected 阶段关闭 reconnect 后直接断开；真实硬件和 modem 测试矩阵仍待补。
- Telnet/Raw TCP 已有 loopback mock 测试覆盖跨 read 分片的 IAC/TTYPE 子协商、Profile TTYPE、双向 BINARY 接受/拒绝/撤销、binary/NVT 数据差异、NAWS 协商前 resize、`0xff` 尺寸转义和连续 resize、NVT `CR NUL`/CRLF 与 EOF 孤立 CR、raw byte IAC 转义、Raw TCP 原样字节发送、内核 keepalive 自定义/关闭、旧 Profile 默认值与边界归一化，以及断线自动重连状态恢复、运行中缩短重连延迟并切换端口、pending/connected 阶段关闭重连的收敛；更广真实 Telnet 服务矩阵仍待补。
- 已有 OpenSSH SFTP 浏览/写操作/传输、SFTP/SCP 五条断点续传路径、SFTP/SCP 取消后 retry、服务端拒写失败状态、活动 SSH 断开后重连续传、lrzsz X/Y/ZModem 双向端到端、X/YModem 数据块与 EOT 的 ACK 丢失重传、静默 XModem 快速取消/CAN 和 transport 重连态旧 worker 快速失败测试；SFTP/SCP 更广服务故障矩阵，以及 modem 物理串口/OpenSSH 活动传输断线/工具变体矩阵仍待补。
- 已有 OpenSSH local/dynamic/remote reverse tunnel 端到端、三种模式目标拒绝后原 tunnel 恢复、remote 失败 channel 主动关闭、服务端撤销 remote forward 后被动探测/原端口重建、重复 cancel 被拒后的本地强制收敛、SSH channel 结束时按 session 清理旧 runtime、自动重连后按原 ID/标签/端口重建和单条端口冲突失败隔离，以及 SOCKS5 错误协议 loopback 测试；`sockstat`/`lsof`/BSD netstat 解析与失败工具回退已有单元矩阵，真实 FreeBSD/macOS SSH 主机仍待纳入集成环境。
- 已有基于浏览器 CDP 的工作区、独立窗口和截图回归，但尚未整理为仓库内正式 Playwright suite。
- Unicode 11、Serialize、write-only OSC 52、WebGL fallback 已有浏览器回归；仍没有 vttest、鼠标协议和全屏程序兼容基线。

## 对照最终目标的完成度

| 目标域 | 当前状态 | 说明 |
| --- | --- | --- |
| 跨平台桌面框架 | 已实现 | Tauri v2 + React/TS + Rust 已成型。 |
| xterm 6 | 已实现 | `@xterm/xterm` 固定 `6.0.0`；当前焦点 pane 增量查找、Unicode 11、write-only OSC 52、进程内有界 Serialize 恢复及 WebGL→DOM fallback 已接入。 |
| WindTerm 风格工作台 | 大部分实现 | 主布局和菜单、最多 16 pane/8 层的递归水平/垂直分屏、每组最多 32 个独立 ID view、v1/v2/v3→v4 迁移、同 session view 复制/独立重命名/逐 view 着色、同组排序/跨组定点拖放/整组合并/四方向新分组/关闭与恢复、可调且持久化的比例、pane/active/tab color 恢复、可配置且支持最多两段 chord/冲突校验的 WindTerm 分屏/方向焦点/关闭/zoom 快捷键、方向 pane 交换、保留 view 身份/颜色的 Tauri 独立窗口/返回、主密码/隐私降级锁屏、启动/空闲锁屏、启动会话策略及 xterm/CSS lazy chunk 已有。 |
| 同步输入 | 已实现 | 多 pane 去重广播、额外目标协议过滤、协议感知换行、目标间延迟、显式批量发送前后缀、FIFO、失败/即时取消反馈、明显目标计数和启动默认关闭均已接入，并有前端状态回归。 |
| SSH | 部分实现 | PTY、密码、公钥、keyboard-interactive、ssh-agent、Profile 级协议 KeepAlive 阈值、带可选认证的 HTTP CONNECT/SOCKS5、多跳 Jump Host 后端连接链路、每跳独立 secretRef/identityRef 和基础编辑可用；代理与 host-key 扫描路径一致且只作用于第一物理跳。两跳 OpenSSH direct-tcpip、三端独立 identity、逐跳 TOFU、第一/二跳连接拒绝、第一/二跳及目标握手超时、逐端认证失败聚合、第二跳 key mismatch、password/keyboard-interactive 混合链，以及真实 ssh-agent 启用/禁用/过滤矩阵已端到端覆盖；健康故障矩阵和 GSSAPI 未完成。 |
| Host key 隔离 | 大部分实现 | profile alias、TOFU、mismatch block、known_hosts 导入导出、连接失败确认弹窗、一次性信任、多跳 Jump Host 目标扫描、多跳连接时逐跳验证、逐跳确认 UX、每跳自定义 host-key 策略已有；高级管理待补。 |
| Bitvise 风格密钥管理 | 大部分实现 | keyring/secretRef、Host Key Manager scope/profile 分组过滤和批量删除/复制、host key 字段编辑、Client Key profile/source 搜索分组、跨 profile 批量复制/置顶/安全移除、私钥文件/粘贴导入、Agent identity 单条/批量添加、identity 字段编辑、Vault 私钥轮换、共享 secret 生命周期保护、Argon2id + IOTA Stronghold portable vault/fallback/主密码轮换，以及带预检 token、durable SQLite journal、原子 commit point、跨重启显式恢复、冲突冻结和安全诊断导出的 SSH/Tmux/TCP/Telnet profile 凭据双向迁移已有；跨平台 provider 故障矩阵待补。 |
| Shell/SSH/Telnet/TCP/Serial | 部分实现 | 基础连接读写、SSH/Tmux/TCP/Telnet 的 Profile 级 HTTP CONNECT/SOCKS5 与可选认证、SSH/Tmux 的重连延迟与协议 KeepAlive 阈值、Telnet 增量协商/NVT CR 编解码/Profile TTYPE/方向性 BINARY/NAWS/raw byte IAC 转义、Telnet/Raw TCP loopback、TCP/Telnet 的 Profile 级重连延迟与 OS keepalive、Serial 的重连延迟/无探测接收空闲阈值/精确有界 RX-TX 捕获/方向与内容过滤/原子 JSONL 导出、SSH/TCP/Telnet/Serial 重连加载最新 Profile 并拒绝过期尝试、TCP/Telnet/Serial pending/connected 阶段禁用收敛、虚拟串口切换最新端口自动重连、runtime 最近断开原因可见、break、DTR/RTS 和 hex 字节发送可用；更深诊断和独立串口分析窗口待补。 |
| Tmux | 部分实现 | list/attach 可用；pane sync 和更完整 tmux workflow 待补。 |
| SFTP/SCP | 部分实现 | 原生 SFTP 和 SCP、双栏、多选/连选/全选、批量删除、rename、chmod、属性查看、面板间及原生外部文件/目录树拖放、远端目录递归下载、空目录保留、安全批次规划、四种冲突策略、retry、速度、local/SFTP/SCP 分块进度与取消、profile 级异步可取消限速、local/SFTP/SCP upload/download 断点续传、远端命令复制大小标记/目标大小轮询进度/取消和 `.portmate-part` 续传、后台串行队列调度、全量队列视图、批量取消/重试和失败诊断展示已有；真实 OpenSSH 递归上传/下载和冲突重命名已覆盖，更广服务故障矩阵待补。 |
| X/Y/ZModem | 部分实现 | 三者都有实现，块级进度与取消已接入；OpenSSH PTY + lrzsz 六方向传输、raw TTY、READY/DONE 门控、XModem 精确长度、静默对端取消后 CAN/worker 清理和 transport 重连态断线失败已覆盖，物理串口、OpenSSH 活动传输断线和工具变体矩阵待补。 |
| 隧道 | 大部分实现 | local/remote/dynamic、运行中列表、停止入口、连接数/字节/最后错误、监听器终止、Linux/FreeBSD/macOS remote forward 被动探测、撤销后重建、cancel 失败本地收敛、SSH 断线清理和重连后原规格恢复已接入；OpenSSH 三模式、撤销/恢复/停止、重建失败隔离和 SOCKS5 错误协议已覆盖，真实 BSD/macOS 主机和更广服务端矩阵待补。 |
| Sysmon | 大部分实现 | 本机 Linux/macOS/Windows 与 SSH/Tmux Linux/macOS/FreeBSD/Windows 的 CPU、内存、uptime、聚合吞吐、Top 进程、磁盘和每接口速率/累计量已进入有界快照、SQLite details、MCP resource 和可刷新四标签工作窗口，Unix 额外提供 load average；本机 macOS/Windows 使用带超时和输出边界的异步平台命令，Windows 远端在 `uname` 失败后使用固定编码 PowerShell/CIM 脚本和二次校验的 marker JSON，不读取进程命令行；历史趋势支持 CPU/内存利用率与 RX/TX 速率、有界查询、去重排序及刷新即时归并；当前会话工具栏 applet 支持立即/10 秒采样、请求去重、断线停止和失败保留旧值；真实 macOS/Windows 桌面构建、macOS/FreeBSD/Windows SSH 主机矩阵、其他 BSD 与独立常驻侧栏待补。 |
| 日志 | 大部分实现 | 结构化 events/SQLite、双向精确 transport raw、Telnet reply/modem control、system Text/JSONL sink、每会话出站 lane、共享路径串行追加、SHA-256 v2 `bytesRef`、预览/筛选/搜索/清理/保留/归档和可选 raw 的脱敏 session bundle 已有；命令关联与毫秒级分片待补。 |
| 触发器 | 已实现 | 多条 contains/regex 规则、多动作编辑、高亮、通知、时间线、本地命令、发送文本、自定义链接和声音均有模型、运行时 dispatch 与回归覆盖。 |
| MCP stdio | 已实现 | bridge、tools/resources/prompts、grant scope、1 MiB 可恢复输入边界、128 项 batch 上限、64 MiB 响应序列化边界、严格 ID/params envelope、逐 envelope Store/endpoint 刷新、live IPC、endpoint 信任边界和有界 IPC I/O 已有。 |
| MCP HTTP | 部分实现 | `portmate-mcp --http` 支持 loopback JSON-RPC、Origin 校验、Bearer/X-Token、本地 keyring token、streamable-http JSON Accept 兼容回归、GET SSE、纯 SSE POST、JSON Content-Type/协议版本/CORS preflight 校验、严格 HTTP framing、64 KiB/128 项请求头边界、64 MiB JSON-RPC/SSE 数据边界、总读取/单次写入超时和 64 连接上限；桌面 UI 可展示配置并轮换 token；客户端矩阵待补。 |
| 测试体系 | 部分实现 | core 单测可用；集成、UI、终端兼容测试不足。 |

## 下一阶段目标

### P0：把 alpha 变成稳定可日用版本

1. Client identity 字段编辑、密钥轮换、引用计数生命周期管理、OS keyring 不可用时的 IOTA Stronghold portable vault/fallback、主密码轮换，以及带 durable journal/跨重启核对/安全 conflict 诊断导出的 SSH/Tmux profile 凭据双向批量迁移已完成；继续补 Windows/macOS/Linux 原生 keyring/Stronghold 故障注入矩阵。
2. Jump Host password/keyboard-interactive 混合认证、连接拒绝、三段握手超时与逐端 identity 失败诊断已覆盖。
3. remote forward 服务端撤销的被动探测/原端口重建、cancel 失败后的本地收敛，以及远端命令型传输失败详情、部分进度、事件摘要和复制诊断均已完成；继续扩展服务端故障矩阵。
4. 扩展端到端集成测试：SFTP/SCP 更广服务故障矩阵、Raw TCP/Telnet 多服务端兼容矩阵和 modem 的物理串口/OpenSSH 活动传输断线；虚拟串口重连、Telnet BINARY/NAWS loopback、静默 modem 快速取消和 transport 重连态失败已覆盖。
5. 扩展自动重连、断线恢复和连接健康检测：SSH、TCP/Telnet 与 Serial 会在重试前加载最新 Profile，并在尝试完成时拒绝已过期配置；三者的重连延迟都可在等待期间按最新 Profile 动态调整，TCP/Telnet/Serial 在 pending/connected 阶段关闭重连均能立即或下次断线时收敛；SSH 协议 KeepAlive、TCP/Telnet OS keepalive，以及 Serial 无探测接收空闲阈值已可按 Profile 配置，runtime 最近断开时间/原因已可见；下一步补 SSH/Serial 更广故障矩阵和更深健康诊断。

### P1：补齐 WindTerm/Bitvise 级工作流

1. 文件管理器多选、可配置冲突策略和远端目录递归下载已完成；继续扩展 SFTP/SCP 服务故障矩阵和跨平台路径边界。
2. pane view group、启动自动连接、session/逐 view 标签颜色、workspace restore、v1/v2/v3→v4 自动迁移、同 session view duplicate/独立 rename、view 同组排序/跨组定点拖放/整组合并/四方向新分组/关闭与恢复、任意递归嵌套分屏、保留 view 身份/颜色的独立 Tauri 窗口/返回、跨主/独立窗口锁屏、可持久化的 dock/sender/status 显示开关、可逆专注模式，以及支持最多两段 chord 的可配置 WindTerm 分屏创建/方向焦点/关闭/zoom 快捷键和方向 pane 交换已完成。
3. 同步输入正式化已完成：多 pane 去重广播、协议过滤、换行策略、延迟、显式批量发送前后缀、FIFO、失败/即时取消反馈和明显目标计数均已接入；为避免误广播，开关不跨启动保留。
4. FreeType 风格自由输入、Quick Commands 和独立 OneKeys 凭据管理已完成：焦点 pane 本地有界多行编辑、剪切/取消/原子提交、终端换行、查找互斥、同步输入复用、有界命令管理/排序/持久化、Quick Bar 插入或执行，以及加密 OneKey Secret、会话绑定和手动敏感字段发送均已接入；下一步补 OneKeys 自动提示补全、SSH 登录弹窗、公钥和 keyboard-interactive 集成。
5. 串口工具增强：精确有界 Hex/ASCII viewer、收发过滤与 JSONL + SHA-256 导出已完成；下一步补独立分析窗口、协议帧解析、书签和重连状态可视化。
6. 密钥管理器继续增强：portable vault 创建/解锁/锁定、主密码轮换、Client identity 字段编辑、密钥轮换、底层 secret 生命周期管理、SSH/Tmux profile 凭据双向批量迁移、migration journal、恢复/重载 UX 和人工 conflict 诊断导出已完成；下一步补跨平台 provider 回归。

### P2：日志、诊断和 MCP 产品化

1. append-only raw/text/jsonl 分片的安全枚举、受限预览、筛选、Text/JSONL 历史全文查询、批量清理 UI、通用归档、profile 自动保留期、双向精确 transport 字节/v2 引用、出站顺序和 system Text/JSONL sink 已完成；继续补命令关联与毫秒级分片。
2. `export_session_bundle` 的桌面 `.tar.gz` 交付包、逐文件/整包校验、平台/store 诊断、默认脱敏和显式 raw 策略已完成；继续补签名和自定义附件选择。
3. MCP HTTP 模式：补 streamable-http 客户端矩阵和更多客户端回归测试。
4. Sysmon 的进程、磁盘、网络接口、本机 Linux/macOS/Windows、Linux/macOS/FreeBSD/Windows 远端采样、四标签工作窗口、CPU/内存/RX/TX 历史趋势、10 秒工具栏 applet 和结构化持久化已完成；继续补真实 macOS/Windows 桌面构建、macOS/FreeBSD/Windows SSH 主机矩阵、其他 BSD 与独立常驻侧栏。
5. 把现有 CDP 截图检查整理为 Playwright UI 回归，并补 vttest、鼠标协议和全屏程序兼容基线；Unicode 11 插件与浏览器验证已完成。

### P3：架构整理与发布准备

1. 拆分当前 `src-tauri/src/lib.rs`：transport、transfer、mcp、storage、security、terminal 模块化。
2. SQLite 大型追加表已改为增量写入并有 INSERT/DELETE 触发器回归；继续拆分存储模块并评估 kv/JSON 兼容快照的异步化。
3. Stronghold portable vault 已覆盖 OS keyring 不可用/禁用场景、主密码轮换、SSH/Tmux 凭据批量迁移和保守的跨重启恢复；继续把 journal/recovery 与 provider 适配从 `src-tauri/src/lib.rs` 拆为独立 security/storage 模块。
4. 增加 Windows/macOS/Linux 打包验证和权限说明。
5. 建立 release checklist：签名、更新日志、迁移测试、回滚策略。

## 建议的近期执行顺序

1. 集成测试环境加入真实 FreeBSD/macOS SSH tunnel 主机；跨平台探测命令与解析单元矩阵已完成。
2. keyring/Stronghold 的 Windows/macOS/Linux 故障注入矩阵；durable migration journal、异常提交核对、重载 UX、双向迁移、跨进程 CAS 和 conflict 诊断导出已完成。
3. 更深连接健康探测和跨平台传输故障矩阵。
4. 把现有 CDP 检查整理为 Playwright UI 回归，并补 vttest、鼠标协议和全屏程序兼容基线。

这个顺序优先补“真实终端工具的可靠性”和“会话控制的安全边界”，比继续堆 UI 设置项更能降低后续返工。
