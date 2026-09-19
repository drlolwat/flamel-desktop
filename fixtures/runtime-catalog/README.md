# Runtime Catalog contract fixtures

These deterministic fixtures are shared by the Cloud publisher tests and the Rust verifier tests. fixture-private-key.pem is a generated test-only key and is referenced only from test-only Rust code. It must never be installed as a Flamel signing key.

The named payload is shared-fixture-v1: one 427-byte skill artifact with SHA-256 fae0463740fe05c819549bd0bfab42ee02a9b851527e4144d77f1487fb2d4459, signed for audience flamel-runtime-catalog by key runtime-catalog-fixture-1. Its valid beta revision is 41. The replay, incompatible-runtime, revoked-key, bad-signature, and higher-revision rollback files preserve the same canonical contract while changing one acceptance boundary.

## Compatibility matrix

| Fixture or case | Expected result | State effect |
| --- | --- | --- |
| valid-manifest.jws + exact artifact | Activate revision 41 | Atomic active/previous/highest update |
| Valid ETag with HTTP 304 | Keep active revision | Refresh success age and ETag only |
| bad-signature-manifest.jws | Reject | None |
| Unknown or cumulatively revoked key | Reject | None |
| replay-manifest.jws or concurrent duplicate | Reject | None |
| incompatible-manifest.jws | Reject | None |
| Missing, malformed, oversized, or bad-hash artifact | Reject | Transaction rolls back |
| Projection failure | Report error | Existing provider projection and catalog snapshot remain usable |
| rollback-manifest.jws | Activate higher revision 43 | Prior content becomes active without decrement |
| Local rollback control | Restore previous verified snapshot and freeze | Highest accepted revision is preserved |

## State transitions and controls

- Startup and the 15-minute scheduler share one async refresh lock with the manual refresh route. SQLite independently rejects a replay or concurrent downgrade inside the activation transaction.
- freeze prevents catalog network access. unfreeze permits it; refresh performs a bounded 30-second sync; channel selects stable or beta.
- A sync verifies RS256, issuer, audience, time window, runtime range, monotonic revision, cumulative revocations, exact fixed-origin artifact URLs, byte limits, and hashes before activation.
- Any fetch, verification, import, or transaction error records concise telemetry while retaining the last-known-good snapshot.
- Local recovery uses POST /api/admin/runtime-catalog with {"action":"rollback"}. Fleet rollback publishes a new signed higher revision through POST /api/admin/runtime-catalog/rollback; revisions are never decremented or unsigned.

## Local measurements

Fixed environment: this repository and toolchain on 2026-08-24 UTC, Rust debug test binary, in-memory SQLite, no external network, 31 cold samples, 31 local signing-to-active contract samples, and 101 warm 304 samples.

| Measure | p50 | p95 |
| --- | ---: | ---: |
| Cold fixture verify/import/activate | 45.994 ms | 53.545 ms |
| Local sign-to-active contract | 46.795 ms | 47.660 ms |
| Warm 304 state refresh | 71 µs | 82 µs |

Cold response bodies total 1,529 bytes; warm 304 bodies transfer 0 bytes with 101/101 cache hits. The measurement command used 34.44 seconds wall time, 44.03 seconds user CPU, 13.11 seconds system CPU, and 1,706,828 KiB peak RSS including Rust compilation. These local contract timings are not a claim about fleet latency; the final release gate owns beta canary and live publish-to-active propagation evidence.

The catalog path is compared with the measured native baselines in the approved plan: 525.8–595.1 seconds for Rust build/test and 721.2–774.9 seconds for a server release. A catalog-only publication runs schema/lint/evaluation/sign/publish checks and does not require a native Rust/Web release.

The final measured optimized flamel_server build completed in 418.17 seconds wall time, used 515.34 seconds user CPU, 40.30 seconds system CPU, and 2,753,512 KiB peak RSS. Its stripped binary changed from 25,737,112 bytes to 25,719,448 bytes (17,664 bytes smaller, about 0.069%); this confirms that binary size is not the catalog’s material benefit. The final post-change flamel_mcp build completed in 70.95 seconds wall time with 1,169,556 KiB peak RSS and remained exactly 6,670,896 bytes.
