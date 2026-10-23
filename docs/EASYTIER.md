# EasyTier integration / EasyTier 集成

## English

PortMate includes the unmodified [EasyTier 2.6.4](https://github.com/EasyTier/EasyTier/releases/tag/v2.6.4)
core and CLI inside the desktop executable. Windows/Linux x64 and macOS ARM64
installers, portable archives and signed updater packages all carry this payload.
There is no separate component download, external program directory or system service.
Networking starts only when requested by the user.

### Configuration and operation

Open **Tools → EasyTier networking**. The layout follows the official
[configuration](https://github.com/EasyTier/EasyTier/blob/v2.6.4/assets/config-page.png)
and [running-status](https://github.com/EasyTier/EasyTier/blob/v2.6.4/assets/running-page.png)
screens. Enter matching network names and secrets on each device, select DHCP or
a static virtual IPv4, and choose a networking method:

- **Public server** selects the official TCP or UDP `public.easytier.top:11010` endpoint.
- **Manual connection** accepts up to 32 peer addresses, with add/remove controls.
- **Standalone** starts without outbound peers; other devices join via this device's listener.

Public peers are added only when selected. Upstream NAT discovery may still contact
STUN servers. **Advanced settings** contains the hostname, address mode, listeners,
MTU and latency preference. Clearing all listeners disables inbound connections.
UPnP port mapping and relaying for unrelated networks are disabled.

DHCP and static mode create a virtual adapter and require administrator rights on
Windows, root on macOS, or the necessary Linux network privileges (including
`CAP_NET_ADMIN` and access to `/dev/net/tun`). PortMate does not elevate automatically.
**Relay only** avoids creating an adapter, so local SSH and other applications cannot
use a virtual IP from that mode.

**Connect network** prepares the built-in core locally, starts it as an owned child
process and opens **Running status**. This page shows the local node, connected
peers, routes, latency, cumulative traffic and redacted logs. The local node is
excluded from the peer count. A single configuration is saved; **New configuration**
resets the draft, which replaces saved settings only on Save or Connect.

Closing the dialog keeps the network running. **Disconnect network**, normal PortMate
exit and verified updater preparation stop the owned process tree. No separate
service or automatic restart outside PortMate is installed.

### Built-in runtime and source builds

The first connection writes the embedded, target-specific core, CLI and required
Windows DLLs/driver into a private `easytier/builtin-2.6.4-<payload-id>/` directory
under PortMate's application data. Every subsequent start verifies file sizes,
SHA-256 hashes and executable permissions; missing or altered files are restored
from the built-in payload. An intact runtime is reused. This preparation needs no
network connection. Early development settings containing an external binary
directory migrate to the built-in core.

The Tauri development and production build hooks run `npm run easytier:prepare`.
This build-time command caches the official archive and checks the pinned SHA-256
in `src-tauri/easytier-manifest.json`; the Rust build checks the archive again and
embeds only required files. Unsupported targets or missing/mismatched archives fail
the desktop build. No incomplete desktop package is produced silently.

For direct desktop Cargo builds, run `npm run easytier:prepare` first. Add
`-- --target x86_64-pc-windows-gnu` for a Windows cross build, for example.
An offline builder can import an already downloaded, matching archive using
`-- --archive /absolute/path/to/archive.zip`; subsequent builds reuse the verified cache.
Archives and generated executables are ignored by Git. Headless library builds
using `--no-default-features` do not require this payload; tests can explicitly
enable the `bundled-easytier` feature without the desktop window runtime.

### Secrets, control and licensing

Saved settings contain no network secret. Enter the secret on each start; the form
clears it after successful launch. The core reads a temporary configuration in a
private application-data directory (0700 directory and 0600 file on Unix), which
normal stop/exit removes. Abrupt power loss or forced process termination can leave
this file behind. Status responses omit the raw core configuration; logs redact
credentials. The management RPC uses a random loopback port, accepts only loopback
addresses and verifies the expected instance identity on queries.

The built-in EasyTier executables remain separate processes, unmodified and
independent of PortMate's Rust libraries. EasyTier uses
[LGPL-3.0](../licenses/EasyTier-LGPL-3.0.txt), incorporating
[GPL-3.0](../licenses/GPL-3.0.txt). Both texts and the matching
[upstream source reference](https://github.com/EasyTier/EasyTier/tree/v2.6.4)
are embedded and written alongside the runtime. PortMate retains its Apache-2.0 license.

## 中文

PortMate 将未修改的 [EasyTier 2.6.4](https://github.com/EasyTier/EasyTier/releases/tag/v2.6.4)
核心、CLI 和必要运行库内置到桌面可执行程序。Windows/Linux x64 与 macOS ARM64 的安装包、
便携版和自动更新包均包含，不需要另行下载组件、选择程序目录或安装系统服务。组网由用户主动启动。

### 配置与使用

打开 **工具 → EasyTier 组网**，填写各设备相同的网络名称与密钥，设置 DHCP 或静态虚拟 IPv4，
选择公共服务器、手动连接或独立组网。公共节点仅在明确选择后添加，EasyTier 的 NAT 探测仍可能访问 STUN。
高级设置提供设备名称、地址模式、监听地址、MTU 和低延迟优先；清空监听地址会关闭入站监听，
UPnP 端口映射和其他网络的中继转发默认关闭。

点击 **连接组网** 会从本地准备内置核心、启动托管子进程并进入 **运行状态**，显示本机信息、
对等节点、路由、延迟、累计收发量与脱敏日志。当前保存一份配置，“新建配置”重置草稿，保存或连接后才替换。
关闭窗口后网络继续运行；断开、正常退出 PortMate 或准备安装已验证的更新时停止托管进程树。

创建虚拟网卡需要 Windows 管理员权限、macOS root 权限，或 Linux 相应的网络权限
（包括 `CAP_NET_ADMIN` 与 `/dev/net/tun` 访问权限），应用不自动提权。仅中继模式无需网卡，
本机 SSH 等应用也无法通过该模式的虚拟 IP 访问组网。

### 内置组件与源码构建

首次连接将内置核心、CLI、Windows 必要 DLL/驱动写入应用数据目录下的
`easytier/builtin-2.6.4-<payload-id>/`。每次启动校验大小、SHA-256 与可执行权限，
缺失或修改时从内置内容恢复，完整文件直接复用。此过程无需网络；早期开发版本的外部目录配置自动迁移到内置核心。

Tauri 开发和生产构建自动运行 `npm run easytier:prepare`，构建阶段缓存官方归档，
校验 `src-tauri/easytier-manifest.json` 中的固定 SHA-256，Rust 再次校验后仅嵌入所需文件。
不支持的平台或缺失、损坏的归档会使桌面构建失败，避免输出不包含核心的安装包。
直接使用 Cargo 构建桌面程序前先执行该命令；交叉编译可加 `-- --target <Rust triple>`，
离线构建可加 `-- --archive /绝对路径/官方归档.zip` 导入已下载的同版文件。
归档和生成文件不加入 Git。`--no-default-features` 的无界面库构建无需组件；测试可单独开启
`bundled-easytier` 功能验证内置核心。

### 密钥与许可

偏好配置不保存网络密钥，每次启动需重新填写，启动成功后输入框清空。
运行期私有临时配置正常停止时删除；Unix 目录权限为 0700、密钥文件为 0600，强制结束或断电可能留下临时文件。
状态不返回原始配置，日志密钥脱敏，管理 RPC 只监听随机本机回环端口并核对实例身份。

内置上游程序作为独立进程执行，未链接到 PortMate 的 Rust 库中。EasyTier 遵循 LGPL-3.0
（包含 GPL-3.0 条款），两份许可文本与对应源码链接一并内置，并写入运行目录。
PortMate 自身仍使用 Apache-2.0 许可证。
