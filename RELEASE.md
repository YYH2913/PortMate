# PortMate Release Checklist

Every release is blocked until each applicable gate below has an attached command output, artifact,
or reviewer record. Do not use a successful source build as evidence that an installer is complete.

## Version And Source

- [ ] `package.json`, `src-tauri/tauri.conf.json`, and all Cargo packages use the intended version.
- [ ] `Cargo.lock` and `package-lock.json` are committed and the release worktree contains no unintended files.
- [ ] Release notes describe user-visible changes, security changes, migrations, and known limitations.
- [ ] `LICENSE`, package publisher, application identifier, and platform metadata are correct.

## Required Verification

- [ ] `npm ci` succeeds with the Node version in `.nvmrc`.
- [ ] `npm test` and `npm run build` pass.
- [ ] `npm run test:terminal-compat`, `npm run test:vttest-compat`, `npm run test:tmux-workflow`, `npm run test:tmux-version-compat`, and `npm run test:workspace-ui` pass.
- [ ] `npm run test:mcp-stdio-client`, `npm run test:mcp-http-client`, `npm run test:mcp-python-client`, `npm run test:mcp-go-client`, `npm run test:mcp-rust-client`, `npm run test:mcp-ruby-client`, `npm run test:mcp-java-client`, `npm run test:mcp-kotlin-client`, `npm run test:mcp-csharp-client`, and `npm run test:mcp-swift-client` pass.
- [ ] `npm run test:ssh-server-compat` and `npm run test:tcp-telnet-server-compat` pass on a Linux Docker host.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo test --workspace -- --test-threads=4` passes without unexpected skipped integration tools.

## Packaging

- [ ] `npm run desktop:build` succeeds on clean Linux, Windows, and macOS runners.
- [ ] The `Native CI` workflow succeeds for `ubuntu-24.04`, `windows-2022`, `macos-14`, and the Linux compatibility job; retain its bundle artifacts with the release evidence.
- [ ] Linux produces DEB, RPM, and AppImage; Windows produces signed MSI/NSIS; macOS produces signed and notarized app/DMG.
- [ ] Every package contains the main executable, target-specific `portmate-mcp` sidecar, standard icons, and license metadata.
- [ ] On Linux, `npm run test:linux-package` passes against the freshly built DEB, RPM, and AppImage, including full file/symlink permission checks.
- [ ] On Windows, `npm run test:windows-package` passes against the freshly built MSI and NSIS installers, including administrative extraction, silent install/uninstall, unique co-located payload files, portable symlinks, and release-source SHA-256 checks.
- [ ] On macOS, `npm run test:macos-package` passes against the freshly built app and DMG, including Info.plist metadata, portable symlinks, release-source SHA-256 checks for unsigned binaries or strict code-signature verification for signed binaries, direct app/DMG payload equality, DMG verification, and a read-only mount audit.
- [ ] Run every official TypeScript/Python/Go/Rust/Ruby/Java/Kotlin/C#/Swift MCP SDK check with `PORTMATE_MCP_BINARY` set to the bridge extracted from each package.
- [ ] Launch the installed application, create a disposable session, restart it, and verify the same Store is loaded.
- [ ] Verify main and detached-window Tauri capabilities and the production CSP against the packaged application (automated for all three Linux packages by `npm run test:linux-package`).

## Security And Signing

- [ ] Signing keys and notarization credentials come only from the release secret store and never enter logs or artifacts.
- [ ] Windows Authenticode, Apple signing/notarization, and published SHA-256 checksums verify independently.
- [ ] Confirm generated MCP commands contain the installed executable and exact Store path but no token or credential body.
- [ ] Confirm Unix app-data endpoint/export files retain owner-only permissions where documented.

## Migration And Rollback

- [ ] Test upgrade from the previous release using a copied app-data directory with profiles, host keys, grants, logs, and workspace state.
- [ ] Verify `dev.portmate.app` data migrates atomically to `dev.portmate.desktop`; two non-empty directories must fail closed.
- [ ] Exercise SQLite, workspace, panel, command-history, and credential-journal migrations on disposable copies.
- [ ] Keep the previous signed installers and checksums available until the rollout is accepted.
- [ ] Before rollback, preserve the upgraded app-data directory. Validate the previous binary on a copy; never downgrade the only live Store.

## Publish

- [ ] Upload immutable installers, detached checksums, release notes, license, and known limitations together.
- [ ] Install the uploaded artifacts on clean machines rather than retesting local build outputs.
- [ ] Smoke-test SSH, local Shell, one file transfer, MCP stdio, MCP HTTP, and application restart on each supported platform.
- [ ] Record artifact hashes, signing identities, test evidence, and rollback owner in the release record.
