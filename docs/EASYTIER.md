# EasyTier integration / EasyTier 集成

## English

PortMate manages an optional, unmodified [EasyTier 2.6.4](https://github.com/EasyTier/EasyTier/releases/tag/v2.6.4)
core as a child process. It queries that version's CLI for status. It does not install
a system service or join a network on application startup. The layout follows the
official [configuration](https://github.com/EasyTier/EasyTier/blob/v2.6.4/assets/config-page.png)
and [running-status](https://github.com/EasyTier/EasyTier/blob/v2.6.4/assets/running-page.png) screens.

### Configuration and operation

Open **Tools → EasyTier networking**. In **Settings**, download the official core
and CLI, or select the directory containing both programs. Automatic downloads
support Windows/Linux x64 and macOS ARM64, with pinned archive SHA-256 verification
and download progress. Both programs must be version 2.6.4. The Windows download
also installs the upstream runtime DLLs and driver alongside the executables.

**Configure network** offers DHCP or static virtual IPv4, the shared network name
and secret, and three networking methods:

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

**Connect network** starts the core and opens **Running status**. This page shows
the local node, connected peers, routes, latency, cumulative traffic and redacted logs.
The local node is excluded from the peer count. Saved configuration remains a single
configuration; **New configuration** resets the editable draft. It does not create
an additional simultaneous network or delete the saved configuration until saved.

Closing the dialog keeps the core running. **Disconnect network**, normal PortMate
exit and verified updater preparation stop the owned process tree. No service or
automatic restart outside PortMate is installed.

### Secrets, control and component files

Saved settings contain no network secret. Enter the secret on each start; the form
clears it after successful launch. The core reads it from a temporary configuration
in a private application-data directory (0700 directory and 0600 file on Unix),
which normal stop/exit removes. Abrupt power loss or forced process termination can
leave this file behind. Status responses omit the raw core configuration; logs
redact credentials. The management RPC listens on a random loopback port, accepts
only loopback addresses and verifies the expected instance identity on queries.

The downloaded component is stored under the PortMate application-data directory,
in `easytier/component-2.6.4/`. It can be replaced with an independently installed
copy by selecting its absolute directory. PortMate's source tree contains no
EasyTier binary. The optional component is licensed under
[LGPL-3.0](../licenses/EasyTier-LGPL-3.0.txt), incorporating
[GPL-3.0](../licenses/GPL-3.0.txt); both license texts and the matching
[upstream source link](https://github.com/EasyTier/EasyTier/tree/v2.6.4) accompany
managed installations. PortMate itself retains its existing Apache-2.0 license.

## 中文

PortMate 将未修改的 [EasyTier 2.6.4](https://github.com/EasyTier/EasyTier/releases/tag/v2.6.4)
核心作为可选子进程托管，使用同版 CLI 查询状态，不安装系统服务，也不会随应用启动自动加入网络。
界面参考官方的配置与运行状态页面。

### 配置与使用

打开 **工具 → EasyTier 组网**，在 **设置** 下载官方组件，或选择同时包含
`easytier-core` 和 `easytier-cli` 的目录。两者均须为 2.6.4。
自动下载支持 Windows/Linux x64、macOS ARM64，显示进度并校验固定 SHA-256。
Windows 组件同时包含上游所需的运行库和驱动文件。

**配置网络** 提供 DHCP/静态虚拟 IPv4、网络名称与密钥，组网方式包括：

- **公共服务器**：选择官方 `public.easytier.top:11010` 的 TCP 或 UDP 地址。
- **手动连接**：逐项添加或删除对等节点地址，最多 32 个。
- **独立组网**：不主动连接节点，其他设备通过本机监听地址加入。

只有选择公共服务器才会添加该公共节点；EasyTier 自带的 NAT 探测仍可能访问 STUN 服务器。
**高级设置** 提供设备名称、地址模式、监听地址、MTU 和低延迟优先。清空全部监听地址会关闭入站监听。
UPnP 端口映射和其他网络的中继转发默认关闭。

DHCP 与静态地址模式创建虚拟网卡，需要 Windows 管理员权限、macOS root 权限，
或 Linux 相应的网络管理权限（包括 `CAP_NET_ADMIN` 和 `/dev/net/tun` 访问权限）。
PortMate 不会自动提权。**仅中继** 模式无需虚拟网卡，本机 SSH 等应用也无法通过该模式的虚拟 IP 访问网络。

点击 **连接组网** 后进入 **运行状态**，查看本机信息、已连接节点、路由、延迟、累计收发量和脱敏日志。
节点列表不重复计入本机。当前保存一份网络配置；**新建配置** 只重置草稿，保存后才替换原配置，
不表示同时运行多个网络。关闭此窗口不会停止组网；点击断开、正常退出 PortMate，
或准备安装已验证的更新时会停止所托管的进程树。

### 密钥、管理端口与许可

偏好配置不保存网络密钥，每次启动需重新填写，启动成功后输入框清空。
运行期密钥位于应用数据目录下的私有临时配置中，Unix 目录权限为 0700、文件权限为 0600，
正常停止或退出会删除。断电或强制结束进程可能留下临时文件。
状态接口不返回核心的原始配置，日志对密钥脱敏；管理 RPC 仅监听随机本机回环端口，
只允许回环地址访问，状态查询核对实例身份。

托管组件位于应用数据目录的 `easytier/component-2.6.4/`，可以改选自行安装的同版程序目录。
源码仓库不包含 EasyTier 二进制文件。可选组件遵循 LGPL-3.0（包含 GPL-3.0 条款），
下载后的目录附带两份许可文本及对应上游源码链接。PortMate 自身仍使用原有 Apache-2.0 许可证。
