# 测试目录 / Test layout

所有测试代码、测试专用运行器和夹具统一维护在此目录。生产代码仍在 `src/`、
`src-tauri/src/`、`crates/*/src/`，构建与打包工具仍在 `scripts/`。

```text
test/
├── frontend/       React / TypeScript 单元和界面回归测试
├── scripts/        构建、运行器、依赖和仓库边界的 Vitest 测试
├── rust/
│   └── <package>/
│       ├── unit/          原内嵌 Rust 模块测试
│       ├── suite/         多模块测试套件
│       ├── support/       测试辅助函数、导入和 fault injection
│       ├── integration/   Cargo 集成测试
│       └── bin/           原生 keyring / Tmux 测试探针（仅 portmate）
├── tooling/        兼容矩阵、浏览器回归、发布检查与 MCP SDK 客户端
├── compat/         Docker 服务端、协议矩阵和浏览器 harness
├── fixtures/       版本升级等固定输入
├── setup/          Vitest 初始化
├── tsconfig.json   测试 TypeScript 类型检查
└── vitest.config.ts
```

## 运行 / Run

从仓库根目录执行，现有 npm 测试命令名称不变：

```bash
npm test
npm run test:rustfmt
cargo test --locked --workspace --no-default-features --features portmate/test-probes
npm run test:release-upgrade
npm run test:portable-vault
npm run test:terminal-compat
npm run test:workspace-ui
```

运行单个前端用例：

```bash
npx vitest run --config test/vitest.config.ts test/frontend/release-upgrade.test.ts
```

Rust 单元用例通过产品模块内的 `#[cfg(test)]` + `#[path]` / `include!` 挂载，
而不是改成全部公开 API 或独立 Cargo workspace。原模块名、私有可见性和 `--exact`
过滤器保持不变。Cargo 集成测试和探针路径显式登记在所属包的 `Cargo.toml` 中。
两个探针还要求显式启用 `test-probes` feature；npm 原生测试运行器会自动启用，
普通应用构建/打包不会启用，包检查也会拒绝意外夹带的探针可执行文件。
仅保留必要的测试挂钩/条件编译字段；它们不进入非测试版本。
API 文档中的 Rust 示例仍随公开 API 保留并由 rustdoc 验证，不作为独立测试文件搬移。

`npm test` 先检查 `test/tsconfig.json`，再运行 Vitest。`npm run build` 只检查生产
前端。`npm run test:rustfmt` 检查 workspace，并显式检查本目录下 rustfmt 不自动遍历的
`include!` 支持文件。构建缓存和测试产物不应提交。

本地审查记录及临时复现仍留在被忽略的 `tmp/`，不纳入测试发现，也不加入 Git 提交。
需要复现时使用对应运行器单独执行，不移动或删除历史证据。

Docker、浏览器、原生 keyring 和 Windows/macOS 打包矩阵需要对应工具及系统。
测试文件搬移不代表这些外部发布门禁已执行；完整清单见 [RELEASE.md](../RELEASE.md)。

Python MCP SDK 运行器优先使用 `PORTMATE_PYTHON`，否则在 Windows 上优先使用
PATH 选定的 `python`（兼容 setup-python/已激活 venv），仅在不可用时回退到 `py -3`。
Windows 使用各 SDK 的 `requirements-windows.txt`，其他平台使用 `requirements.txt`；
两者都要求实际安装的完整依赖快照精确匹配，不忽略平台专属依赖。

Maven 与 .NET SDK 引导在 Windows 上明确调用 `%SystemRoot%\System32\tar.exe`，
避免 Ruby/MSYS2 的 GNU tar 把盘符当成远程主机或无法解包 ZIP。归档仍先做 SHA-512
校验，解包失败/超时仍会中止；不重排全局 PATH 或改用未校验的工具下载。

Windows 安装包检查单独限制 MSI 行政解包期限，并把原始 MSI 日志保留在
`target/native-ci/windows-msi-extract.log`，随 CI diagnostics 上传。错误输出包含有界的
UTF-8/UTF-16 日志尾部，安装包生命周期也逐阶段打印进度，避免整项 job 超时后无法定位。
Windows 包验证步骤另设 20 分钟上限，为后续始终执行的诊断/构建产物上传留出时间。

## SFTP 性能对照 / SFTP performance probes

本地 OpenSSH 组件矩阵是显式启用的 ignored 测试，涵盖 russh/libssh 上传、
串行与流水下载、远端复制，以及闲置终端竞争。三轮交替块大小并逐字节核验结果。
这些数据是本机组件对照，不代表 Windows GUI 或现场局域网速度：

```sh
CARGO_PROFILE_TEST_OPT_LEVEL=3 cargo test --locked --offline -p portmate \
  --no-default-features --lib \
  tests::sftp_component_benchmarks::sftp_component_throughput_matrix \
  -- --exact --ignored --nocapture --test-threads=1
```

默认文件为 32 MiB，可用 `PORTMATE_PERF_SIZE_MIB` 设置 1–128 MiB。
另外的隔离协议测试为每个响应注入 2 ms 延迟，比较 1/4/8/16 路 READ；
它不包含 SSH 加密或真实网卡延迟：

```sh
cargo test --locked --offline -p russh-sftp \
  client::fs::file::tests::read_pipeline_latency_benchmark \
  -- --exact --ignored --nocapture
```
