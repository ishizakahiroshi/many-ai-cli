# #3 many-ai-cli rust: C5 integration, PR and independent review

> 最終更新: 2026-10-03(土) 12:44:31

## Prerequisites and ownership

C1 APIs are fixed; C2/C3/C4 have implementation and focused test receipts. The parent/integration owner edits shared Cargo/lib/bin/router wiring, .github/workflows/, packaging scripts/manifests, README and CHANGELOG. Other lanes return changes to those files as requests. Go and Web behavior remain the fixed-SHA oracle.

## Integration procedure

1. Compare all changed paths and deleted lines with every instruction, not only the implementer's report. Enumerate all CLI entries, registered HTTP/WS routes, protocol types, schema operations, provider integrations and channel artifacts. Each K01-K15 must map to a Rust caller and test/acceptance evidence. A success stub, omitted route, fallback to Go, ignored test or TS workaround is unresolved.
2. Integrate lifecycle cancellation, storage queue drain/generation reset, approval source switching/epoch, wrapper reconnect/input ACK, Files/memo mention scope, routines and notification events. Repeat tests at caller boundaries where unit success cannot prove real wiring.
3. Generate Web only in the isolated checkout. With the repo's locked Bun dependency workflow, build the existing TS bundle, then build both Rust binaries. Record exact versions and actual outputs. Do not rebuild/restart the daily Go Hub.
4. Run cargo fmt --manifest-path rust/Cargo.toml -- --check; cargo clippy --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings; cargo test --locked --manifest-path rust/Cargo.toml; cargo build --locked --manifest-path rust/Cargo.toml --release --bins. Use targeted tests and a Go baseline comparison for any failure; do not weaken timing/security tests to obtain green.
5. Check existing source/static guards where applicable and extend their language reach to Rust behavior without replacing behavior tests with matching implementation text. Add clean Rust CI jobs on Windows/Linux/macOS; keep Go baseline jobs while migration is incomplete. All four targets need clean artifact receipts, including macOS arm64.
6. Separate dependency-fetch/build jobs from credential-bearing publish jobs. Hand over immutable artifacts and verify hashes/readback. Do not execute arbitrary third-party build scripts with publish credentials. Distribution candidate validation is separate from public release.
7. In the isolated runtime, exercise A01-A12 and frozen Web screens. Verify served version and binary/embedded assets; observe pane/chat/history/approval input/reconnect/Files/Git/profile/routine/voice/tray flows. Browser, mobile, OS/native and real remote results are separate receipts. If not observed, mark pending.
8. Add CHANGELOG Unreleased entries for implemented user-visible C1-C4 behavior and clarify README candidate status, compatibility and rollback. Do not alter historical release descriptions to conceal differences.
9. Create a develop-targeted draft PR titled with the task label #3 many-ai-cli rust. Describe concrete behavior, traceability, tests, limitations and rollback. Include PROGRESS updates. Request Codex independent review through the operator's established path, then correct confirmed findings with regression tests and rerun relevant checks.

## Diff base and reviewed revisions

Use the initial instruction commit `6b0fb8e198750245ef8b4b475fbac9070db0ac3e` as the implementation-review diff base. The older Go SHA is the behavior oracle, not the instruction-publication base. Identify later instruction/board supplements separately; documentation and the required generated inventory in a PR are not automatically unrelated implementation changes.

Review the actual code SHA, record the independent reviewer and its verdict, and distinguish that SHA from the latest progress-only commit. After a fix, identify the changed code SHA and rerun/review the affected scope. Do not claim the old reviewed SHA covers newer code. Validate historical-data loading separately from new-input validation using the synthetic cases in 00-contracts.

## Git and external boundaries

Do not push from a linked worktree when repository hooks prohibit it. The parent can push the reviewed branch explicitly from a regular checkout without switching/resetting the user's branch. Preserve unrelated staged/dirty work. Inspect actual staged content, run the repository secret/residue checks and never publish local paths, credentials or private audit records.

The operator controls Slack/ChatGPT messages and existing Hub actions. Do not send messages to other tasks or external destinations. Do not merge/tag/release merely because a draft PR exists.

## Completion evidence

C5 is reviewed when the independent reviewer inspected the final diff, fixes and tests, CI has a final successful conclusion for the actual candidate SHA, and all programmatic A cases have receipts. Manual/native/provider/device gaps remain in C6. PR existence alone is not validation; a local file is not a clean CI artifact.
