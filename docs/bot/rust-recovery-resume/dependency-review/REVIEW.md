# V06 dependency evidence, 2026-10-05

> 最終更新: 2026-10-05(月) 02:33:00 UTC

## Result and scope

Current-lock source evidence is complete for the RustSec lookup and cached archive/source hashes. V06 remains **pending**, particularly for final clean-build binding, SQLite upstream-fix reachability, complete selected publisher history, and actual distribution/native payloads. No dependency, lockfile, application, or task Git ref was changed; no package was installed, built, published, or upgraded by this review. No application code or secrets were uploaded.

- Published source checkpoint: `15db189db643143191e8b8063165fc6f2f5d2b14`
- Recovered baseline: `7c2d7d320e04cd1d550d937d97515ff692500c6a`
- Fixed Go oracle: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`
- R/V supplement read: `b454720b631f7004bff8a0ca6fe76900ae64ecc1`
- Cargo.lock SHA256: `0466139df475d43177199440af0e9aa2455b26aabe55927cacb2d365f8cd0a12`, identical at the checkpoint and during review; 316 registry package/version entries.

The earlier 277-package advisory receipt has a different lock hash and cannot establish acceptance of this lock.

## Completed checks

1. All 316 registry entries were checked against [RustSec snapshot ef6173c](https://github.com/RustSec/advisory-db/tree/ef6173cbc5c50ec8166f9a5b28f07834144373ee), committed October 3, 2026. The snapshot contains 1,270 crate advisory documents. Ninety-three distinct advisories produced 100 package/version matches: 97 were in explicit patched/unaffected ranges and three were withdrawn. There was no active affected-range match. The receipt retains exact ranges, aliases, document hashes and immutable URLs. This was a fail-closed, stable-version SemVer comparison, not cargo-audit, a full exploitability assessment, or a guarantee against unknown vulnerabilities.
2. All 316 cached `.crate` SHA256s matched Cargo.lock. All 20,285 archived files matched the paired extracted sources. A separate archive-type/path check found only regular files, no links or invalid paths. Extra Cargo-generated/nonarchive cache files are outside that equality claim. A mutable cache receipt is not a clean-host attestation.
3. [CISA's official KEV mirror](https://github.com/cisagov/kev-data/tree/b244ed1a640323565afba92100d7308d51c6614e) was pinned to October 4 commit `b244ed1a640323565afba92100d7308d51c6614e`. Catalog `2026.10.04` contains 1,734 entries; JSON and CSV CVE sets agree. None of the 84 CVE aliases on matching RustSec records appears in that snapshot. A bounded SQLite/AWS-LC/rustls/glibc/zlib product-name check also found none. This is not a complete CPE mapping and absence from KEV is not safety evidence. Direct cisa.gov access returned 403, so the official mirror supplied the snapshot.
4. Exact crates.io version metadata was captured for 18 of 32 selected native, crypto, process, parser and build packages. All 18 registry checksums matched the lock and none was yanked. Nine selected source commits and current owner lists were retrieved separately. AWS-LC's selected publications are attributed to `justsmth`; SQLite's wrapper to `gwenn`; portable-pty to `wez`. libc 0.2.190 exposes trusted-publishing metadata identifying `rust-lang/libc`, run `37054821350`, and source SHA `7b0ab5528dc7f361f3a0b4c06c2cad9971617f75`, matching its crate VCS claim. These are attribution observations, not independent attestations or proof that no account takeover occurred. Recent-version-history API calls failed with HTTP 400; historical ownership continuity remains unverified.
5. Five cached Rust 1.90.0 Linux component archives matched the official HTTPS release manifest; the cached manifest itself matched the fresh upstream manifest and its published SHA256. The Bun 1.3.14 Linux ZIP matched the official GitHub release asset digest. Signature verification and other target toolchains were not performed.

## Native inputs and bounded reachability

### SQLite

`rusqlite 0.40.2` selects bundled `libsqlite3-sys 0.38.2`, containing SQLite 3.53.2. Its sqlite3.c SHA3-256 is `44fd61b9f93b4155105cb2d80c957ae6c64a8b5bd6ed51a4992f0dbd438e4e11`, exactly the [official release hash](https://www.sqlite.org/changes.html). SHA256 is `0a409f1633283fa31a9126b11fbfd64a1991c5d30defad07e5745d4667f5e23d`.

Upstream [3.53.3](https://www.sqlite.org/releaselog/3_53_3.html) and [3.53.4](https://www.sqlite.org/releaselog/3_53_4.html) include later fixes. The [official branch timeline](https://sqlite.org/src/timeline?from=version-3.53.0&to=version-3.53.4&to2=branch-3.53&y=ci) describes malformed database/WAL, FTS, JSONB and journal-processing corrections. The wrapper build enables FTS3/FTS5, JSON, RTree and STAT4. The application opens existing DB files in WAL mode and uses FTS5 MATCH/snippet through parameterized queries (`rust/src/storage/schema.rs`, `repository.rs` at the reviewed checkpoint). It does not expose arbitrary SQL through those search calls. No calls to ATTACH, FTS integrity-check, SQLite deserialize or backup APIs were found in the reviewed application source, despite the backup crate feature being selected.

That narrows some entry points but does not rule out crafted/corrupted on-disk database or sidecar reachability. No exploit or malformed-file reproducer was run. **Remaining gate:** owner-led assessment of relevant post-3.53.2 fixes against actual compile options, stored-data trust boundaries and copied-data regressions; any upgrade requires separate coordination. Do not mark V06/V08 complete from the wrapper's single RustSec advisory alone.

### TLS and cryptography

Current Linux normal/build selection uses reqwest → rustls 0.23.45 / webpki 0.103.15 → aws-lc-rs 1.18.1 / aws-lc-sys 0.45.0. [Rustls's September advisory](https://github.com/rustls/rustls/security/advisories/GHSA-2mjx-qc3c-rqvc) identifies 0.23.45 as patched. AWS's [March bulletin](https://aws.amazon.com/security/security-bulletins/2026-005-AWS/) and [CRL bulletin](https://aws.amazon.com/security/security-bulletins/2026-010-AWS/) have fixed aws-lc-sys thresholds below 0.45.0. The sys crate header identifies AWS-LC 5.7.0; its upstream repository submodule points to `02561621ffa4cf17c0c4f70bc11a82df36b42ae9`, matching the [official 5.7.0 release](https://github.com/aws/aws-lc/releases/tag/v5.7.0). Full vendor-file equality against the upstream Git tree remains unverified after a tunnel 403. Cached archive/source equality is verified.

TLS is an active production outbound network boundary; this review does not establish provider/account or deployment acceptance. Ring and QUIC are in Cargo.lock and metadata but absent from the current Linux normal/build cargo tree and inspected debug fingerprints. Their lock presence alone is not evidence they were built or linked.

### Process and build boundaries

Current Linux dependency selection contains libc 0.2.190, Tokio 1.53.1, portable-pty 0.9.0 and nix 0.28.0. Source callers use native PTYs, process spawning, signals and libc filesystem operations. Current ranges do not match active RustSec advisories; OS APIs, process-tree cancellation and native lifecycle behavior still require target receipts, including V01/V02. The libc crate is bindings, not proof of the deployed system libc/kernel patch level.

Cargo 1.90.0 predates fixes for [CVE-2026-33056](https://blog.rust-lang.org/2026/03/21/cve-2026-33056/), [CVE-2026-5222](https://blog.rust-lang.org/2026/05/25/cve-2026-5222/) and [CVE-2026-5223](https://blog.rust-lang.org/2026/05/25/cve-2026-5223/). These concern build-time extraction or alternate-registry credential handling. This lock uses crates.io exclusively and the verified offline archives have no symlinks; the documented attack prerequisites are not demonstrated for this reviewed path. Do not generalize that observation to future alternative registries, credential-bearing builders or publish jobs. The three CVEs are absent from this KEV snapshot. Keep V03's build/publish isolation gate and explicitly review any future registry/toolchain changes.

## Build evidence and remaining work

The current Linux normal/build cargo tree selects 237 registry packages. Existing debug fingerprints observe the selected native/TLS/process crates, with static SQLite and AWS-LC crypto link directives. They are mutable cache observations, not a successful clean two-binary release receipt. `cargo metadata` over-reports optional nodes for this purpose; the final packaging collector must use actual compiler/build-script observations.

Remaining gates: bind reviewed input hashes and features to final clean candidate/both binaries on all four targets; finish interrupted selected metadata/history and AWS-LC upstream-tree evidence; resolve SQLite post-release-fix triage; review actual native DLLs, models, frontend/system components, notices/SBOM and distribution hashes. No native payload authenticity, complete distribution SBOM, installation, provider, production or release acceptance is claimed.

## Evidence files

Public-ready receipts alongside this report: `advisory-receipt.json`, `archive-source-integrity.json`, `archive-members-safety.json`, `kev-receipt.json`, `selected-provenance.json`, `publisher-source-corroboration.json`, `toolchain-provenance.json`, `cargo-tree-selection-linux.json`, and `existing-linux-debug-build-observations.json`.

Raw Cargo metadata/tree outputs, local collection scripts, logs and downloaded upstream snapshots are working evidence, not part of the public-ready bundle. Do not publish raw local paths.

Selected metadata/history collection remains incomplete after a tool approval cancellation; the cancelled operation was not retried or rerouted. Completed saved receipts retain only their stated scope.
