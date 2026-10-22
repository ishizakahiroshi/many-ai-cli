# Independent review for P-20261023-002

> Last updated: 2026-10-23(金) 02:36:51 JST

Use the final fixed instruction commit as diff base and identify the exact
implementation SHA. The reviewer is separate from the implementer. The local
owner additionally reviews the entire submitted diff before integration.

Inspect the real CLI caller from archives through metadata and Linux payloads.
Refute version substitution, mixed targets, duplicate checksum/member handling,
archive traversal/symlinks, shell/YAML/Ruby injection, partial output and archive
hash mismatch. Confirm fail-before-output using an existing sentinel directory.
Confirm release URLs and embedded hashes refer to the same verified ZIPs and
both Linux binaries match their original ZIP bytes.

Validate cask and winget identities and both macOS architectures. Check the
winget schema against official sources and keep prereleases out of stable
channel updates. A build-input SPDX file is not a complete shipped-distribution
SBOM or license-clearance assertion. Record metadata preparation, package build,
install/update/rollback and actual publication as separate proof states.

Required findings include severity, exact caller, reproducible synthetic input,
expected/actual behavior and a meaningful test. Re-review fixes at the new SHA.
Do not change the instruction documents, PROGRESS.md or files owned by others.
Separate review reports belong to the permitted Drive submission; summarize the
code SHA, findings and verification in the management Issue and request thread.
