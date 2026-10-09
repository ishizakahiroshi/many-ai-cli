---
type: reference
status: draft
tags: [ui, handoff]
owner: user
related: [README.md, REVIEW.md, FACTS.md]
last_reviewed: 2026-10-09
---

# Progress board — #P-20261008-007 many-ai-cli

> 最終更新: 2026-10-09(金) JST — combined requirement-11 candidate recovered; final publication/review/product acceptance pending

Updated: 2026-10-09. This folder is public and may be read by the delegated implementation/review workers.

Work repo: `ishizakahiroshi/many-ai-cli`.
Work branch: `dots/P-20261008-007-session-list-tabs`.
PR base: `dots/rust-recovery-resume-3`.
Behavior oracle: `4a3c141a1e6573459d3ff2a8cc6c9962bce6def3`.
Management Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/12
First instruction commit: `c29b58fa1c5bff2298197d9700c61b503b581b18`.
Current extension instruction: [add50391876467cb08ecbdb2d15e7449933af1e1](https://github.com/ishizakahiroshi/many-ai-cli/blob/add50391876467cb08ecbdb2d15e7449933af1e1/docs/bot/session-list-tabs/README.md).
Draft PR: https://github.com/ishizakahiroshi/many-ai-cli/pull/13

## Current combined candidate

Rust checkpoint 2, the exclusively transferred registration-test corrections and
Web checkpoints 1/2 have been recovered and verified locally. Requirement 11 is
implemented in this unpublished candidate; final acceptance is pending.
The current code SHA and independently reviewed SHA are not fixed yet. Local
checks include Rust correlation tests (7), session engine contracts (63), affected
Rust formatting and Clippy, plus Bun/TypeScript 6 checking and caller/DOM tests
(27). Earlier Web checkpoint 1 also passed 368 unit tests and 421 fixtures; final
combined build and product/browser acceptance will be recorded separately.
The earlier frontend-only candidate was built and checked in an isolated Windows
Hub with synthetic sessions; those observations do not prove the new correlation
candidate or native drag/touch acceptance. No production Hub was operated.

## Historical frontend-only publication (requirement 11 excluded)

The SHA, table, test counts and capability statements in this historical section
describe the earlier published frontend-only candidate, not the current work.
Fixed instruction: [4102dd2aeb4df8452c8e3f8c2751d9fdeaa0abda](https://github.com/ishizakahiroshi/many-ai-cli/blob/4102dd2aeb4df8452c8e3f8c2751d9fdeaa0abda/docs/bot/session-list-tabs/README.md).
C2 initial code: `f2ba035558882b34e465bc91481809235d93d40c`.
C2-R1 fix code: `e7eea35dc5c31f7f17a92bf0175cc7960e86d520`.
Historical published code SHA = independently reviewed code SHA: `081b0679022934eca7d19577d7c3b80b60580196`.
Draft PR: https://github.com/ishizakahiroshi/many-ai-cli/pull/13
Publication receipt: https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6070797580
Independent review receipt: https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6070849986
Validate CI for the code SHA: https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37857413422 (success; release-token-scopes intentionally skipped for pull requests).

| Stage | Owner | Issue evidence | Remaining gate |
|---|---|---|---|
| Publication/dispatch | Local coordinator / dots implementer | [Acceptance and read-back](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6062609938) | Accepted; partial scope continuation authorized |
| C2 state/save | dots implementer | [C2 code receipt](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6063138449), [C2-R1 finding](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6063190078) | Fix verified; requirement 11 remains unimplemented |
| C3 UI/manage | dots implementer | Published at 081b0679; C3-R1 long-press lifecycle fix verified | Product layout/native-input acceptance |
| C4 drag/move/status | dots implementer | Actual-listener synthetic checks and exact-source review passed | Browser/native-input acceptance |
| Independent review | Different dots worker | Fixed public SHA 081b0679 verified; no further requested fixes | Complete for this source; product acceptance remains separate |
| Product acceptance | Owner/local coordinator | Typecheck, module-init, related tests and CI passed | Windows/native input/product UI, no merge implied |

The management Issue comments are the state-transition source of truth once created. Link accepted/submitted/reviewed/blocked/owner-accepted receipts here; do not store private chat locators or machine paths. Keep instruction SHA, code SHA, reviewed SHA and board commit distinct. Do not self-reference the current board commit inside itself.

Next: implement requirement 11 and obtain actual browser/Windows product acceptance. On 2026-10-09 the owner explicitly approved all remaining steps and parallel subagents, including the focused Rust-only API/registration/WS extension in README. Requirement 11 remains **unimplemented** until a new candidate and its evidence are submitted. Go changes, guessed spawn affiliation and label correlation remain outside scope. The earlier partial-continuation receipt is historical. This is a partial submission, not whole-task acceptance. Merge, release and deployment have not been performed.

Synthetic state validation: C2-R1 reproduction failed before the fix and passed after it. New state fixtures (17) plus existing sidebar-tree fixtures (21) pass. Connected UI tests exercise actual menu, pointer and scroll callers with the repository's existing synthetic-DOM/TypeScript-transpiler testing pattern; counts and exact commands are recorded in the publication Issue receipt. These are not browser layout/native-input tests.

Capability receipt: dots originally lacked Bun and the specified TypeScript dependencies. The local coordinator subsequently used existing Bun 1.3.14 and TypeScript 6.0.3: `bun run check` passed, 177 modules evaluated without TDZ, and 94 related tests passed. CI web-check passed all steps, including 356 unit tests and 412 fixtures. No new local dependencies were installed, and no local product build or running Hub operation occurred. Dots browser attempts were blocked by IPC/URL restrictions; no bypass was attempted. Browser layout/native input and Windows product UI acceptance remain unverified. CI build/test success is not product acceptance.

Rollback: revert this task branch's feature commits or remove the browser-local `many-ai-cli-session-list-tabs-v1` item to reset only tab organization. Sessions, processes, history and server preferences are not deleted by tab reset.

## Requirement 11 resumed candidate — 2026-10-09

Fixed extension instruction: `add50391876467cb08ecbdb2d15e7449933af1e1`.
Stage start: 2026-10-08 23:57 UTC / 2026-10-09 08:57 JST.
90-minute checkpoint: 2026-10-09 01:27 UTC / 10:27 JST.
[Stage receipt](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6071492017).
The earlier “unimplemented” entry above describes the prior published code, not this candidate.

- Rust checkpoint 2 and Web checkpoint 1 were handed off as immutable hash-addressed
  patch/full-file artifacts for the local coordinator's normal Git publication.
  The artifact route avoids repeating the stalled large GitHub tree operation.
- Rust lib `spawn_correlation`: the local coordinator reported 7 passed, zero failed,
  on Rust 1.90 offline. The registration integration fixture needed completion-effect
  application and a consistent wrapper PID; ownership of that one test file was
  explicitly transferred to the local coordinator. The collision guard is unchanged.
- Web checkpoint 1: dots ran 9 pure and 23 actual-source caller/DOM tests with Node 24
  plus existing TypeScript 5.9.3, and an alternate `tsc --noEmit` check (exit 0).
  These are supplemental checks, not Bun/TypeScript 6, build or browser acceptance.
- Current candidate implements the [correlation contract](SPAWN-CORRELATION.md).
  Its exact published code SHA, independent final review SHA, required toolchain
  checks and product acceptance are pending. Do not mark requirement 11 accepted
  from these intermediate artifacts or old green CI alone.

One shared implementation owner continues Rust/Web; a separate worker reviews.
The sole transferred file is `rust/tests/session_engine/startup_registration.rs`;
future dots artifacts exclude it so the local correction is not overwritten.
No Go investigation/change/regression, generated DTO edits, new dependency/workflow,
merge, release, deployment, or production Hub operation was performed in this stage.
