# 4dc961ce native CI and artifact identities

> 最終更新: 2026-10-05(月) 22:55:24 UTC

Source and independent review SHA: `4dc961ce600d12aa0911e5a9620e50411ab13d9f`. Fixed Go oracle: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`. These are the observed artifacts of this immutable baseline, not hashes promised for a later documentation or repair commit.

Rust workflow run **23**, [37364997189](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37364997189), Rust 1.90.0. Linux, Windows and Intel macOS executed successfully in attempt 1. ARM executed successfully in attempt 2 after a zero-runner cancellation; attempt 2 retained the other three results. All targets passed formatting, strict Clippy, all-target tests, doctests, both release builds and packaging-input collection.

| Target | Library | All-target total | Doctests | Actual execution |
|---|---:|---:|---:|---|
| `x86_64-unknown-linux-gnu` | 1047 | 1354 | 3 | attempt 1, job 111947918023 |
| `x86_64-pc-windows-msvc` | 987 | 1266 | 3 | attempt 1, job 111947918548 |
| `x86_64-apple-darwin` | 1047 | 1353 | 3 | attempt 1, job 111947918509 |
| `aarch64-apple-darwin` | 1047 | 1353 | 3 | attempt 2, job 111965372489 |

All-target totals include library tests; doctests are a separate command. Every shown test summary has zero failed and zero ignored. Historical 1253 and 1149 counts are separately scoped receipts, not substitutes for these results.

## Artifacts and both binaries

### x86_64-unknown-linux-gnu

- Artifact name: `rust-candidate-x86_64-unknown-linux-gnu-4dc961ce600d12aa0911e5a9620e50411ab13d9f`
- [Artifact 11368467629](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37364997189/artifacts/11368467629); 21,025,194 ZIP bytes; expires 2026-10-12T19:58:13Z
- ZIP SHA-256: `db08a64cc19d5c37cde37d8828a7e5a0ab89a031b26ffe16b259c1634cfc306b`

| Binary | Bytes | SHA-256 |
|---|---:|---|
| `many-ai-cli` | 48,239,488 | `69c5ec56db6c3b5b934b3692920af098aaded79637897f0a9e88ac3a2070b9d0` |
| `many-ai-cli-launcher` | 14,040,392 | `8a6397b15e1516ca66a680dcfa4d73f640af9a31bb1ae30e37bfb4f311b900be` |

### x86_64-pc-windows-msvc

- Artifact name: `rust-candidate-x86_64-pc-windows-msvc-4dc961ce600d12aa0911e5a9620e50411ab13d9f`
- [Artifact 11369690033](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37364997189/artifacts/11369690033); 19,196,549 ZIP bytes; expires 2026-10-12T20:18:05Z
- ZIP SHA-256: `2f40d41fd319a38f11461355c0aa2e891e2298939f067887ffec55830e5eed31`

| Binary | Bytes | SHA-256 |
|---|---:|---|
| `many-ai-cli.exe` | 43,334,656 | `47656ff16d5230b2d61a33b31855fc0ed8e074db60bbe656ba83e3a1b9b08c57` |
| `many-ai-cli-launcher.exe` | 10,388,992 | `715009ee0006e5c0b2007e8845fa52d213faa68cf427e821f326958a80393665` |

### x86_64-apple-darwin

- Artifact name: `rust-candidate-x86_64-apple-darwin-4dc961ce600d12aa0911e5a9620e50411ab13d9f`
- [Artifact 11369281987](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37364997189/artifacts/11369281987); 19,892,356 ZIP bytes; expires 2026-10-12T20:27:11Z
- ZIP SHA-256: `3a5cfbeac01cded26ca0fe1c8ab4388196e29838b954a1b004ed16eba230b98b`

| Binary | Bytes | SHA-256 |
|---|---:|---|
| `many-ai-cli` | 43,368,064 | `83475c1dff9169dd26065cebd50af7764e15558e86f62309589fc00c22bb32e7` |
| `many-ai-cli-launcher` | 11,269,800 | `4c9b27162bdbbc6b6c5a9530c9949716b3ad72519a488b68b3119cac30c8a265` |

### aarch64-apple-darwin

- Artifact name: `rust-candidate-aarch64-apple-darwin-4dc961ce600d12aa0911e5a9620e50411ab13d9f`
- [Artifact 11370592750](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37364997189/artifacts/11370592750); 19,364,384 ZIP bytes; expires 2026-10-12T20:54:22Z
- ZIP SHA-256: `2e38292afb2e115ffaa17a091b2f574617444a2b08ad9a6e91595ca1c3977abd`

| Binary | Bytes | SHA-256 |
|---|---:|---|
| `many-ai-cli` | 40,594,368 | `47697f6a3c6743cc7529b139a99e7ddaf89ab62e080cfc6913be856605d77b32` |
| `many-ai-cli-launcher` | 10,124,896 | `b74427844d53869950dd719973d7de41c092d74df9b952d6f7e1843f654139e8` |

ZIP CRC, published ZIP digest, both binary sizes/digests, and BUILD-RECEIPT source/review SHA were independently verified for all four targets. The build receipt preserves its native-build-only acceptance limitations. No binary was executed as part of the artifact hash review.

## Lockfile, checks and acceptance limits

Windows checkout Cargo.lock uses CRLF: `90d6d2ebdf259ae214308ded77ac4156fffec2697a62fca16f1a60f06307d2ed`. Its exact LF-normalized bytes match the committed/Linux/macOS digest `0466139df475d43177199440af0e9aa2455b26aabe55927cacb2d365f8cd0a12` (3094 lines). Raw lockfile bytes differ; dependency content is unchanged.

Validate and PR secret-scan checked out synthetic merge `296ffb5063c0f6018733651ac208f4bb2986b12b`; its tree `999ebd19270bda47e61cb377034a07f6830aa2a7` equals the source head tree. Commit IDs remain distinct. Rust explicitly checked out the source head.

The [machine-readable receipt](VALIDATION-4dc961ce.json) records each attempt, both final scan ranges, the six intervening single-commit push scans, and full 38-commit PR coverage by union. The PR action itself still returns only the first 30 commits. Existing approved fingerprints were retained; no exclusion was added. Other historical findings/private-watchlist coverage are not accepted by this union.

Green native CI and source-composition counts do not establish real-provider/device/installed-data acceptance, a filled Windows whisper runtime payload, signed distribution, rollback rehearsal or cutover. PR #11 remains Draft.
