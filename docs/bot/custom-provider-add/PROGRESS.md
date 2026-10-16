---
type: reference
status: draft
tags: [progress]
owner: user
related: [README.md, FACTS.md, REVIEW.md]
last_reviewed: 2026-10-16
---

# #P-20261016-012 progress board

Updated by the owner's side only. The record of each state change is the management Issue's comments.

- Task: #P-20261016-012 many-ai-cli, custom provider commands (C3) and orchestrate children (C4)
- Management Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/22
- Instruction repository: `ishizakahiroshi/many-ai-cli` (public), folder `docs/bot/custom-provider-add/`
- Instruction and base branch: `dots/custom-provider-add`
- Work branch: `dots/custom-provider-add-P-20261016-012`
- Draft PR base: `dots/custom-provider-add`
- Behavior baseline: `aa9c0505`
- First instruction commit: `20fcc4e5a6b957d25446b96302113244a1f5ddf7`
- Final instruction commit: `34974b3a2afde431730947063803c94563bea016`
- Updated: 2026-10-16

| Step | Owner | State in the Issue | Evidence, remaining work, next action |
|---|---|---|---|
| Acceptance | dots | [accepted](https://github.com/ishizakahiroshi/many-ai-cli/issues/22#issuecomment-6213547116), [checked](https://github.com/ishizakahiroshi/many-ai-cli/issues/22#issuecomment-6213561296) | No Rust, cargo, bun or crates in the dots environment, so nothing is built there. The owner's side compiles and runs the full suite on the PR head |
| Stage 1 (C3) implementation | dots | [stopped: instruction vs code](https://github.com/ishizakahiroshi/many-ai-cli/issues/22#issuecomment-6213695185), answered by Supplement 1 in README | Custom providers were excluded from approval detection in `observations.rs`; the owner allowed that one file, with Go as the oracle |
| Stage 2 (C4) implementation | dots | not started | |
| Independent review | dots (a different agent) | not started | |
| Full-line review and Windows suite | owner's side | not started | |
| Hub screen and muse start check | owner | not started | |

| Item | Value |
|---|---|
| Drive version and manifest SHA256 | |
| Code SHA | |
| Reviewed SHA | |
