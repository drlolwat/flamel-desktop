# Third-party notices and provenance

Flamel-owned source is governed by the root LICENSE.md and Required Notice. Third-party source keeps its own terms; the root license does not replace them.

| Embedded component | Terms retained |
| --- | --- |
| `vendor/wayland-scanner` | MIT, original `LICENSE.txt` retained |
| `engine/resources/ponytail` | MIT, original `LICENSE` retained |
| Cargo and npm dependencies | Their respective package licenses; exact versions in Cargo.lock and package-lock.json |
| Runtime Catalog signature fixtures | Explicitly generated test-only signing key, documented in fixtures/runtime-catalog/README.md |

The candidate was exported from an explicit source allowlist into a fresh local Git repository without private history. Operational deployment files, Cloud/Forge implementation source, and the hosted ServerPicker/auth shell were excluded. Private service URLs in client protocols are intentional integration endpoints, not embedded credentials.

The source audit is a bounded automated check, not proof of authorship or a complete license clearance. Before publication, the owner must confirm rights to Flamel branding, icons, installer artwork, agent prompts, and catalog content. The packaging tools retain upstream license texts and README attribution in `DEPENDENCY_LICENSES.txt`, record all locked components in `dependencies.cdx.json`, and list missing standalone license documents in `license-review.json`. `dependency-sources.tar` contains the exact Cargo/npm registry sources, verified against lockfile checksums. Redistribute that archive with the binary evidence to preserve corresponding third-party source, including file-copyleft components. Optional platform packages and build/dev dependencies are included conservatively. The review list must not be mistaken for legal clearance.
