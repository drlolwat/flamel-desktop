# Publication gates

Status: private candidate; publication is blocked until every applicable gate has recorded evidence.

- Legal: review the tailored license and contributor agreement; use the privately recorded individual licensor and the public contact at https://flamel.dev; check authorship and dependency rights. This text is a proposed implementation of the selected policy, not legal clearance. A restricted-commercial-use license is source-available, not OSI open source: https://opensource.org/osd.
- Source boundary: reviewed allowlist, no service source/history/secrets, dependency and package inspection, independent public-only build, review the generated dependency sources, notices and SBOM. Start the public repository with fresh history.
- Runtime: offline profile, populated credential restore, no Flamel HTTP listener, native IPC ownership, local/remote switching and shutdown; Linux and Windows cross-build/Wine tests, with WebView2 and native-platform limits recorded. macOS is preview until native validation.
- Identity: staged browser consent and PKCE, concurrent refresh replay/revocation, wrong audience/server/tenant, suspension, admin denial and socket expiry. Native-client guidance: https://www.rfc-editor.org/rfc/rfc8252.html. Never infer an official build from a public client identifier.
- Release artifacts: rebuild from the reviewed source; inspect package contents and dependencies; platform code signatures, checksums, provenance, installer/update tests. Old candidate binaries do not validate newer source.
- Operations: stage the companion private migrations and endpoints, check rollback/restore and existing clients, then obtain explicit authorization to deploy or publish. No production deployment or publication is part of this checkout preparation.
