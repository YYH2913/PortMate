<p align="center">
  <img src="./src-tauri/icons/128x128.png" width="96" height="96" alt="PortMate Logo">
</p>

<h1 align="center">PortMate</h1>

<p align="center">面向 SSH、串口与远程运维场景的跨平台终端工作台，并提供受控的 MCP 会话桥接能力。</p>

<p align="center">
  <a href="./README.md">English</a> |
  <strong>简体中文</strong>
</p>

<p align="center">
  <a href="https://github.com/YYH2913/PortMate/actions/workflows/native-ci.yml">Native CI</a> ·
  <a href="https://github.com/YYH2913/PortMate/actions/workflows/mcp-sdk-freshness.yml">MCP SDK Freshness</a> ·
  <a href="./LICENSE">Apache-2.0</a> ·
  <a href="./.nvmrc">Node.js 22.20.0</a>
</p>

> [!IMPORTANT]
> PortMate 当前处于 **alpha** 阶段。仓库内可在 Linux、Docker 和浏览器环境复现的实现与兼容矩阵已经建立，但 Windows/macOS 原生安装包、真实 Microsoft AD、物理串口设备和发布签名仍需要外部环境完成最终验证。请勿将当前构建直接用于无人值守的生产关键链路。

## PortMate 是什么

PortMate 是一个以终端为核心的 Tauri v2 桌面应用。它把 SSH、Shell PTY、串口、Telnet、Raw TCP、Tmux、文件传输、隧道、日志和系统监控放在同一个可分屏工作区中。

项目的一个重点是会话级身份与信任隔离：SSH Host Key、客户端身份、认证顺序和 Jump Host 策略都属于具体 Profile，不依赖系统全局 `~/.ssh/known_hosts`。同一 IP 和端口对应不同设备、重装系统或实验室板卡时，可以分别保存和审查信任关系。

PortMate 本身不内置 AI 助手。随包提供的 `portmate-mcp` bridge 允许外部 MCP Host 在用户授权范围内读取会话状态、查询日志或执行控制动作。

## 主要功能

| 领域 | 能力 |
| --- | --- |
| 会话与协议 | SSH、Shell PTY、Serial、Telnet、Raw TCP、Tmux；TCP/Telnet 支持 TLS，SSH/Tmux/TCP/Telnet 支持 HTTP CONNECT 与 SOCKS5 代理 |
| 终端工作区 | 多标签、递归分屏、跨分组拖放、独立窗口、布局恢复、Insert/Normal 模式、对应光标、搜索、行跳转、文本/Hex/二分视图 |
| 交互效率 | WindTerm 风格命令补全、参数提示、自动多色交互命令行、Quick Commands、OneKeys、自由输入、同步输入 |
| SSH 安全 | Profile 级 Host Key、TOFU 与变更阻断、Host/Client Key Manager、多级 Jump Host、ssh-agent、密码/公钥/keyboard-interactive，以及 Linux libssh GSSAPI 认证 |
| 文件与传输 | SFTP/SCP 文件管理、拖放、队列、限速、取消、重试、断点恢复，以及 X/Y/ZModem |
| 运维与诊断 | SSH 健康检测、local/remote/dynamic tunnel、Sysmon 侧栏与历史趋势、结构化日志、触发器、会话诊断包 |
| 串口工具 | 常用波特率、DTR/RTS/Break、文本与 Hex 收发、精确字节捕获、独立分析器、SLIP/COBS/Modbus RTU 解码 |
| MCP | stdio 与 Streamable HTTP、细粒度授权、会话范围、写操作确认、审计、Token 轮换和托管 sidecar 生命周期 |

## 当前状态

- 桌面目标平台：Linux、Windows、macOS。
- 当前主要实机开发与验收环境：Linux + VMware。
- 终端基于 `@xterm/xterm` 6，包含 Search、Serialize、Unicode 11、Clipboard、Web Links、Fit 和按需 WebGL 支持。
- SSH/SFTP/SCP、Telnet/TCP、vttest、全屏程序和多个 Tmux 版本均有自动化兼容矩阵。
- MCP 回归覆盖 TypeScript、Python、Go、Rust、Ruby、Java、Kotlin、C# 和 Swift 官方 SDK。
- Linux DEB、RPM 和 AppImage 已有本地打包与包内生命周期门禁；Windows 和 macOS 仍需原生 runner 的成功证据。

详细实现进度和未完成边界见 [PROGRESS.md](./PROGRESS.md)，正式发布要求见 [RELEASE.md](./RELEASE.md)。

## 快速开始

### 前置环境

- Git
- Node.js `>= 22.12.0`，仓库 `.nvmrc` 固定已验证版本 `22.20.0`
- Rust stable toolchain
- 当前平台所需的 [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/)

Linux 构建桌面应用时通常还需要 WebKitGTK、GTK3、libssh、Kerberos、udev 和系统托盘相关开发包。GitHub Actions 中的完整 Ubuntu 依赖列表可参考 [.github/workflows/native-ci.yml](./.github/workflows/native-ci.yml)。

### 启动桌面应用

```bash
git clone https://github.com/YYH2913/PortMate.git
cd PortMate

nvm use
npm ci
npm run desktop:clean
```

`desktop:clean` 只清理当前仓库遗留的 Vite 监听和已知的 Snap GTK/WebKit 环境污染，不会删除 Profile、日志或 PortMate 数据。正常情况下也可以直接运行：

```bash
npm run desktop
```

启动顺序是：构建开发版 MCP sidecar、启动 Vite、启动 Tauri/Rust 后端。首次 Rust 编译可能需要一些时间。

### 浏览器预览

```bash
npm run dev
```

访问 <http://127.0.0.1:1420/>。浏览器模式适合检查布局和前端交互；真实 SSH、串口、文件传输、密钥库和本地 IPC 需要 Tauri 桌面后端。

## 基本使用

1. 打开 `会话 -> 新建会话`。
2. 选择 Shell、SSH、Tmux、Telnet、TCP 或 Serial。
3. 分别填写主机/IP、端口和用户名；无需使用 `username@host` 组合格式。
4. SSH 首次连接时核对服务端 Host Key 指纹，并选择一次性信任或保存到 Profile/项目信任域。
5. 在 `工具` 菜单中打开传输、隧道、Tmux、Sysmon、串口分析器、密钥管理器、日志或 MCP Bridge。

连接状态会显示在会话标签前：绿色表示已连接，其余不可用状态显示为红色，并通过 tooltip 保留具体诊断。启动恢复会静默尝试已保存且可用的凭据，不会在每次打开应用时自动弹出重连窗口。

SSH、Tmux、TCP、Telnet 和 Serial 的自动重连会在下一次尝试前重新读取最新 Profile。连接参数、认证策略、重连延迟或开关的修改不需要等待旧配置完成全部重试。

## SSH 信任与凭据

PortMate 将两类密钥分开管理：

- **Host Key**：证明远端服务器身份。默认保存在 PortMate 自有信任库，不自动写入系统 `known_hosts`。
- **Client Identity**：用于登录远端的私钥或 agent 身份。可以限制尝试顺序，避免无关密钥耗尽服务端 `MaxAuthTries`。

SSH 密码、私钥口令、代理密码、OneKey Secret 和 Profile Vault 私钥统一保存在由用户主密码保护的 IOTA Stronghold 中；SQLite 只保存 `stronghold:` 引用，保存或使用前必须先解锁 Vault。系统原生 keyring 仅保留持久 MCP HTTP Token、bundle 签名身份等程序内部材料。已有用户 `keychain:` 引用仍可读取、删除并单向迁移到 Stronghold，但 PortMate 不再向系统 keyring 新建或覆盖用户凭据。短期桌面 IPC Token 会在每次启动时轮换，只写入原子更新且仅属主可读的 `portmate-ipc.json`，不会进入 SQLite。

SSH 建连期间，可信凭据弹窗提交后的明文只留在桌面后端；前端后续只持有绑定调用窗口、会话和当前 SSH 配置的短期一次性句柄。MCP 请求不能提交密码、私钥口令或凭据句柄。完整信任边界与限制见 [SECURITY.md](./SECURITY.md)。

Host Key 变化默认阻断连接。请在确认设备替换、系统重装或密钥轮换后，再使用一次性信任、追加或替换操作。不要为了消除提示而关闭验证。

## MCP Bridge

### 推荐配置方式

1. 保持 PortMate 桌面应用运行。
2. 打开 `工具 -> MCP Bridge -> 授权`，创建独立 Client ID，并选择权限和允许访问的会话。
3. 对写权限启用“每次确认”，由桌面端逐次批准或拒绝。
4. stdio 客户端使用界面显示的 bridge 与 Store 精确路径。
5. HTTP 客户端在 `HTTP` 页设置监听 IP、端口、Origin、Client ID，生成 Token 后启动托管服务。

### stdio 示例

先构建开发 sidecar：

```bash
cargo build --locked -p portmate-mcp
```

多数 MCP Host 使用类似以下配置。请将路径替换为 MCP Bridge 页面显示的真实值：

```json
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
```

bridge 会在每个 JSON-RPC envelope 前重新读取 Store 和桌面 IPC endpoint，因此桌面重启或 IPC Token 轮换通常不要求重启长驻 stdio 客户端。

### HTTP 模式

- 默认监听 `127.0.0.1:8787`，端点为 `/mcp`。
- 支持 `127.0.0.1`、`0.0.0.0`、`::1`、`::` 和自定义数字 IP。
- 非回环监听必须显式启用远程访问。
- 每个请求都需要 Bearer Token 或 `X-PortMate-MCP-Token`，存在 Origin 时还会校验 allowlist。
- bridge 本身不终止 TLS。远程监听应只用于可信网络，或放在配置正确的 TLS 反向代理后方。

不要把 HTTP Token 写入 README、启动命令、issue、日志或 MCP 客户端的公开配置示例中。

### 权限范围

| Scope | 含义 |
| --- | --- |
| `read-sessions` | 读取已授权会话及运行状态 |
| `read-logs` | 读取和搜索已授权会话日志 |
| `write-input` | 向终端发送文本、按键或命令 |
| `transfer` | 创建文件传输任务 |
| `tunnel` | 创建 SSH tunnel |
| `manage-sessions` | 打开或关闭会话 |

授权可以设置到期时间、撤销状态、允许会话列表和写操作逐次确认。所有 MCP 写操作都会进入审计记录。

## 构建

前端生产构建：

```bash
npm run build
```

桌面安装包构建：

```bash
npm run desktop:build
```

产物位于 `target/release/bundle/`。正式发布前必须在目标平台执行 [RELEASE.md](./RELEASE.md) 中的安装、升级、回滚、签名和产物校验，不应把一次本地源码构建直接视为可发布安装包。

## 验证

常规开发门禁：

```bash
npm test
npm run build
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

重点兼容门禁：

```bash
npm run test:terminal-compat
npm run test:vttest-compat
npm run test:tmux-workflow
npm run test:tmux-version-compat
npm run test:workspace-ui
npm run test:ssh-server-compat
npm run test:ssh-gssapi-compat
npm run test:tcp-telnet-server-compat
```

MCP 客户端矩阵：

```bash
npm run test:mcp-stdio-client
npm run test:mcp-http-client
npm run test:mcp-typescript-client
npm run test:mcp-python-client
npm run test:mcp-go-client
npm run test:mcp-rust-client
npm run test:mcp-ruby-client
npm run test:mcp-java-client
npm run test:mcp-kotlin-client
npm run test:mcp-csharp-client
npm run test:mcp-swift-client
npm run test:mcp-sdk-freshness
```

Docker 兼容矩阵、Chrome/Playwright、系统 keyring 和桌面打包测试需要额外工具或对应操作系统。完整发布命令以 [RELEASE.md](./RELEASE.md) 为准。

## 项目结构

```text
PortMate/
├── src/                    React/TypeScript 桌面工作区
├── src-tauri/              Tauri 应用、命令适配与平台集成
│   └── src/
│       ├── backend_application.rs
│       ├── backend_automation.rs
│       ├── backend_security.rs
│       ├── backend_storage.rs
│       └── backend_transport.rs
├── crates/
│   ├── portmate-core/      共享模型、Store、Host Key 与授权策略
│   ├── portmate-mcp/       MCP stdio/HTTP bridge
│   ├── portmate-kdf/       Portable Vault KDF 边界
│   ├── portmate-keyring/   跨平台原生 keyring 边界
│   └── russh-sftp/         项目使用的 SFTP 兼容实现
├── scripts/                构建、打包与兼容矩阵脚本
├── tests/                  外部服务端与协议夹具
└── .github/workflows/      Native CI 与 SDK freshness 工作流
```

Tauri 根 `lib.rs` 只保留模块注册与公开重导出。transport、security、storage、automation 和 application 的实现按各自 owner 维护，避免新的跨领域逻辑重新堆回根模块。

## 数据与隐私

- Profile、运行状态、授权和索引数据保存在应用数据目录中的 `portmate-store.sqlite3`。
- JSON compatibility snapshot、日志、导出和 IPC endpoint 均使用有界、原子或私有权限写入策略。
- 原始终端日志可能包含敏感业务数据；只在确有需要时启用 Raw 日志，并在分享诊断包前检查脱敏范围。
- MCP 读取结果会移除凭据引用和本地敏感路径；写操作仍需权限，并记录来源 Client ID、动作、会话和最终结果。
- 漏洞请通过 GitHub Security Advisory 私下报告，不要在公开 issue 中提交凭据、私钥、生产主机名或未脱敏 Store。

更多说明见 [SECURITY.md](./SECURITY.md)。

## 已知限制

- 当前不是完整的 WindTerm 或 Bitvise 替代品，也没有正式稳定版本承诺。
- 真实 Microsoft Active Directory GSSAPI/PAC 尚待实证；当前 Samba 结果只代表 AD-compatible 协议覆盖。
- Windows OpenSSH 远端 Sysmon、真实 macOS/FreeBSD SSH/SFTP/SCP/remote-forward 仍需外部主机证据。
- 物理串口/USB 串口和 Modem 的断电、拔插及线路状态矩阵不能由虚拟 PTY 完全替代。
- Windows Authenticode、Apple Developer ID 与 notarization 凭据尚未用于最终发布产物。
- MCP HTTP 不提供内置 TLS。

这些限制不会用模拟结果替代。当前边界和所需资源持续记录在 [PROGRESS.md](./PROGRESS.md#剩余外部验证门槛)。

## 相关文档

- [PLAN.md](./PLAN.md)：产品目标与关键设计
- [PROGRESS.md](./PROGRESS.md)：实际实现、兼容矩阵与剩余门槛
- [RELEASE.md](./RELEASE.md)：发布检查清单
- [SECURITY.md](./SECURITY.md)：安全策略、依赖例外与漏洞报告

## 参与贡献

提交改动前请：

1. 先确认行为属于现有模块边界，避免无关重构。
2. 为用户可见行为或协议边界添加对应测试。
3. 运行与改动范围相称的前端、Rust 和兼容门禁。
4. 不提交真实凭据、私钥、Token、生产 Store、未脱敏日志或签名材料。
5. 对较大的协议、存储格式或安全策略变更先创建 issue 说明兼容性和迁移方案。

## 致谢

PortMate 的工作区和交互设计参考了 WindTerm，SSH 信任与密钥管理思路参考了 Bitvise 和 OpenSSH。终端使用 xterm.js，桌面框架使用 Tauri，默认等宽字体为 JetBrains Mono。

## 许可证

PortMate 使用 [Apache License 2.0](./LICENSE)。随应用分发的 JetBrains Mono 使用 SIL Open Font License 1.1，许可证位于 [THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt](./THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt)。
