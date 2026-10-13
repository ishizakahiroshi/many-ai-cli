# Progress board
> 最終更新: 2026-10-13 22:50:12 +09:00

## Task

- Task: #P-20261013-001 many-ai-cli
- Issue URL: https://github.com/ishizakahiroshi/many-ai-cli/issues/20
- Instruction repo/path: ishizakahiroshi/many-ai-cli / docs/bot/settings-usage-hub-navigation/
- Implementation branch: dots/P-20261013-001-settings-usage-hub-nav
- Planned PR base: wt/rust-ui-publication-20261011
- Product base at instruction creation: d95effb70cf550ff93ca593160ee8aa827abc28b
- Parent PR: https://github.com/ishizakahiroshi/many-ai-cli/pull/18

## Evidence pointers

| Evidence | Value |
|---|---|
| Initial instruction commit | 4d744dfbf5cea15eef16c3351694690d352f910c |
| Final fixed instruction URL | https://github.com/ishizakahiroshi/many-ai-cli/blob/9db24622dad1c2fc5ee48d8661b337a1e98a7249/docs/bot/settings-usage-hub-navigation/README.md |
| Latest PROGRESS URL | https://github.com/ishizakahiroshi/many-ai-cli/blob/dots/P-20261013-001-settings-usage-hub-nav/docs/bot/settings-usage-hub-navigation/PROGRESS.md |
| Product base SHA | d95effb70cf550ff93ca593160ee8aa827abc28b |
| Slack send/read-back | 2026-10-13; sent via verified configured DM; fixed README SHA and adopted behavior confirmed on read-back |
| Receipt | Issue #20 response reports fixed instruction, parent PR, and task branch SHAs; environment/check details are self-reported and not independently verified |
| Implementation code SHA | not submitted |
| dots independent review SHA | not submitted |
| Codex review SHA | pending |
| CI SHA and runs | pending |
| Branch commit containing latest board | record from branch history after commit |

## Work state

| Work | Owner | Issue status | Evidence and next action |
|---|---|---|---|
| Accepted design and task instruction | Codex | sent; read back; receipt received | Issue #20 contains the commit-pinned README and current board URL; receipt recorded in Issue #20 |
| Implementation | dots | receipt received; code not submitted | Receipt reports Linux x86_64 / Node v24.19.0, Bun/Cargo/rustc unavailable in PATH/cache, and a no-dependency test route under investigation. Codex rechecked parent/task refs; they match. No implementation SHA, test result, or CI result yet |
| Independent implementation review | dots reviewer | not started | Reviewer must be separate from the implementation author |
| Adversarial source review and canonical-source reflection | Codex | not started | Review the exact submitted SHA; retain unrelated dirty work |
| Windows build, Hub reload, and actual screen acceptance | user | pending | Record after the user checks the Settings initial view and both navigation destinations |

Issue comments are the source of truth for send, receipt, implementation, review, CI, and acceptance state. This board is a navigation aid. Keep send success, receipt, implementation start, submitted code, review, and CI as separate states.
