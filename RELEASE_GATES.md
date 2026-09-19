# Release checks

This repository distributes the source-available Desktop client. Its source can be built without private repositories, service credentials or a code-signing identity.

Before publishing a source revision:

- Audit the explicit source allowlist, Git history, package contents and dependency notices. Keep Server, SaaS, sign-in issuer, hosted isolation and deployment source private.
- Build from an independent export and exercise native IPC, profile ownership, local execution and the bundled MCPs. Recheck custom stdio and authenticated remote MCP connections when their transport changes.
- Validate the companion identity service: browser consent, PKCE, exact redirects, short-lived audience-specific grants, tenant isolation, refresh rotation and revocation. Public client IDs never establish trust in a build.
- Record production migration and issuer retirement evidence privately. Never put database backups, vault keys, signing keys or environment files in this repository.
- Preserve the chosen license and third-party rights. Commercial inquiries use https://flamel.dev. This source-available license is not an OSI open-source license. See [LICENSE.md](LICENSE.md) and [DEPENDENCY_REVIEW.md](DEPENDENCY_REVIEW.md).

Source builds and the current test installers are unsigned, with automatic updates disabled. Publishing an official signed binary or enabling an updater requires its own signing and installation verification. Windows WebView2/terminal behavior under Wine and native macOS behavior remain unverified; they are preview targets. These platform limits do not prevent building or distributing the reviewed source under its license.

See [VALIDATION.md](VALIDATION.md) for measured results. Passing the checks is evidence for the tested revision, not a claim that software has no defects.
