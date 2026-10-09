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
cargo test --locked --workspace --no-default-features
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
仅保留必要的测试挂钩/条件编译字段；它们不进入非测试版本。
API 文档中的 Rust 示例仍随公开 API 保留并由 rustdoc 验证，不作为独立测试文件搬移。

`npm test` 先检查 `test/tsconfig.json`，再运行 Vitest。`npm run build` 只检查生产
前端。`npm run test:rustfmt` 检查 workspace，并显式检查本目录下 rustfmt 不自动遍历的
`include!` 支持文件。构建缓存和测试产物不应提交。

本地审查记录及临时复现仍留在被忽略的 `tmp/`，不纳入测试发现，也不加入 Git 提交。
需要复现时使用对应运行器单独执行，不移动或删除历史证据。

Docker、浏览器、原生 keyring 和 Windows/macOS 打包矩阵需要对应工具及系统。
测试文件搬移不代表这些外部发布门禁已执行；完整清单见 [RELEASE.md](../RELEASE.md)。
