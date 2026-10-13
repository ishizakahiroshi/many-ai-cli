# Independent implementation review
> 最終更新: 2026-10-13(火) 22:20:28

## Review boundary

Reviewer must not be the implementation author. Review the code changes after the initial instruction commit on branch dots/P-20261013-001-settings-usage-hub-nav, while comparing behavior to the recorded PR #18 product base SHA in PROGRESS.md. Report the exact code SHA reviewed.

The public task documentation is part of the branch. Review product code and tests separately from the instruction files.

## Required review checks

1. The lower session footer contains one persistent Settings entry. Independent Usage and Hub menu buttons are gone.
2. Opening Settings shows the existing general Settings content first, even if Usage, Hub, or a different settings section was selected before closing.
3. Settings navigation exposes one Usage destination and one Hub destination. It does not add a duplicate Settings destination.
4. Usage shows all existing contract remaining amounts, provider links, unavailable-provider explanations, and local-run links. Links open with the existing security attributes.
5. Hub exposes all four existing actions: external access, mobile QR connection, Hub shutdown, and bug report. Confirmations, status feedback, and exposure state updates still work.
6. The Usage and Hub destinations are inside the Settings screen. No duplicate dropdown, body-level popover, or detached overlay remains as the user-facing route.
7. Settings search, section selection, Back, Close, Save, and Reset still work. Save handling has not been silently removed or changed.
8. Session token counts remain independent from provider Usage.
9. CSS matches the updated DOM. Nav labels, active state, focus ring, scroll, and narrow layout remain readable. No icon is clipped.
10. Japanese, English, and Vietnamese labels exist where the existing UI supports them.
11. Tests cover the user-visible transition and retained actions. Static typecheck alone is not screen evidence.
12. No new dependency, secret, private data, or unrelated UI cleanup is included.

## Report format

- Review SHA and product base SHA
- Paths and callers inspected
- Findings, each with severity, affected path/lines, impact, and concrete fix
- Commands run with exit codes
- Tests or CI not run, queued, skipped, or unavailable
- Whether the review is complete or partial

After a fix, review the new SHA. Do not carry forward a pass from an earlier SHA.
