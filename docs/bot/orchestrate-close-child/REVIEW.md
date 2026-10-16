---
type: reference
status: draft
tags: [review]
owner: user
related: [README.md, FACTS.md, PROGRESS.md]
last_reviewed: 2026-10-16
---

# #P-20261016-013 independent review

The reviewer is a different agent from the implementer. Read the whole diff from the first instruction commit to the PR head, and check it against [README.md](README.md). Do not trust the implementer's summary.

## Check in particular

1. A conductor can never close another conductor's child, by role or by `id`, and the 404 does not reveal that the other child exists.
2. A working child and a finished child of the caller are both closed, and closing goes through `dismiss_bound` and the effects path, not through killing the process directly.
3. After a close, the child is gone from the session engine and no longer counts toward the per-parent child limit, so the conductor can spawn the same role again.
4. The board and the conductor notice say "closed by the conductor" for `orchestrate close`, and the dashboard ✕ still says "closed by the user". The mark that carries this cannot reach a different child (for example two children closed close together, or a child closed by ✕ right after a failed `close`).
5. The pending-confirmation refusal answers 409 and leaves the child running.
6. `--id` closes exactly the named child; without `--id` the newest matching child is closed.
7. Every usage string, `orchestrate --help`, `delegation_prompt.txt` and the conductor rules in `transcript_path.rs` list `close` and describe it the same way as the code behaves. Lines 11 to 13 of `delegation_prompt.txt` are untouched.
8. No output or error path prints the Hub token.
9. Nothing outside the scope changed, in particular none of the files under "Files you must not touch" in the README, no Go code, no CI workflow, no dependency.
10. Tests use synthetic sessions and temporary homes only. No real home path, personal name or private URL appears in code, tests or docs.

## Submission

Put `results/REVIEW-RESULT.md` into the Drive version folder given in the README. Include: the reviewed code SHA, each finding with severity, steps to reproduce and the file and line, the commands you ran and their results, and what you could not check. When the implementer fixes findings, review the new head again for the affected area. Earlier passes do not carry over. If there are no findings, state the range you checked.
