# #3 many-ai-cli rust: progress

> 最終更新: 2026-10-03(土) 12:07:43

Task label: #3 many-ai-cli rust. This is a coordination label, not GitHub issue/PR number 3. Continue replies only in the thread where the operator starts this task; do not mix #1/#2 tasks. The operator starts dots from Slack or ChatGPT Web. This repository file and implementation diff are the reviewable progress sources.

Baseline Go SHA: 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
Instruction/implementation branch: feat/rust-migration.
PR base: develop.
Current state: instructions prepared; dots has not been started by this checkout.
Rust implementation: not started. Build/tests/manual acceptance: not run.

| Stage | Owner | State | Evidence / next action |
|---|---|---|---|
| C1 foundation | dots shared-interface owner | pending | Read 01-foundation; fix shared APIs and trial-root contract |
| C2 core | dots core lane | pending | After C1, parallel with C3/C4 |
| C3 services | dots services lane | pending | After C1, parallel with C2/C4 |
| C4 launcher/delivery | dots launcher lane | pending | After C1, parallel with C2/C3 |
| C5 integration/PR | dots integration owner; Codex independent review | pending | Complete traceability; build/test/CI; open develop draft PR |
| C6 manual/data/cutover | operator; Codex evidence review | pending | Candidate accepted, rollback rehearsal, real devices, stability |

## Update format

Append time, stage, actual owner, state, changed paths, exact commands/exit codes, candidate commit/PR/CI/artifact links, unperformed cases and next action. Update the table in the same commit. Evidence must not include secrets, private records, real home paths or copied data contents.

Allowed states: pending, in_progress, implementation_complete, reviewed, accepted, blocked. A blocker includes cause and the input/environment required to resolve it. Preserve earlier receipts; do not replace a failure with a success claim lacking rerun evidence.

## Acceptance ledger

Initialize the K01-K15 and A01-A12 mappings from 00-contracts when implementation starts. Record each mapping's fixture and caller/OS/UI evidence. For focused behavior and validation gaps, use neutral requirement descriptions and state each actual result. Missing evidence stays pending.

## Preparation record

Codex prepared these instructions after source and existing-fixture review with parallel subagents. No Go/Rust source changes, builds, Hub start/stop, real provider probes, data migration or deployment were performed in instruction preparation. The operator's initiation and dots execution receipts will be added here when they happen.
