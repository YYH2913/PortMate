# PortMate Release Checklist

Every release is blocked until each applicable gate below has an attached command output, artifact,
or reviewer record. Do not use a successful source build as evidence that an installer is complete.

The current source version is `0.1.14`; its accumulated changes are recorded in
[CHANGELOG.md](./CHANGELOG.md). The preceding `0.1.12` commit `00b4cedf` passed all
six [Native CI jobs](https://github.com/YYH2913/PortMate/actions/runs/39376186871),
including native Windows MSI/NSIS runtime and installer checks. Its artifacts and
verification records are retained under the ignored `tmp/native-ci-review/` directory.
Version `0.1.14` needs its own build artifacts and verification records.
The signed updater-enabled `0.1.12` test installer is the baseline for the
0.1.14 upgrade test; previously published 0.1.13 packages remain unchanged.

The earlier local `0.1.12` Windows GNU x86-64 portable build is recorded in
`tmp/windows-release-0.1.12-2026-10-08.md`, including the source revision, payload
hashes, PE/version/frontend checks, and executed tests. Files under `tmp/` are local
review and build records, are ignored by Git, and are not included in commits.
That version includes the centralized `test/` layout and newly covered native diagnostic
translations. The optional SSH/Tmux automatic-reconnect fingerprint bypass introduced
in 0.1.11 is retained; its earlier local function-by-function recheck is in
`tmp/reconnect-host-key-review-0.1.11-2026-10-07.md`.
Signing acceptance remains pending.

## Version And Source

- [ ] `npm run test:release-source` confirms `package.json`, both lock files, Tauri metadata, every PortMate-owned Cargo package, licenses, and the current changelog section use the intended release identity and version. Embedded upstream forks retain their upstream package versions.
- [ ] `Cargo.lock` and `package-lock.json` are committed and the release worktree contains no unintended files.
- [ ] Release notes describe user-visible changes, security changes, migrations, and known limitations.
- [ ] `LICENSE`, package publisher, application identifier, and platform metadata are correct.

## Required Verification

- [ ] `npm ci` succeeds with the Node version in `.nvmrc`.
- [ ] `npm run test:release-source` passes from the committed release source.
- [ ] `npm run test:dependency-audit` and `npm run test:rust-dependency-audit` pass with fresh npm and RustSec advisory data.
- [ ] `npm run test:native-keyring-dependencies` confirms that desktop and MCP use only the shared platform-specific native provider boundary.
- [ ] `npm run test:portable-cross` checks the portable Rust crates for Windows GNU, Apple Silicon macOS, and FreeBSD with the locked dependency graph.
- [ ] `npm test` and `npm run build` pass.
- [ ] `npm run test:terminal-compat`, `npm run test:vttest-compat`, `npm run test:tmux-workflow`, `npm run test:tmux-version-compat`, and `npm run test:workspace-ui` pass.
- [ ] `npm run test:mcp-stdio-client`, `npm run test:mcp-http-client`, `npm run test:mcp-typescript-client`, `npm run test:mcp-python-client`, `npm run test:mcp-go-client`, `npm run test:mcp-rust-client`, `npm run test:mcp-ruby-client`, `npm run test:mcp-java-client`, `npm run test:mcp-kotlin-client`, `npm run test:mcp-csharp-client`, and `npm run test:mcp-swift-client` pass.
- [ ] `npm run test:mcp-sdk-freshness` confirms that every latest stable official SDK release is represented in the pinned compatibility matrices.
- [ ] `npm run test:ssh-server-compat` and `npm run test:tcp-telnet-server-compat` pass on a Linux Docker host.
- [ ] `npm run test:linux-desktop-smoke` passes on a Linux X11/Xwayland desktop or isolated Xvfb display.
- [ ] `npm run test:rustfmt` checks workspace formatting and external test support files.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo test --workspace -- --test-threads=4` passes without unexpected skipped integration tools.

## Packaging

- [ ] `npm run desktop:build` succeeds on clean Linux, Windows, and macOS runners.
- [ ] The `Native CI` workflow succeeds for `ubuntu-24.04`, `windows-2022`, `macos-15`, and the Linux compatibility job; retain its bundle artifacts with the release evidence.
- [ ] Linux produces DEB, RPM, and AppImage; Windows produces signed MSI/NSIS; macOS produces signed and notarized app/DMG.
- [ ] Every package contains the main executable, target-specific `portmate-mcp` sidecar, standard icons, the Apache-2.0 application license, and the JetBrains Mono SIL OFL 1.1 license in the platform resource directory.
- [ ] On Linux, `npm run test:linux-package` passes on X11/Xvfb against the freshly built DEB, RPM, and AppImage from a space-containing extraction/runtime root, including full file/symlink permission checks, canonical AppImage root links and source-identical 256x256 icons, three successful restart/legacy-migration launches, and one fail-closed two-Store conflict start from every extracted package.
- [ ] On Linux, `npm run test:linux-appimage-smoke` renders the final AppImage three times, closes it normally, rotates IPC credentials, removes each endpoint, preserves the Store SHA-256 across an idle restart and `dev.portmate.app` migration, removes the migrated legacy directory, and then rejects distinct non-empty current/legacy Stores without publishing IPC or changing either Store.
- [ ] On Windows, `npm run test:windows-package` passes against the freshly built MSI and NSIS installers using a space-containing install/runtime root, including administrative extraction, silent install/uninstall, unique co-located payload files, portable symlinks, release-source SHA-256 checks, three successful restart/legacy-migration launches, and one fail-closed two-Store conflict start.
- [ ] On macOS, `npm run test:macos-package` passes against the freshly built app and DMG using a space-containing DMG mount/runtime root, including Info.plist metadata, portable symlinks, release-source SHA-256 checks for unsigned binaries or strict code-signature verification for signed binaries, direct app/DMG payload equality, DMG verification, a read-only mount audit, three successful restart/legacy-migration launches, and one fail-closed two-Store conflict start.
- [ ] Run every official TypeScript/Python/Go/Rust/Ruby/Java/Kotlin/C#/Swift MCP SDK check with `PORTMATE_MCP_BINARY` set to the bridge extracted from each package.
- [ ] Launch the installed application, create a disposable session, restart it, and verify the same Store is loaded.
- [ ] Verify main and detached-window Tauri capabilities and the production CSP against the packaged application (automated for all three Linux packages by `npm run test:linux-package`).

## Security And Signing

- [ ] Signing keys and notarization credentials come only from the release secret store and never enter logs or artifacts.
- [ ] Windows Authenticode, Apple signing/notarization, and published SHA-256 checksums verify independently.
- [ ] Confirm generated MCP commands contain the installed executable and exact Store path but no token or credential body.
- [ ] Confirm Unix app-data endpoint/export files retain owner-only permissions where documented.

## Signed In-App Updates

### Local Windows MSI test builds on Linux

When a Windows runner is unavailable, `desktop:build:windows-msi-local` can rebuild
an unsigned test MSI using a previously verified native MSI of the **same version**.
Install `msitools` and `gcab`, or supply an extracted Ubuntu tools directory with
`--tools-root`. The template checksum must come from its native build record:

```bash
npm run desktop:build:windows-msi-local -- --template /path/to/verified.msi --template-sha256 <verified-sha256>
```

The command builds the current Windows GNU production frontend/application/sidecar,
preserves the template's installation UI, UpgradeCode and upgrade rules, assigns new
ProductCode/PackageCode values, and includes the required WebView2 loader. It rebuilds
the embedded cabinet and MSI file hashes, repairs the executable's PE checksum after
setting its MSI bundle marker, then extracts the MSI and compares every application
file with its build source. Installer UI/resource streams and unrelated tables must
remain unchanged. Output, checksums and a build record go to `target/release-artifacts/`.
Use `--skip-build` only immediately after a successful production Windows GNU build.
This is a local test packaging path; native Windows lifecycle checks remain required
before publishing a release. Existing signed MSI templates are rejected.

### Signing and publication

After native package checks pass, Native CI also archives the portable packages
and the macOS `.app.tar.gz` updater payload under `target/release/bundle/`.
Windows portable executables carry the unknown bundle marker so they continue
to offer manual upgrades instead of selecting an installer update.

Update checks query the repository's latest stable GitHub release. The selected
release must include `latest.json`, and each platform entry must point to an asset
in that exact release. The installed app pins `plugins.updater.pubkey` and requires
the package signature's trusted comment to bind the announced version. Keep the
corresponding private key in the release secret store and back it up securely;
changing the embedded public key breaks updates for previously installed builds.
This signature is separate from Windows Authenticode and Apple signing/notarization.

1. Export `TAURI_SIGNING_PRIVATE_KEY_PATH` (or `TAURI_SIGNING_PRIVATE_KEY`) and, when
   applicable, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from the release secret store.
   On CI, use repository secrets and restrict signed release builds to trusted refs.
2. Run `npm run desktop:build:update` on the intended native architecture. The
   additional updater config generates the macOS `.app.tar.gz` and signed bundle
   artifacts. Normal `desktop:build` remains available for CI/development builds.
3. Run the native package gates above. Create each platform's manifest **after**
   all finalization/signing steps. The Linux AppImage finalizer changes its bytes;
   the manifest command re-signs the final artifacts and verifies every signature
   against the embedded public key before writing output.

Use the same bilingual notes file, version/tag, and publication timestamp on all
three native runners. For example, on Windows x64:

```bash
npm run release:update-manifest -- --artifacts target/release/bundle --target windows-x86_64 --notes release-notes.md --published-at 2026-10-20T00:00:00Z --output windows-update.json
```

Use `linux-x86_64` on Linux x64 and `darwin-aarch64` on Apple Silicon. A fragment
keeps separate `windows-x86_64-msi`, `windows-x86_64-nsis`,
`linux-x86_64-appimage`, and `darwin-aarch64-app` entries. Publish only architectures
whose native package gates passed. DEB/RPM, Windows ZIP and Linux tar packages remain
manual upgrades; do not map them to an incompatible updater target.

Collect the platform fragments and merge them:

```bash
npm run release:update-manifest -- --merge windows-update.json --merge linux-update.json --merge macos-update.json --output latest.json
```

Upload the exact packages referenced by the manifest, their `.sig` files,
`latest.json`, checksums, licenses and bilingual release notes to the matching
stable GitHub Release. Upload the manifest after all package assets are present;
keep tags, asset names and bytes immutable. Exercise a confirmed upgrade from a
previous updater-enabled build on every supported native platform, including
cancellation, invalid signature, declined elevation, failed installation, child
process termination, data preservation and restart. A source build and mocked UI
checks do not substitute for those installer lifecycle checks.

## Migration And Rollback

- [x] `npm run test:release-upgrade` upgrades the versioned 0.1.0 app-data fixture with profiles, host keys, grants, logs, and workspace state, then saves and reloads it with the current code.
- [x] `npm run test:release-upgrade` verifies `dev.portmate.app` data migrates atomically to `dev.portmate.desktop`; the existing app-data tests verify two non-empty directories fail closed.
- [x] `npm run test:release-upgrade` exercises the fixed 0.1.0 SQLite v4 schema, typed mirrors, workspace, panel, command-history, and credential-journal formats on disposable copies.
- [ ] Keep the previous signed installers and checksums available until the rollout is accepted.
- [ ] Before rollback, preserve the upgraded app-data directory. Validate the previous binary on a copy; never downgrade the only live Store.

## Publish

- [ ] Upload immutable installers, detached checksums, release notes, license, and known limitations together.
- [ ] Install the uploaded artifacts on clean machines rather than retesting local build outputs.
- [ ] Smoke-test SSH, local Shell, one file transfer, MCP stdio, MCP HTTP, and application restart on each supported platform.
- [ ] Record artifact hashes, signing identities, test evidence, and rollback owner in the release record.
