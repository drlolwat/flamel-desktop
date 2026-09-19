# Flamel Runtime candidate

Flamel Desktop runs a local workspace without a Flamel account or an HTTP API listener. It can also connect to a standalone Server. Cloud account management, Git hosting, and the SaaS web shell remain in the private repository.

This is a **source-available candidate under Flamel Community License 1.0**. The tailored license and contributor agreement require legal review before publication. See [LICENSE.md](LICENSE.md), [NOTICE](NOTICE), and [commercial licensing](COMMERCIAL.md). Nothing in this workspace has been published or deployed.

| Package | Responsibility |
| --- | --- |
| `engine` | Workspace operations, persistence, providers, tools, native callback broker |
| `desktop` | Bundled Tauri UI, native IPC, local profiles, remote connections, native credential storage |
| `protocol` | Versioned native transport messages and limits |
| `frontend` | Versioned `@flamel/workspace` UI package and Desktop entry |
| `shared` | Shared TypeScript contracts |

## Build and verify

Requirements: stable Rust, Python 3.11+, Node.js/npm, the Tauri CLI, and native Tauri build dependencies for your platform. Linux needs GTK 3 and WebKitGTK 4.1 development packages; credential persistence uses Secret Service. Windows uses Credential Manager, and macOS uses Keychain. macOS remains a preview target.

```sh
npm ci
npm run build:ui
node scripts/build-sidecars.mjs debug
cargo check --workspace --all-targets --locked
cargo test -p flamel-engine --lib --locked
cargo test -p flamel-engine --test native_runtime --locked
npm test --workspace frontend
python3 scripts/audit-source.py
```

Build Desktop with `npm run build`; that builds the bundled MCP tools before Tauri packaging. The bundle does not contain the standalone Server executable. Source builds use declared package versions, independent of private Git history. Automatic updater publication is not configured in this candidate. FlamelGit workflows live in `.flamel/workflows`: verification on Linux, Windows cross-builds and Wine tests, manual candidate packaging, and a separate macOS preview. The Windows jobs provision cargo-xwin, LLVM, NSIS, Wine and a hash-pinned Windows Git in an isolated prefix. macOS runners need Rust, Python, and Xcode tools. Dependencies are installed in the workflow containers.

To prepare the distribution evidence, run `python scripts/dependency-sources.py artifacts`, `npm run build -- --bundles deb` (Linux) or `--bundles nsis` (Windows), `python scripts/export-source.py artifacts/flamel-runtime-source.tar.gz`, and `python scripts/package-evidence.py artifacts`. Packaging produces an unsigned review candidate with notices, exact dependency sources, source hashes, and checksums. It does not publish anything.



Use `DB_PATH`, `FLAMEL_HOME`, and `FLAMEL_VAULT_KEY_PATH` to select an isolated test profile. Existing default database and vault paths remain supported. Back up the database together with its vault key before migration. Startup preserves owner IDs; an existing database with multiple identities requires explicit Desktop profile selection.

Server hosting and account administration are implemented in the private service repository. `FLAMEL_CATALOG_REFRESH=1` enables Cloud catalog updates. The bundled catalog remains usable without them. Set `FLAMEL_PROVIDER_AUTO_UPDATE=0` to disable provider CLI update checks.

## Authentication and transport

Desktop dispatches versioned requests in-process and carries responses over native IPC in 64 KiB chunks. Realtime and terminal streams use the local workspace handlers. MCP callbacks use an authenticated Unix socket or Windows named pipe. Renderer-supplied identity headers cannot select another user. Hosted pages receive no native capability grant.

Remote credentials stay in the native host. Remembered sessions use platform credential storage; unavailable storage produces an error rather than a plaintext fallback. HTTPS origins are required except explicit loopback development connections, and credentialed redirects are rejected.

Optional Flamel sign-in uses the system browser, S256 PKCE, and a temporary loopback callback. The private integration candidate adds the `flamel-desktop` OAuth client, 10-minute Cloud access grants, 30-day rotating refresh families, separately scoped server grants, and short-lived Git assertions. Every new public-client authorization requires explicit browser confirmation. The Cloud grant is never sent to a selected Server. These new Cloud endpoints require the companion private changes and database migration; they are not deployed.

Free rebranded forks, internal business use, and paid work producing independent output are allowed. Selling Flamel, commercial hosting, advertising in it, and paid Flamel-specific services require separate commercial permission. Voluntary donations cannot gate benefits. Distributed modified builds must include corresponding source and build instructions. See [CONTRIBUTING.md](CONTRIBUTING.md) and [RELEASE_GATES.md](RELEASE_GATES.md).

See [VALIDATION.md](VALIDATION.md) for measured results and remaining platform/release checks. The Tauri security configuration follows its [capability model](https://v2.tauri.app/security/capabilities/); credential backends follow the [keyring package documentation](https://docs.rs/crate/keyring/3.6.3/source/README.md).

### Windows from Linux

Use a separate source export to avoid mixing Linux and Windows sidecars. Install cargo-xwin 0.23.1, the Rust `x86_64-pc-windows-msvc` target, LLVM/Clang, NSIS and the Tauri CLI 2.11.2, then run `npm ci` and `bash scripts/build-windows.sh`. The installer is written under `target/x86_64-pc-windows-msvc/release/bundle/nsis`.

With Wine and Xvfb installed, set `WINEPREFIX` to a disposable absolute directory named `flamel-wine.*` and run `python3 scripts/verify-windows-wine.py <installer>`. This test supplies a WebView2 registration fixture to check the unmodified installer's payload, MCP startup and uninstall; it does not validate WebView2. The FlamelGit workflows also run the Windows engine tests with a real Windows Git executable. GUI compatibility results are recorded separately in VALIDATION.md. Cross-build instructions follow https://v2.tauri.app/distribute/windows-installer/.

Signing in with Flamel opens a separate workspace picker with the account's linked servers and a local-work option. Local work retains the active Flamel account for connected services while local profile ownership stays on the device. Switching connections returns to the picker; signing out ends the account session. The login screen has no saved-session or remember-session controls.
