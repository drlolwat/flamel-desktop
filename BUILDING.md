# More build options

Start with the [README](README.md) for a normal desktop build.

## Build Windows on Linux

Use a separate clone so Linux and Windows tool binaries do not mix. Alongside Node, Python and Rust, install LLVM/Clang, LLD, NSIS, pkg-config and the Ayatana AppIndicator development package. On Debian or Ubuntu:

```sh
sudo apt install clang lld llvm nsis pkg-config libayatana-appindicator3-dev
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin --version 0.23.1 --locked
cargo install tauri-cli --version 2.11.2 --locked
npm ci
bash scripts/build-windows.sh
```

The installer is in `target/x86_64-pc-windows-msvc/release/bundle/nsis/`. Building downloads the Microsoft SDK and CRT needed for the cross-build. See the [Tauri Windows packaging documentation](https://v2.tauri.app/distribute/windows-installer/).

The `.flamel/workflows` files also build and test the client on FlamelGit. The Wine installer check uses a test-only WebView2 registration fixture; it verifies the installer and bundled tools, not the browser interface. See [VALIDATION.md](VALIDATION.md) for the limits of those tests.

## Keep development data separate

You can set `DB_PATH`, `FLAMEL_HOME` and `FLAMEL_VAULT_KEY_PATH` to use a disposable local profile. For example, in Bash:

```sh
flamel_test_root=$(mktemp -d)
export DB_PATH="$flamel_test_root/flamel.db"
export FLAMEL_HOME="$flamel_test_root/workspace"
export FLAMEL_VAULT_KEY_PATH="$flamel_test_root/vault.key"
export FLAMEL_PROVIDER_AUTO_UPDATE=0
export FLAMEL_CATALOG_REFRESH=0
npm run dev
```

Back up an existing database together with its vault key before testing a migration. Do not commit either file. Cloud account sign-out and local profile ownership are separate: working locally keeps the current account session until you sign out or close it.

## Check the source boundary

```sh
python3 scripts/audit-source.py
python3 scripts/test-source-boundary.py
cargo test -p flamel-engine --test native_runtime --locked
```

The explicit source allowlist prevents private service files, local data and generated artifacts from entering a source export. When adding a public file, review it and update `scripts/source-allowlist.json`.

## Check packaged MCPs

Install Git, Node.js and Chrome/Chromium, then point the check at the directory containing the six packaged MCP executables:

```sh
python3 scripts/verify-packaged-mcp.py --bin-dir desktop/resources/bin
```

This creates a disposable fixture directory and exercises actual tool calls, including a headless browser. Docs checks require internet access. Logs remain in the printed fixture directory. It does not invoke a paid AI provider.

For native callback and custom integration checks, start Desktop with an isolated profile as described above, open a local workspace, and add `--runtime-file /path/to/isolated/config/@flamel/cli-runtime.json`. These optional checks currently use Unix sockets. Use only an isolated test profile: the checks create test integration records there. The runtime file contains a local credential; keep it private.
