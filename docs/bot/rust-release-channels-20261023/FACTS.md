# Facts for P-20261023-002

> Last updated: 2026-10-23(金) 02:36:51 JST

Verified against product source 332725ef on 2026-10-23.

- README.md and README.ja.md preserve winget identifier
  ishizakahiroshi.many-ai-cli and Homebrew cask ishizakahiroshi/tap/many-ai-cli.
- rust/src/launcher/delivery.rs defines four native target suffixes. ZIP carries
  both binaries, npm carries the main binary, deb/rpm carry both Linux binaries.
- scripts/package-rust-candidates.py already validates four native receipts,
  flat binary entries and SHA256SUMS.txt. Candidate-only restrictions are being
  retained while a separate explicit --release-tag mode is added locally.
- The local native owner preserves file names many-ai-cli-VERSION-SUFFIX.zip,
  adds tag identity before compilation and canonical THIRD_PARTY_NOTICES.md,
  and keeps build-input evidence under packaging-inputs/ in the ZIP.
- The existing release.yml is not connected to Rust yet. It is owned by the
  local integration owner, which will consume the adapter outputs. Do not use
  its old GoReleaser paths as a working Rust implementation.
- Official Docker images, container build and automatic updates are outside
  v1.0.0 support. GitHub Docker publishing has been disabled. This task does not
  restore Docker, remove historical images, or change Linux/SSH support scope.
- Synthetic offline verification, CI success, generated package inputs, actual
  package installation and public release acceptance are distinct evidence.
  The helper must never label generated metadata as published or installed.

No private conversation, local installation path or credentials are required.
