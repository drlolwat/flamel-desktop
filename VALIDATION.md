# Separation validation

This is the separated, source-available Desktop client. The original application checkout was preserved; 1,169 baseline source-file hashes were verified unchanged. Deployment details and credentials are retained privately.

## Measured checks — 2026-09-19

| Check | Result |
| --- | --- |
| Public engine unit tests | 422 passed, 1 ignored |
| Public UI tests | 445 passed across 61 files |
| Native Desktop unit tests | 2 passed |
| Windows engine under Wine 11.12 | 404 passed; real MinGit 2.55.0.5 provisioned in the isolated prefix |
| Windows native IPC under Wine | Profile, forged-identity rejection, named-pipe callback authentication and realtime checks reached successfully; terminal-output assertion failed, so the complete integration test is not a pass |
| Windows installer under Wine | MSVC cross-build and NSIS packaging; install/uninstall, exact six-tool payload, notices and all six installed MCP initializations passed |
| Remote MCP cold start | Separate-process regression on Linux and Windows/Wine: authenticated discovery/call and invalid-token rejection passed |
| Custom MCP arguments | Quoted and Windows paths, empty arguments, literal command text, JSON argv and malformed input checks passed |
| Account picker | Browser UI tests with mocked native OAuth: separate server screen, local access retaining account, server selection, remount, sign-out, empty/offline server list passed |
| Companion private service tests | 378 passed, 1 ignored |
| Companion Cloud tests | 15 passed; isolated consent/grant, actual migrations and PostgreSQL backup/restore passed |
| Real staged Cloud and Server | Browser consent/origin, exact callback preservation, PKCE, consent/code replay, linked-server and audience isolation, admin/internal/fleet denial, concurrent refresh replay, revocation, suspension and logout passed |
| Live staged realtime transport | An open WebSocket closed after grant-family revocation; existing Flamel Web, Chattle Web and WebServer management sign-in contracts passed |
| Public and private Rust workspaces | All-target checks passed |
| Desktop UI and private web consumer | TypeScript and production asset builds passed |
| Independent source export | No Git checkout: source audit, locked npm install, 440 UI tests, full release Desktop/MCP build and Linux Debian packaging passed |
| Native Linux integration | Profile selection, forged identity rejection, callback ownership, realtime/terminal, no TCP listener, shutdown passed |
| Restored profile | Isolated live-database backup: four owner-table hashes unchanged, two identities preserved, SQLite integrity passed |
| Populated credential fixture | Seven values sealed; repeated migration preserved state |
| Isolated Cloud database integration | Consent tied to browser session/user, one-use consent, audience/membership checks, concurrent refresh replay and revocation passed |
| Source and npm package boundary | 540 allowlisted source files; workspace/shared package inspection passed |
| Dependencies | JavaScript audits: zero known vulnerabilities. Rust audits: zero vulnerability-class advisories; documented informational warnings remain. Exact dependency sources, SBOM and notices generated |
| Linux installer | Extracted release package contains seven local/client executables and required notices; no private executables or maintainer legal name |
| Packaged Linux MCPs | Real tool calls through all six executables and the main gateway passed; native planning callback, saved argv, custom stdio discovery/execution and authenticated remote discovery/execution passed. No paid provider turn was invoked |
| Linux GUI | Packaged flat login, local workspace, no TCP listener in the process tree, and maximize/restore verified on Samsung HDMI-A-4 |
| Private fresh install | Real loopback HTTP bootstrap/login, forged-token rejection, refresh rotation and replay revocation passed |
| Build separation | 16 active pipeline/package regressions and 4 standalone source-boundary tests passed |

The final runtime code and lockfiles were built from the standalone source export. Existing compiler warnings remain. The final private web and Cloud npm audit rechecks completed with zero known vulnerabilities. The final lockfile edits added integrity metadata without changing JavaScript package versions. These checks establish an implementation candidate; platform limits below remain separate.

## Boundaries and remaining gates

The public engine contains local workspace execution and persistence. Server/SSO issuer, account authorization, worker fleet, hosted isolation, Actions executor, and hosted assistant implementations belong to the private service. Local execution rejects remote-worker requests and imports legacy identity labels without adopting server roles or modifying private tables. Native Desktop has no Flamel HTTP listener; an OAuth loopback callback is temporary.

The private service consumes a hash-pinned snapshot of the public engine. Its legacy combined src-server tree and Desktop packaging scripts have been removed from the private candidate and preserved outside it for rollback. Active builds use the separated workspaces; Desktop CI lives in the public repository’s `.flamel/workflows`. The private rollout document records the service entry point and required identity migration order.

The individual licensor is recorded privately outside repositories. Public licensing inquiries go through Flamel.dev; no separate corporate entity is asserted. Source provenance, checksums, dependency sources and automated notices are generated for reviewed releases. Third-party material retains its own license. See [DEPENDENCY_REVIEW.md](DEPENDENCY_REVIEW.md). Source builds are unsigned and automatic updates are disabled. Windows GUI/terminal and native macOS validation remain outstanding, so those platforms are previews. Git assertions and independently created Forge tokens have separate revocation lifecycles.

The source exporter uses an explicit allowlist and excludes Git history, ignored artifacts, caches, local databases, and credential files. Passing this audit establishes the configured boundaries; it is not a guarantee that software contains no security defects. See [RELEASE_GATES.md](RELEASE_GATES.md).

## Windows compatibility testing — 2026-09-19

The Windows client and six tools were cross-compiled with cargo-xwin 0.23.1 for x86_64-pc-windows-msvc. The NSIS installer was tested in a disposable Wine 11.12 prefix. Installer testing deliberately uses a WebView2 registration fixture and does not count as browser validation. Windows engine tests use a real Windows Git; their fixtures now compare decoded Windows paths, canonical repository paths, platform path separators and controlled Git newline handling.

GUI attempts used the real Microsoft WebView2 153.0.4234.48 runtime in separate Wine 11.12 and Proton Experimental 11.0 (20260910b) prefixes. The runtime installed, but browser subprocesses failed during startup; logs include Wine's unsupported TokenSecurityAttributes query. Diagnostic browser flags did not establish a working rendered interface. The Windows native integration executable passed its profile, identity, named-pipe callback and realtime assertions, then failed because the terminal returned an exit event without the expected PowerShell output. Terminal behavior remains unverified under Wine. No sandbox-disabling flags were added to the product. Windows GUI compatibility remains unverified, and the Wine result must not be presented as native Windows certification.

The current Linux package includes the separate account workspace picker and was opened on the Samsung display. Signing in keeps account access in the native host; selecting local work does not revoke it or replace existing local profile ownership. The browser UI tests mock native authentication. Separate real HTTP tests against the isolated companion Cloud and Server now verify the authorization endpoints, existing browser clients and live WebSocket revocation; production evidence is recorded separately below.

The packaged MCP checks caught and fixed a missing TLS crypto-provider initialization in the remote MCP client. They also cover exact custom argument handling, including quoted paths and Windows separators. Run `scripts/verify-packaged-mcp.py` against the packaged tool directory to repeat the checks; its optional runtime checks require an isolated native Desktop profile.

## Companion production cutover — 2026-09-19

The companion Cloud and Server rollout completed before source publication. Public HTTPS checks passed for consent/PKCE, linked-server selection, scoped workspace access, administrator denial, immediate revocation and closure of an established WebSocket through the public proxy. Retired issuer tokens are rejected and only the replacement public key is served. Existing browser sessions were retained. The Server now sends a WebSocket close frame on grant revocation so proxies propagate the closure.

Production verification retained the recorded workspace table counts, account identities and roles, credential values and vault key. The existing access-list sync refreshed membership timestamps. Public sites and the web-client origin check passed; Actions polling resumed. Backup and deployment receipts remain private.
