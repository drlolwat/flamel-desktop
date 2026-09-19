# Flamel Desktop

A desktop workspace for coding with AI agents. Work with projects, files, Git and terminals on your own computer, or connect to a Flamel Server.

**You can work locally without a Flamel account.** Signing in lets you choose one of the servers linked to your account or keep working locally under that account. AI providers may require their own account, credentials or CLI installation.

This repository contains the desktop client and local workspace engine. Desktop does not host a Flamel Server. The hosted service, Server and sign-in service implementations are private.

## Build it yourself

You need **Git, Node.js 22+, Python 3.11+, Rust stable** and the [Tauri prerequisites for your platform](https://v2.tauri.app/start/prerequisites/).

On Ubuntu 24.04 or Debian 12+, install the native build packages:

```sh
sudo apt update
sudo apt install build-essential pkg-config curl file python3 python-is-python3 \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libdbus-1-dev libssl-dev libxdo-dev
```

On Windows, install Visual Studio Build Tools with **Desktop development with C++**, the Windows SDK and Microsoft WebView2. Use a normal PowerShell terminal with Git, Node, Python and Cargo on your PATH. macOS requires Xcode Command Line Tools and remains an unverified preview target.

Then clone and build:

```sh
git clone https://git.flamel.dev/lolwat/flamel-desktop.git
cd flamel-desktop
cargo install tauri-cli --version 2.11.2 --locked
npm ci
npm run build -- --bundles deb
```

The Linux installer is in `target/release/bundle/deb/`. On Windows, replace the final command with `npm run build -- --bundles nsis`; the installer will be in `target/release/bundle/nsis/`.

The build includes the local tools and license notices. It does not need the private Flamel repository, a Flamel account or release-signing credentials. The first build downloads dependencies and can take a while.

## Run while developing

After installing the prerequisites and running `npm ci`:

```sh
node scripts/build-sidecars.mjs debug
python3 scripts/dependency-notices.py desktop/resources/notices
npm run dev
```

On Windows, use `python` if `python3` is unavailable. To run the tests:

```sh
npm test
```

## What works today

- **Linux:** built and tested, including the desktop window, local workspace and native transport.
- **Windows:** cross-build, installer and 404 engine tests passed under Wine. The WebView2 interface and terminal remain unverified under Wine/Proton.
- **macOS:** source preview; no native validation yet.

Source builds are unsigned and do not receive automatic Flamel updates. See [VALIDATION.md](VALIDATION.md) for the test results and [BUILDING.md](BUILDING.md) for Windows cross-builds and isolated development profiles.

## MCP tools and integrations

The build includes Flamel's tools for files, code, shell commands, plans, Git, docs, databases, browser automation and memory. The main `flamel` MCP loads its bundled helpers automatically. Local tools work without a Flamel account; hosted operations require the relevant account and permissions.

In a local workspace, add your own MCP in **Settings → Capability Hub → Add**, using a local command or a remote MCP URL. Connect it and run its connection test to discover the tools. Local commands run on the computer or server hosting the selected workspace. Install any dependencies that your MCP needs there. In Arguments, quote paths containing spaces or paste a JSON array of strings. On a Flamel Server, its administrator controls which integrations are available.

Git tools need Git on your PATH. Browser tools need Chrome or Chromium and Node.js; the browser helper uses `chrome-devtools-mcp@1.7.0` and can download it through `npx` on first use. Browser automation runs headlessly in an isolated session by default. Database drivers are built in.

Packaged tool checks cover all six executables, calls through the main Flamel MCP, custom local commands, and authenticated remote MCP calls. See [BUILDING.md](BUILDING.md) to run them yourself.

## License and contributions

Flamel Desktop is **source-available under the [Flamel Community License 1.0](LICENSE.md)**. You can use it, modify it and distribute free forks under the license terms. Internal business use and paid work producing independent output are allowed. Monetizing Flamel itself requires a commercial license; contact [Flamel.dev](https://flamel.dev).

Modified distributions must use distinct branding and include their corresponding source and build instructions. Third-party components keep their own licenses. Read [CONTRIBUTING.md](CONTRIBUTING.md) before sending changes.
