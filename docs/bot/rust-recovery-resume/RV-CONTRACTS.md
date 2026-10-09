---
type: reference
status: draft
tags: [rust, recovery, contracts, dots]
owner: unknown
review_status: draft
related: [README.md]
last_reviewed: 2026-10-05
---

# #3 R01–R03 and V01–V08 definitions

This corrects a missing public reference in the recovery README. The original
public [contracts](../rust-migration/00-contracts.md) include the focused behavior
and validation gaps without the R/V identifiers. The definitions below preserve
the existing migration handoff requirements and assign their existing identifiers;
they are a supplement to task #3, not new work or a restart. Keep K01–K15 and
A01–A12 unchanged. Use the supplied immutable supplement commit when citing this
file; record that SHA in PROGRESS.md and the existing #3 conversation.

The Go behavior oracle remains `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.
R01–R03 are intentional defect corrections: document their deliberate differences
from the oracle and regression evidence rather than reproducing the defects.
V01–V08 are validation gaps, not a claim of demonstrated code defects or passed
acceptance. The integration owner records each item's implementation/test or
review evidence, exact candidate SHA, remaining target acceptance and next action
in PROGRESS.md. Historical handoff statuses are not current acceptance receipts.

## Required defect corrections

| ID | Required behavior | Existing contract / acceptance |
|---|---|---|
| R01 | On initial seed of an absent profile, omit only many-ai-cli-owned temporary hook/session references. Preserve user settings and user hooks. Cover both absent and existing destinations. | K07 / A08 |
| R02 | Resolve and run the update command's argv[0]. Preview, log and the job's actual executable must agree. A synthetic launch=A/update=B case runs B; missing B fails without running A. | K08 / A09 |
| R03 | Preserve the complete remote script through both quotation layers (SSH argument join and remote shell parsing). Keep import/start/CWD/env/cleanup consistent. Use fake SSH with synthetic paths containing spaces, quotes, newlines and shell punctuation. | K14 / A09 and A12 |

## Required validation gaps

| ID | Required verification | Evidence and remaining acceptance |
|---|---|---|
| V01 | Windows headless cancellation: reproduce failed Job attachment together with a grandchild retaining pipes, and check whether waiting finishes after the deadline. Decide the correction from the reproduction. | Synthetic child fixtures plus native Windows acceptance; relates to K03/K10 and A04/A11. A Linux fixture cannot establish the Windows result. |
| V02 | Windows ConPTY teardown: check normal clients, detached children, retained pipes, early exit and supported OS versions for exit and residual processes. Decide any Job-based approach from results and side effects. | Native Windows lifecycle receipts plus available synthetic coverage; relates to K03/K10 and A04/A11. Do not prescribe an unverified Job change as the acceptance result. |
| V03 | Before finalizing release construction, evaluate whether build dependencies persist into the publish runner and can reach credentials. Evaluate isolation while preserving signatures and artifact identity. | Source/workflow review and clean artifact evidence; relates to K15/A12. This does not authorize release publication or credential use. |
| V04 | Check canonical-path, rename and symlink races and concrete sensitive-name cases using synthetic files. Do not impose a blanket cwd restriction on token holders. Preserve the established token-holder authorization contract. | Files fixtures and boundary review, K06/A07; synthetic names only, without publishing private watchlists or real secrets. |
| V05 | Close the unscanned full-history secret gap using a scanner path checked for redaction, private watchlist handling and no external secret upload. Never print raw hits or secret values. | Record scan scope, revisions and sanitized results. Full-history/PR-range evidence and latest-push evidence remain separate; unresolved safe-path or scan coverage stays pending with the next action. |
| V06 | For selected Rust dependencies and distribution inputs, verify recent publisher/provenance, vendored byte hashes, advisories/KEV and reachability against official primary sources. | Record pinned inputs and upstream evidence, K15/A12. Evidence gathering does not authorize unrelated dependency upgrades. |
| V07 | Confirm effective deployment boundaries: proxy/Host/Origin/PIN/egress/IAM and native DLL/model values on the actual target. Repository configuration alone does not prove the target's state. | Available synthetic coverage and owner-led target/native acceptance, including K02/K13/K14 and A02/A12. Keep unavailable account/GUI/remote/target checks explicitly pending. |
| V08 | Before cutover, verify stored-data backup, copied migration, rollback, FTS, locking and pruning. Prohibit concurrent old/new writes to the same DB. | Synthetic/copied-data receipts and separately authorized installed-data acceptance, K04/A05. Close writers before copying/restoring; synthetic rollback is not installed-data acceptance. |

V01, V02 and V03 are required and must not be dropped as optional. An unavailable
environment leaves an item pending with its owner and concrete next action;
automated stub success does not stand in for real-provider, remote, mobile or OS
acceptance. Keep the continuation's isolation and authorization boundaries:
no running-Go cutover, merge, release, package publication, real credential/data
operations, paid provider probes, real notifications or destructive installation.
Preparing fixtures and review evidence does not authorize those operations.
