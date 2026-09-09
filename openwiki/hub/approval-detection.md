---
type: architecture-component
title: Approval Detection and Marker System
description: How the Hub detects an AI CLI's approval prompt across providers — transcript vs terminal-mirror marker sources, the single candidateKey+sourceEpoch identity rule, pattern-profile trigger phrases, and the separate opt-in auto-approval policy layer.
tags: [approval, hub, marker, candidate-key, transcript, vt-mirror, autoapproval, risk-tier]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-ebe81a9b81f3ddb0fe384827
    resource: repo://internal/approval/summary.go
  - id: openwiki-source-f4df6c3f52df708e7f496105
    resource: repo://internal/autoapproval/policy.go
  - id: openwiki-source-148a7b07d79dbf113290d7ce
    resource: repo://internal/hub/approval_detector.go
  - id: openwiki-source-bfbd2107456c432ff01fee48
    resource: repo://internal/hub/approval_identity.go
  - id: openwiki-source-f94f819ef1c0ccbba6bb6c7f
    resource: repo://internal/hub/approval_marker_transcript.go
  - id: openwiki-source-7bbcfa927cb515aa35dc8901
    resource: repo://internal/hub/approval_marker.go
  - id: openwiki-source-f56250d3883911b64abc3676
    resource: repo://internal/hub/auto_approval.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Two marker sources, one per session

The Hub extracts the `[MANY-AI-CLI]...[/MANY-AI-CLI]` approval marker block a wrapped CLI writes into its own output through one of two sources, and — critically — only ever one source per session at a time: for `claude`/`codex` (the providers with a readable agent-chat transcript file), the Hub reads the marker straight out of that transcript; every other provider's marker comes from the Hub's own VT (terminal) mirror of the PTY output. `approvalMarkerSourceIsTranscriptLocked` is the single switch deciding which applies to a given session.

This split exists because of a concrete failure mode (`docs/local/bugfix_approval-marker-block-overflows-screen_2026-08-29.md`): the approval marker is really an AI-to-Hub message, but reading it out of "what the terminal displays" makes its capacity a function of the window's height. Claude Code repaints its alternate screen at absolute coordinates via Ink, so an answer that overflows the visible screen is not scrolled — it is simply never drawn at all, and the Hub's VT mirror's scrollback only retains lines that were actually pushed out by a `newLine()`, so an overflowing block was unrecoverable from either the live screen or scrollback. The same content exists, complete and independent of terminal size, in the CLI's own transcript file, written in the same second the terminal drew the closing marker.

Mixing both sources for one session is explicitly avoided: if the same question were extracted from both the VT mirror (where a long question can be truncated by line-wrap) and the transcript (where it is not), the two would normalize to different candidate keys, silently doubling the "one identity source" rule described below. So once a session locks onto transcript mode, `wrapperLoop`'s PTY-data path and the native-approval replay evaluator both skip VT-side marker extraction entirely for that session.

**Falling back when the transcript stops being readable**: locking onto the transcript irreversibly would mean a moved/deleted/unparseable transcript file goes silent — approvals stop appearing with only a debug-log line to show for it, which is exactly the class of silent failure this design was built to eliminate. So `approvalMarkerSourceIsTranscriptLocked` re-evaluates on every poll: after `approvalMarkerTranscriptMissLimit` (3) consecutive failed reads, the session falls back to the VT mirror (logging one `Warn`, deliberately not silent), and switches back once the transcript becomes readable again. Three is chosen because polling runs on a fixed 1-second rhythm, so a miss count doubles as elapsed seconds without needing a separate timer, and 3 seconds tolerates a transient blip (log rotation, a `--resume` file swap) without waiting too long to fail over. The accepted tradeoff: a session mid-failover can show the same approval twice (once under each source's differently-keyed candidate) — judged far less harmful than an approval never appearing at all.

## Extracting the latest marker block only

`extractApprovalMarkerBlock` (`approval_marker.go`) deliberately returns the block bounded by the **last** CLOSE tag and the nearest OPEN tag *before* it, not the first OPEN/first CLOSE pair. The terminal is a history: when a CLI redraws the same question, an earlier generation's OPEN can still be sitting in scrollback. Taking the first OPEN through the first CLOSE after it would span across generations — swallowing a stale OPEN from an old block inside a new one, which the extractor would then flag as `marker_leak` (an OPEN nested inside another OPEN) and refuse to show at all, even with a non-greedy regex, because the *start* position is still pinned to that first stale OPEN. Anchoring both ends to the last CLOSE guarantees only the newest generation is ever extracted.

## Native (non-marker) approval detection

Beyond the `[MANY-AI-CLI]` marker protocol (which requires the wrapped CLI to have approval-rules cooperation injected into its own rule file), the Hub also runs `approval_detector.go`'s native Go-side VT scan: it re-scans the tail of the terminal mirror (`vtTailLinesForApproval` lines pulled, `approvalRecentLines` treated as the valid candidate window) whenever a new PTY chunk arrives, looking for approval-shaped hint tokens — for Japanese, a fixed list including 許可/承認/続行/実行しますか/よろしいですか/確認してください. This is a separate, non-overlapping code path from the browser's own `approval.ts` scan (which works over the full xterm.js buffer rather than a PTY-chunk-triggered VT tail); a shared detection signature (`sig`) lets the Hub deduplicate if both layers happen to fire on the same content, so the two independent scans never cause a double-send.

## Candidate identity: `candidateKey` + `sourceEpoch`, one source of truth

`approval_identity.go` is, by its own header comment, the canonical statement of this rule for the whole codebase (moved out of `CLAUDE.md` in 2026-08-19 specifically so the file that must be opened to fix an approval mis-display is also the file that states the rule, rather than a separate always-loaded document that kept growing every time an incident added another paragraph). Exactly one piece of state answers "has this approval already been answered": the pair `(candidateKey, sourceEpoch)` — mirrored on the browser side by `web/src/app/approval-answered.ts` (see [Approval UI and Marker Filtering](/openwiki/frontend/approval-ui.md)). Before this, the same job was split across three mechanisms with three different definitions of "same question" — a timer-expiring option-signature (`approvalConsumedSig`), a permanently-marked full-block hash (`answeredMarkerSigs`), and a manual-dismiss question-hash (`approvalQuestionKey`) — and the disagreements between the three were themselves the source of bugs: the timer variant re-showed an already-answered approval mid-redraw once its timer lapsed, while the full-block-hash variant permanently suppressed a question the agent had genuinely asked again. All three were removed in v0.7.

Rules fixed by this file, enforced in source by `TestApprovalSuppressionStateIsSingleSource`:

- **Never add a second suppression mechanism to patch a mis-display.** First determine whether the existing single source already explains the symptom; if it does, the fix belongs in how `candidateKey` is built or how `sourceEpoch` advances, not in a new piece of state.
- `candidateKey` is derived from provider, approval kind, normalized question text, option numbers, and send text — never label whitespace or box-drawing characters, which would otherwise make every TUI redraw look like a new candidate. One documented exception: a native approval whose displayed question does not itself name the command folds the extracted command text into the identity (see `approvalIdentityQuestionWithContext`'s own comment for the tradeoff accepted there).
- `sourceEpoch` only advances at a live prompt boundary — never during replay or terminal reflow, since advancing then would make a merely-restored prompt look like a brand-new one.
- A new generation showing the same question text as a prior one is not suppressed — that is the intended behavior, not a bug to "fix" by reinstating permanent suppression.
- An answered record is carried across user turns only for as long as the answered block can still be extracted from the live VT — not bounded by a reply count. (This was a deliberate 2026-08-27 user decision replacing an earlier "carry for exactly one turn" rule; the accepted tradeoff is that if the agent reissues a byte-identical question while the answered copy is still visible on screen, that reissue does not reopen the panel, even though it is still visible in the raw terminal — judged better than the previous fixed-turn-count rule's guaranteed reappearance by the third turn.)

## Pattern profiles and remote sync

Beyond marker/native detection, the browser also matches plain trigger phrases per provider (`matchProviderApprovalTrigger` in `approval.ts`), sourced from `resources/approval-patterns/<provider>.md` — one Markdown file per provider (`claude`, `codex`, `copilot`, `cursor-agent`, `grok`, `opencode`, `command-code`, plus a shared `common.md`), each a flat bullet list of backtick-quoted phrases (e.g. `` `do you want to` ``, `` `esc to cancel` ``). `approval_patterns.go` and `approval_patterns_sync.go` fetch these from their configured source (official GitHub-raw URL by default, or a local/custom override — see `ApprovalPatternSources` in [Configuration and On-Disk Layout](/openwiki/architecture/configuration.md)) at Hub startup and cache them for the browser to load; `approval_patterns_custom.go` lets a user maintain edits separately from the official set via `approval_profiles`. The exact same mechanism, via a custom provider's `approval_pattern_source`, extends this to an arbitrary registered CLI — see [Custom Providers](/openwiki/extensibility/custom-providers.md).

## Risk facts vs. an opt-in policy decision: two different packages

Two small packages divide "what is this approval" from "should it be allowed automatically," on purpose:

- **`internal/approval`** — its own package doc states it "provides the provider-neutral facts used to present and decide pending tool approvals" and "deliberately does not make a decision about whether an approval may be bypassed." It parses raw approval text into a command, referenced paths, and a `RiskTier`, which callers may use as one input to their own policy, but it never decides anything itself.
- **`internal/autoapproval`** — the opt-in policy layer that actually decides. Its `Policy`, loaded from `~/.many-ai-cli/auto-approval.yaml` via `autoapproval.Load()`, evaluates a set of user-authored `Rule` entries (a command match plus optional `risk`/`working_dir` constraints) against a candidate and returns a `Decision`. `Rule` is deliberately narrow — the package doc notes there is no "deny override" for a hard-blocked command; a rule can only narrow what is auto-approved, never re-permit something the Hub already refuses to auto-approve.
- **`internal/hub/auto_approval.go`** — the Hub-side wiring: it keeps a bounded (`autoApprovalHistoryLimit` = 100) in-memory history of `autoApprovalCandidate` records (session, provider, cwd, the `approval.Summary`, and the resulting `autoapproval.Decision`) for the UI to display, and writes an audit trail through a `lumberjack`-rotated log file rather than the main Hub log, so auto-approval decisions have a durable, size-bounded record independent of general log rotation settings.
