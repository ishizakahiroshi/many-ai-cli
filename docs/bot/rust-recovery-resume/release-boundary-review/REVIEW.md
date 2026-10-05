# V03: release dependency persistence and immutable artifact transfer

> 最終更新: 2026-10-05(月) 03:19:03 UTC

Review date: 2026-10-05 UTC. Result: **source/workflow evaluation complete; production isolation and signed native artifact acceptance remain pending.** No release, real signing, credential access, network request, dependency installation, source edit or Git mutation was performed.

## Scope and exact source identity

Read-only source: the preserved repository checkout. Observed HEAD: `15db189db643143191e8b8063165fc6f2f5d2b14`; the worktree was dirty with concurrent continuation edits. This is **not a final-SHA review of those edits**. SOURCE-INPUTS.json identifies 32 selected working-source inputs by individual hashes. Source references below use repository-relative paths and line numbers at that snapshot. The source manifest SHA256 is `c17c9e130f18e1f5902c610c90c3c6122d078987a8250be71e944743f19db47e`.

Read AGENTS.md, CLAUDE.md, recovery README.md, REVIEW.md, RV-CONTRACTS.md and BEHAVIOR-MATRIX.md V03. This work evaluates the mandatory K15/A12 release boundary without duplicating general behavioral, V06 advisory, licensing or native runtime acceptance reviews. The fixed Go oracle is `21d0bc7935a2c4696fb89ccff2e324157a528c2d`. Source claims below are workflow/configuration observations and engineering inferences, not a live GitHub permissions audit.

Selected reviewed hashes:

- `.github/workflows/release.yml`: `fbbe2e18663432ad159f4ddee73b086be7c5a318c8c12bef0d6364e4a04aed3a`
- `.github/workflows/rust-migration.yml`: `20083138ae0c2565108b0248c48828a5abc98ecf419f7a199672f0b56f82a182`
- `scripts/rust-candidate-ci.py`: `d46a9c3a4d42b7b6ea0485d0d5ddf04bf5db8b9467703bf6fb04b38394fe90ce`
- `.goreleaser.yaml`: `d9aa11b6a41e435c4b5c74dc03e70deb8b896312041da17499e6b986a387bbeb`
- `rust/Cargo.lock`: `0466139df475d43177199440af0e9aa2455b26aabe55927cacb2d365f8cd0a12`

## Findings

### V03-F1 — High: step-scoped secrets do not isolate the release runner

`release.yml:26–28,69–87,195–219,224–268,345–349` places dependency preparation and credential-bearing publication in the same `release` job. The explicit PAT/NPM secrets are step-local, which reduces immediate environment exposure, but dependencies can leave files, modified executables/scripts/configuration, PATH entries or processes in that job's shared workspace/home/temp. A later credential-bearing step can execute or read that persisted state. The workflow does not discard that execution environment before publication.

Concrete execution reaches:

- `bun install --frozen-lockfile` and `bun run build` run in the publisher's workspace. Even if lifecycle scripts are blocked, `web/scripts/build.mjs:5` imports and executes esbuild and its supporting toolchain.
- `go install github.com/tc-hib/go-winres@v0.3.3`, followed by both `go-winres make` calls, executes a downloaded third-party tool (`release.yml:214–219`). Ordinary compilation of a Go dependency is not itself evidence that its package runtime executes; the explicit tool invocations are sufficient for the boundary finding.
- `goreleaser release --clean --skip=before` is still the normal build/archive/SBOM/sign/publish pipeline. `.goreleaser.yaml:34–80` defines both Go builds. Skipping only before-hooks does not convert it into a publish-only consumer. GoReleaser, Go tooling, syft and cosign run in the same later process tree/environment or job with publish authority.
- The verified GoReleaser installer is a worthwhile integrity control, but it runs **after** dependency execution and uses the same mutable runner/PATH/cosign. Verification on a potentially modified host is not independent isolation.
- `release.yml:26–28` grants contents-write and OIDC capability at workflow level; the release job does not narrow them. Its checkout omits `persist-credentials:false`. Under the standard action defaults, repository authentication persists and is not restricted to the two explicit publication environment blocks. OIDC permission likewise is not a guarantee of absence from earlier dependency steps. Actual token audiences, repository policies and runtime exposure were not inspected.
- Go build caching is enabled (`release.yml:106–109`), expanding persisted state to be considered. Cached build/dependency state must never be restored into a future signer/publisher simply because the build job used it.

This establishes a possible credential-reach path, not evidence of an actual malicious dependency or compromised token. Removing environment variables or deleting node_modules later cannot reliably revoke earlier arbitrary code's persistence.

### V03-F2 — High: npm credential step consumes mutable directories and insufficient retry identity

`release.yml:332–349` stages and rewrites npm package directories, then `scripts/publish-npm.sh:35` publishes those directories with `NODE_AUTH_TOKEN`. It neither consumes prebuilt immutable tarballs nor explicitly disables lifecycle scripts. The five current package manifests have no lifecycle scripts; there is no claim that the checked-in manifests currently exfiltrate credentials. The weakness is that earlier code can modify those manifests/scripts/files before the token-bearing invocation.

Normal staging (`scripts/stage-npm-binaries.mjs:33–68`) copies loose binaries selected from `dist/artifacts.json`; it does not independently compare the copied bytes with the release ZIP or final npm tarball. The source's intended same-build identity is reasonable under an honest unmodified runner, but is not a hostile-runner custody check.

The npm-only path is stronger: `release.yml:320–330` verifies the checksum signature with an exact expected workflow/tag identity and issuer; `scripts/stage-npm-from-release.mjs:93–126` checks ZIP SHA256 then extracts the main binary. Preserve that chain. The extraction uses general `unzip`, then checks existence, rather than a narrowly verified regular-member inventory; moving it into a credentialed clean job without parser/path/type limits would carry avoidable risk.

`publish-npm.sh:40–43,63–69` treats version conflicts as successful skips and only prints registry version visibility. Version existence does not prove that already-published bytes equal the requested artifact. The later real publication owner must compare registry tarball integrity and unpacked main bytes, or stop on mismatched existing versions. No registry query was performed here.

The npm-only workflow intentionally permits current branch scripts/shim with historical tagged binary bytes (`release.yml:80–83`). Do not silently relabel the complete npm package as originating entirely at the old tag: record **binary source SHA and wrapper/package source SHA separately**, plus the final tarball digest, or intentionally change that contract through the integration owner.

### V03-F3 — Medium: artifact gates and signature identity need placement/trigger review

`release.yml:259–272` publishes before running `check-artifact-clean.mjs`. The earlier instrumentation source check is useful but does not make the later artifact check a pre-publication gate. Check actual final unsigned contents before authorizing signing/publication.

`.goreleaser.yaml:104–124` signs `SHA256SUMS.txt` with cosign. Preservation means transporting the exact checksum bytes, signature and certificate/bundle without regeneration, verifying the signature and identity, then verifying every referenced payload digest. A SHA beside an artifact, a matching tag string or an inert certificate file is insufficient authentication.

Also test both release triggers explicitly. A full workflow_dispatch checks out the requested tag, while the OIDC workflow identity is governed by the workflow invocation, not merely the checked-out source. The npm-only verifier hardcodes a tag-ref certificate identity. Whether a dispatch-created release will meet that identity must be established with actual authorized signing evidence; this review did not request a certificate. A new signer job/workflow must retain the intended identity contract or explicitly update and test consumer policy, never simply accept arbitrary identities.

### V03-F4 — Required gate: Rust candidate CI is build evidence, not an isolated release chain

`rust-migration.yml:15–16,47–50` correctly uses contents-read and non-persistent checkout credentials. It has four native matrix targets and no publisher. `rust-candidate-ci.py:87–118` requires an initially clean checkout and binds revision/toolchains/version; `124–171` runs dependency resolution, frontend, tests, Cargo release bins, metadata and observed-build input collection. `143–165` hashes/readbacks both binaries and notice copies. This is a good base for an unprivileged producer.

Do not promote its current artifact directly into a release:

- Upload runs with `always()` and can retain partial/failing receipts (`rust-migration.yml:77–85`). Names, directory presence and an uploaded receipt do not prove all producer gates succeeded. Require exact run/attempt/job conclusion and a complete per-target manifest.
- It emits `native-build-only-not-accepted-for-cutover`, a candidate version and an independently explicit pending list (`rust-candidate-ci.py:173–188`). A release build with different embedded version/time is different bytes. Freeze release metadata before the unprivileged build; do not rebuild or patch embedded metadata inside the signer/publisher and call it the same artifact.
- An initial clean Git status plus a HEAD field is traceability under an honest builder, not hostile-host attestation. Build scripts/proc macros may execute and mutable source/receipts can be rewritten. Record source/input digests in a trusted orchestration/provenance context; a compromised authorized builder can still emit malicious but consistently hashed output.
- `collect_inputs.py:200–247,264–313` selects actual successful Cargo observations, verifies locked source/license input evidence and explicitly keeps `release_ready:false`. Its SPDX excludes frontend, Go-derived assets, toolchain/system libraries and downloaded native payloads. It is not the complete distribution manifest or a signature.

## Actual distribution inputs to preserve

The boundary change must preserve the existing package contract, not create another release owner:

- `.goreleaser.yaml:34–102,184–194` and `rust/src/launcher/delivery.rs:13–118`: four ZIP target archives, each with main and launcher; Linux deb/rpm carry both; Homebrew casks refer to macOS archives; winget refers to Windows archive. Keep existing target suffixes and filenames.
- `npm/*/package.json`, stage scripts and `delivery.rs:79–85`: four platform npm packages carry **main only**; root shim package pins platform optional dependencies to the exact version. Do not add the launcher to npm during isolation work.
- Freeze notices/licenses and final per-channel inclusion before packaging. Existing ZIP inputs are Go-era root notices and Web vendor texts; Rust Cargo input notices must be composed through the existing packaging review, not assumed present in every channel.
- `rust/build.rs:30–82` embeds generated `web/dist`, launcher HTML and version/commit/time. Preserve their hashes and inspect actual output retention before package acceptance.
- Windows runtime preparation already occupies a separate read-only job (`release.yml:30–67`); `fetch_windows_runtime.ps1:48–62,111–141` verifies VS-redist source DLLs, Microsoft Authenticode and x64 PE with no default System32 fallback. The transfer uploads DLLs without a dedicated hash/provenance receipt and receiver checks only filenames/count (`release.yml:161–176`). Extend the immutable input manifest to the four exact DLL bytes, version, signer/provenance and redistribution evidence. Rust currently has `runtime_payload: Vec::new()` (`serve.rs:1059–1062`); do not label empty payload as native packaged acceptance.
- PE resources, native voice/runtime/device tests, deb/rpm contents, notices/SBOM completeness and four-target executable acceptance stay with their respective existing owners. This report did not generate or accept any real package.

## Proposed immutable transfer architecture

This is a design for owner review, **not an applied workflow change**.

1. **Freeze the trusted invocation.** Record repository, immutable candidate/source SHA, tag-to-commit resolution (tag object too if applicable), exact workflow revision, run ID, run attempt, expected target set, release version, build metadata, locks/assets/native input digests and reviewed packaging tools. Reject PR/fork artifacts, unrelated successful runs, stale attempts and moved tag/source mismatches. Use a trusted control-plane artifact ID/digest and provenance decision; do not take a self-declared digest from the same untrusted payload as its trust anchor.
2. **Build natively without signing/publish authority.** Four clean ephemeral producers with contents-read, non-persistent checkout, no OIDC-write, no publish secrets, no publish protected environment and no privileged reusable workflow access. Cargo build scripts, proc macros, frontend/tool/package code stay here. No producer-controlled workspace, HOME, PATH, cache, tools or background process may survive into signer/publisher. Fresh directories or containers sharing a user/host are not automatically sufficient isolation.
3. **Package and inspect before credentials.** Assemble immutable ZIPs, Linux deb/rpm, the five packed npm tarballs, complete notices and distribution SBOM using existing owners. Disable lifecycle execution when packing; run needed application/native smoke only in unprivileged jobs. Inspect uncompressed contents/types/size/path bounds, exactly eight native binary roles, npm main-only roles, root shim/version/dependencies, resources and licenses. Compare native binary hashes across all consumer formats. Keep supported architecture and ABI results separate.
4. **Finalize the content manifest.** Canonical release manifest binds all inputs, producer identities, final archive/tarball hashes/sizes, member binary digests, all four success receipts and explicit validation outcomes. Finalize checksum-file bytes before signing. Include a manifest digest in the checksummed/signature-protected release data with a non-circular layout. Store immutable artifacts by service-provided identity, not a broad name search or “latest successful” match. Never transfer caches, repository scripts, node_modules, Cargo home, compiler tools or arbitrary build workspace.
5. **Independent clean verification and signing.** A separately provisioned fresh runner uses pinned, trusted verifier/signing tooling obtained independently of the builder. Treat downloads as data only, reject extra/missing/duplicate/path-escaping/symlink or special members, cap expanded sizes, rehash every final payload and verify the control-plane identity/provenance. No candidate execution, build, dependency install, npm lifecycle, archive-supplied config or downloaded script execution here. Only the signing job gets narrowly scoped OIDC, and only after the verification gate. Keyless signing is credential use and remains unauthorized in this task. Keep exact signature/certificate/bundle bytes in a second immutable signer output; reverify them independently.
6. **Minimal publisher and channel-specific authority.** A separate clean publisher verifies signer identity, signed manifest/checksum bytes and every payload digest before any publish credential is exposed. Publish already-packed npm tarballs with lifecycle scripts explicitly disabled; test the pinned npm version's behavior before deployment. Upload exact ZIP/deb/rpm/SBOM/signature bytes without rebuilding, rearchiving, modifying manifests or rerunning GoReleaser's build pipeline. A GoReleaser split/resume mode must be checked against its pinned official contract before selecting it; no unverified flag recipe is proposed here. Separate npm, repository release and Homebrew/winget credentials/jobs where feasible, with smallest permissions and distinct environment approval. Persistent permission/security changes require maintainer approval.
7. **Channel identity and retry verification.** Homebrew/winget manifests contain approved immutable URLs and exact signed archive hashes. Unpack npm/deb/rpm/ZIP consumer downloads in unprivileged verification to establish byte equivalence after actual authorized publication. On conflict/retry require already-published digest equality; never accept “version exists” as artifact identity. Preserve separate binary and wrapper sources for the npm-only contract. A mismatch blocks that channel and must not trigger an unapproved overwrite/rebuild.

OS executable signing/notarization, if later required, changes binary bytes. Do it in an authorized dedicated signing stage **before** final package hashes and channel equivalence checks. It is distinct from the existing cosign checksum signature and must not be implied by it.

## Credential-free demonstration actually executed

Fixture: `handoff_fixture.py`. SHA256 `4241239468c9178448104607ce7d5a8ba822eb452e29fe2409fe04cd35e6acbf`.

Equivalent standalone command (from the repository root; output goes to an explicitly selected temporary directory):

`PYTHONDONTWRITEBYTECODE=1 python3 docs/bot/rust-recovery-resume/release-boundary-review/handoff_fixture.py --output <SELECTED-TEMP-DIRECTORY>`

Exit 0. `DEMO-RECEIPT.json` reports **22 cases, zero unexpected failures**: 18 expected negative-case rejections, one positive four-target package identity case, two persistence/transfer models, and one explicitly accepted compromised-producer residual risk. The receipt is authoritative for the exact case list. Receipt SHA256: `61f6e405cb9c95abfc7ed2c75abb47ad29ee04fe651d97a5d973b4f74210a6f6`.

The fixture creates inert synthetic bytes for four two-role ZIPs and five npm tarballs, checks main-byte identity across ZIP/npm, transfers only allowlisted artifacts, and rejects changed archive/checksum/signature-marker bytes, manifest rewriting without the independent pin, wrong source/tag/workflow/run/attempt, missing target, unexpected dependency helper, npm consumer byte drift, unsafe path/link and excessive expanded data. It reads archive members into bounded memory without extracting or executing them.

A separate public-canary subprocess shows that a helper file left in the same directory by an earlier stage can read a value supplied only in a later stage. It uses an intentionally empty inherited environment plus `V03_PUBLIC_CANARY`, a plainly public non-secret string. No real token, credential store or provider was inspected. The separate-directory allowlist model excludes that helper.

**Strict limits:** directories still share the current host and UID. This is not a VM/runner isolation test, permission audit or evidence that residual builder processes cannot reach a signer. `.sig.fixture` and `.pem.fixture` are deliberately labeled **NOT A SIGNATURE / NOT A CERTIFICATE**: the test preserves and binds their exact bytes, but performs **no cryptographic signing or signature verification**. It does not use the GitHub artifact service, a native build, real npm pack/publish, deb/rpm tooling, network or real consumer installation. The fully self-consistent compromised producer case intentionally passes; hashes and signatures establish custody/identity, not software benignness or reproducible construction.

## Required next evidence / ownership

- Integration/release-security owner: apply a reviewed isolation proposal only within approved paths; freeze the final SHA and re-review workflow/tool diffs. Production release.yml changes require explicit ownership coordination. This review edited no shared source.
- CI owner: no-publish run on actual separate ephemeral runners at that final SHA. Use only non-secret canaries and inspect planned token/OIDC permissions without requesting tokens. Demonstrate builder-created executable/config/path/process/cache state cannot enter or influence the independent verifier/publisher runner. Return exact jobs, runner identities and immutable service artifact IDs/digests.
- Packaging/native owners: complete four-target same-source real binary builds, runtime/resources, final package inventories, notices/SBOM and consumer-byte comparison. All must be prior to any publication decision.
- Authorized signing owner: separately approved signature issuance/verification with exact checksum/manifest bytes, expected issuer and trigger-specific workflow/ref identity, and transport/readback identity. This task supplies no such approval and no actual signature receipt.
- Independent reviewer: review those exact final source/artifact/run identities and negative cases. V03 remains **required and pending clean isolated native-artifact/signature evidence**. Production publication/cutover and real credential use remain unauthorized.
