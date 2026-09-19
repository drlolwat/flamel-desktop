# Dependency review — 2026-09-19

The candidate locks rustls 0.23.45, event-listener 5.4.2, mysql 28.0.2, and chacha20 0.10.2. This fixes the Rust TLS and event-listener advisories found during release preparation and replaces two yanked versions. Final public, private web and Cloud JavaScript audits reported zero known vulnerabilities. Integrity-only lockfile repairs did not change JavaScript package versions. Rust audit reports zero vulnerability-class advisories, with informational warnings retained below; this is not a claim of zero risk.

## Informational soundness warnings

- **RUSTSEC-2024-0429, glib 0.18.5:** the GTK 3 stack pins this older major version. A source search of the resolved dependencies found `VariantStrIter` / `array_iter_str` references only in glib's own implementation, documentation, and tests, not application callers. This is bounded source evidence, not a proof of unreachability. Upgrade with the Tauri/GTK stack when supported. [Upstream advisory](https://rustsec.org/advisories/RUSTSEC-2024-0429.html).
- **RUSTSEC-2026-0253, lru 0.16.4:** mysql 28.0.2 uses this crate for a statement cache keyed by `u32`. The advisory requires a key destructor that panics during unwinding; these keys have no destructor, and release builds use `panic = "abort"`. The current cache use does not meet that trigger. Reassess if dependency usage or the release panic strategy changes. [Upstream advisory](https://rustsec.org/advisories/RUSTSEC-2026-0253.html).

These warnings have not been suppressed from cargo-audit. Public dependencies also retain maintenance warnings for fxhash, proc-macro-error, rustls-pemfile, and the unic Unicode crates. Resolve them through compatible upstream upgrades; the presence of a maintenance warning alone does not demonstrate an exploitable defect.

## Distribution evidence

`dependencies.cdx.json` inventories all locked Cargo/npm dependencies, including build/dev and optional platform packages. `DEPENDENCY_LICENSES.txt` preserves available upstream license documents and README attribution. `license-review.json` explicitly records packages without separate license files; it is not an approval record. Exact third-party package sources are retained in `dependency-sources.tar`, with their registry integrity checked against both lockfiles. The patched vendored Wayland source and its license are in the public source archive.

Review the generated license information and project authorship/branding rights before publication. Do not apply the Flamel Community License to third-party material, omit its notices, or infer license compatibility solely from this inventory. Changes to either lockfile require regenerating this evidence.
