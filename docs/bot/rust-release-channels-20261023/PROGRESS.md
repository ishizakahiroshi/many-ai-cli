# P-20261023-002 many-ai-cli: release channel adapter

> Last updated: 2026-10-23(金) 03:07:52 JST

Product baseline: 332725efeed223489553ce64f015af59ce5f9dd0.
Instructions: docs/rust-release-channels-20261023.
Implementation: dots/P-20261023-002-rust-release-channels; Draft PR base develop.
Only the local integration owner updates this file. The management Issue is the
status source: https://github.com/ishizakahiroshi/many-ai-cli/issues/33
Initial instruction publication / review diff base:
79a69141446176a11ef46314cf309e222b56add2. Instruction publication,
delivery, acceptance, submitted code SHA, reviewed SHA, CI and native acceptance
are separate observations. Instruction commits are not implementation acceptance.

| Work | Owner | Evidence / remaining conditions |
|---|---|---|
| Channel adapter and synthetic tests | dots | Submitted and locally reviewed at b0d76039d6ebb340aa55ef4a95dd4c993c04d8fb in Draft PR #34; RC-01/RC-02 corrected and independently rechecked |
| Native stamp and ZIP identity | local AI | Integrated after independent review; actual Windows icon resources replace raw asset search; formal product builds remain unaccepted |
| npm content identity and notices | local AI | Integrated after independent review; ancestor-link protection and exact published-tarball integrity checks included |
| Workflow integration and independent review | local conductor | Integrated; 140 packaging tests, 41 npm/version tests and one actual assembler-to-channel/npm integration pass locally; develop commit and normal CI results are recorded in Issue #33 |
| Native packages/install/update/rollback | release acceptance | Not established by synthetic tests |

The source release preflight currently blocks on four existing active
investigation entries. Full release checks this before native compilation;
source integration and normal CI do not establish release approval. Docker
publishing remains disabled and outside v1.0.0 support.

No application build, CI dispatch, package publication, tag creation, main merge
or Docker re-enable is authorized to dots by this assignment.
