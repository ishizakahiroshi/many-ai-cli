---
type: reference
status: draft
tags: [review]
owner: user
related: [README.md, FACTS.md, PROGRESS.md]
last_reviewed: 2026-10-16
---

# #P-20261016-012 independent review

The reviewer is a different agent from the implementer. Read the whole diff from the first instruction commit to the PR head, and check it against [README.md](README.md). Do not trust the implementer's summary.

## Check in particular

1. Failed checks leave `config.yaml` byte-identical and create no backup, for every invalid input in README stage 1 section 1. Also when several inputs are wrong at once.
2. Each successful `add` and `remove` creates exactly one backup, not zero and not two (C2's `write_config` may also back up).
3. The Hub token and other `config.yaml` lines never appear in any output, including error paths and `--json`.
4. An id equal to a top-level CLI command name is refused by `provider add`, while existing entries with such ids still load as before.
5. A built-in id cannot be removed. A dropped (invalid) custom entry can be removed.
6. Approval patterns: the user's strings are loaded for the custom id, and a later remote sync does not overwrite them. `approval_pattern_source` is set only if the loader supports it.
7. The output tells the truth about whether a running Hub needs a restart. Check against the code.
8. Stage 2: a usable custom id is accepted by `orchestrate spawn`; a dropped custom id and an unknown id are refused with the list of valid ids; relay behaves as the PR says, and the child-AI text says the same.
9. Nothing outside the scope changed, in particular none of the files under "Files you must not touch" in the README, no Go code, no CI workflow, no dependency.
10. No help, doc or message text asks the user to sign in to or pay for muse.
11. Tests use synthetic temporary homes only. No real home path, personal name or private URL appears in code, tests or docs.

## Submission

Put `results/REVIEW-RESULT.md` into the Drive version folder given in the README. Include: the reviewed code SHA, each finding with severity, steps to reproduce and the file and line, the commands you ran and their results, and what you could not check. When the implementer fixes findings, review the new head again for the affected area. Earlier passes do not carry over. If there are no findings, state the range you checked.
