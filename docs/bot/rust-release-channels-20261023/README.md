# P-20261023-002 many-ai-cli: Rust release channel adapters

> Last updated: 2026-10-23(金) 02:38:35 JST

The owner approved the remaining native Rust release wiring on 2026-10-23.
Read FACTS.md, REVIEW.md and PROGRESS.md in this directory first.
Management Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/33
The initial instruction publication (review diff base) is
79a69141446176a11ef46314cf309e222b56add2; use the final instruction revision
supplied in the request for the implementation branch start and task contract.

## Objective and ownership

Implement an offline adapter that prepares Linux deb/rpm packaging inputs,
the existing Homebrew cask, and the existing winget portable ZIP manifests from
the same verified four native release ZIPs. Do not build any application binary.
Work repository: ishizakahiroshi/many-ai-cli. Product source baseline:
332725efeed223489553ce64f015af59ce5f9dd0. PR base: develop.
Instruction branch: docs/rust-release-channels-20261023.
Implementation branch: dots/P-20261023-002-rust-release-channels, starting from
the final instruction commit supplied in the request. Only these new paths are
owned by this assignment:

- scripts/prepare-rust-release-channels.py
- rust/packaging/test_release_channels.py

The local integration owner exclusively edits release.yml and version wiring.
Other local implementers own native receipt/ZIP packaging and npm scripts.
Do not edit their files, existing workflow files, npm, Go, Rust product code,
the instruction documents, generated indexes or PROGRESS.md.
If source contradicts these facts, report the exact discrepancy and use source
as evidence rather than silently copying an incorrect assumption.

## CLI and immutable input contract

`python scripts/prepare-rust-release-channels.py --archives DIR --tag v1.0.0 --repo owner/repo --output NEWDIR`

Python 3.12, standard library only. No dependency installation or network calls.
The caller verifies the checksum signature before invoking this helper on a
published release. The helper must independently verify all archive bytes
against SHA256SUMS.txt, without claiming to verify the signature itself.
Require exactly one matching ZIP per suffix: windows-x64, linux-x64,
macos-intel, macos-apple-silicon. Formal archive names are
many-ai-cli-VERSION-SUFFIX.zip, where VERSION is the strict semantic version
from the tag, with no leading v. Candidate versions, mismatched versions,
duplicate targets/checksum records, missing/empty binaries, unsafe names,
symlinks and duplicate ZIP members must fail before creating the output.
Validate repo as an owner/name identifier and tag/version before emitting any
URL, YAML, Ruby or RPM field. Reject injection and unsupported prerelease
channel semantics explicitly; never silently publish a stable channel for an RC.

Each ZIP is flat at root for the two binaries (Windows has .exe), LICENSE and
THIRD_PARTY_NOTICES.md. The historical THIRD-PARTY-NOTICES.md alias may coexist.
It also contains packaging-inputs/BUILD-INPUTS.json,
packaging-inputs/CRATE-NOTICES.txt and packaging-inputs/build-inputs.spdx.json.
These are build-input evidence, not proof of complete distribution clearance.
Do not execute extracted binaries or copy arbitrary members from the archive.
Preflight all four archives and expected selected members before any output.

## Required outputs

Create a new output directory only after validation, with deterministic files:

- deb-root/DEBIAN/control and deb-root/usr/bin/{many-ai-cli,many-ai-cli-launcher}
  for the Linux x64 bytes. Package name many-ai-cli, Architecture amd64, MIT,
  both binaries executable, and license/notices/build-input evidence under
  usr/share/doc/many-ai-cli. No service activation, post-install scripts or data
  migration. The integration job invokes dpkg-deb --build --root-owner-group.
- rpm/SOURCES/ containing the same selected Linux bytes and documents, and
  rpm/SPECS/many-ai-cli.spec, with x86_64 architecture, MIT license and both
  binaries in /usr/bin. No compile step, service activation or install hooks.
  RPM metadata and all paths must be safe for rpmbuild; the integration job
  supplies an isolated _topdir and _sourcedir. Describe the exact invocation
  contract in the acceptance reply and final PR description.
- homebrew/many-ai-cli.rb: a cask named many-ai-cli for the existing tap
  ishizakahiroshi/tap. Preserve macOS Intel and Apple Silicon suffixes and
  install both binaries. Use each canonical GitHub Release ZIP URL and its
  verified archive SHA256. No secondary build or local payload download.
- winget/ with valid multi-file YAML for PackageIdentifier
  ishizakahiroshi.many-ai-cli, x64 Windows ZIP, NestedInstallerType portable,
  the two existing executable/command names, canonical URL and SHA256. Use
  the current official microsoft/winget-pkgs schema; cite it in the PR.
- channel-inputs.json: tag, version, repository, verified ZIP names/hashes and
  selected Linux payload hashes plus relative output paths. No local absolute
  paths, credentials or application data. This records preparation only.

For prerelease tags, validate the ZIPs but do not emit stable Homebrew/winget
updates; record the explicit skipped channels and reason in channel-inputs.json.
Deb/rpm versions must follow their documented packaging syntax or be explicitly
rejected rather than silently rewritten. Stable v1.0.0 is the acceptance case.

## Verification and submission

Use synthetic ZIPs/binaries only in tests. Test the complete CLI caller path,
correct platform payload identity, metadata URL/hash identity, both binaries,
safe notice selection and all preflight failures. Keep prior output unchanged
on failure. Add at least one test that fails against the baseline without the
new implementation. Bound negative probes at 60 seconds and retain raw errors.
Run `python -m unittest discover -s rust/packaging -p test_release_channels.py -v`
and `git diff --check` if the tools are available. Report commands, exit codes,
counts and unexecuted checks separately; no application builds or execution.

GitHub operations must use ishizakahiroshi-bot, never the owner's account.
If the bot cannot perform an operation, stop that operation and report it.
Permitted: reading this repository, creating blobs/trees/commits and updating
only the assigned implementation branch, a single Draft PR targeting develop,
fix commits on that branch, and comments on this assignment's management Issue.
Each code commit ends with Agent: dot. Keep changes in one final implementation
commit per review round. Do not push develop/main, merge, publish, release,
create tags, dispatch/rerun CI, use secrets, contact production, change Docker,
install new dependencies, modify other repositories, or create other Issues.

Code is submitted as the Draft PR. Separate non-code reports, if any, go only
to Drive bot/blob/2026/10/P-20261023-002/v1/ and all subdirectories, with a final
manifest.json listing relative paths, bytes and SHA256 after read-back.
Use v2 for revisions; do not overwrite any existing submission version.
GitHub code-save cancellation may be retried at most twice after checking the
branch head and existing PR. If the third attempt is cancelled, provide the base
SHA and format-patch in that Drive version. If an operation is rejected, stop
it and report the reason; do not switch routes to bypass a rejection.

Reply only in this request's thread. Every report starts
`#P-20261023-002 many-ai-cli: ...`. Report acceptance, questions, stop and delivery
to the management Issue too; if Issue comments are unavailable, report that in
the thread for the local owner to transcribe. In acceptance report instruction
commit read, base commit, bot identity, number collisions, tool availability,
implementation/review separation and any mismatch. Continue through the allowed
scope without waiting for a second approval. Questions must describe a safe
default and must not stop independent work. Final delivery gives PR URL, head
SHA, bot authorship, tests, raw failure evidence and unmet acceptance conditions.
