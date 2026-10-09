# G1 same-class observation and session_update payload audit

> 最終更新: 2026-10-06(火) 14:15:48 UTC

This is the retained pre-fix baseline audit, not a review of the concurrent branch-refresh implementation. All source and documentation line references bind the public baseline below. The current G1 status and pending validation are recorded in [PROGRESS.md](PROGRESS.md) and [BEHAVIOR-MATRIX.md](BEHAVIOR-MATRIX.md). Publication adds this sanitized report and its structured inventory; the underlying audit itself ran no tests.

Audit baseline: public `bec576445a7b74505ddcd95c68f7ad2aca41ff94`, local `87fa9b735c7fa99b8776b8a58bb278290078d94b`, tree `206643f0bdf248ea0d8df05997edbfb16c2dbe08`. The owner confirmed the 2026-10-06 13:50 UTC source snapshot preceded G1 implementation edits.

## Findings at a glance

The exhaustive inventories cover all 10 SessionObservation variants, 48 top-level fields actually emitted by Go session_update, all 33 nested fields, 10 literal Go emitter shapes plus registration/reattach extensions, and 106 broad-Message fields excluded from this payload.

Two additional producer/payload gaps are verified beyond the owner’s G1 work:

- `effort`: Rust receives/stores it, but no session_update constructor assigns Message.effort. Go model detection explicitly emits it. Live model/banner effort changes can therefore fail to update an already connected card through this message path; snapshots and usage have separate paths.
- `subscription_name`: Rust has the DTO and emitter, but no production assignment to snapshot.subscription_profile_name. The configured human-readable subscription name stays empty and the UI falls back to the ID.

Three narrower differences are recorded separately:

- `transcript_grew_at` has a producer, but transcript growth only notifies the internal handoff path, without Go’s immediate growth-triggered session_update. It is later included on activity changes; Rust also uses parsed safe-offset changes rather than Go’s file-size/subagent-mtime signal.
- `route` has an output-model producer, but native wrapper registration does not send it and Rust initialization does not perform Go’s fallback inference. An explicit-model launch can retain an empty route until a model-changing output.
- `first_message`/`last_message` have working confirmed-input producers. Rust omits them in the model-update packet that Go sends, but the UI preserves prior values when absent. This is a lower-impact packet-shape difference, not proof of a visible regression or missing summary feature.

Five enum variants have no production constructors: Messages, Branch, Model, Done and CrossSessionMessage. Four of them have direct production paths outside that enum. Branch is the only baseline case with neither a constructor nor an alternative producer found, and is already owned by G1. The remaining five enum variants have actual non-test callers. All 19 relay status and five cross-message nested fields have production assignments.

## Evidence and scope

This is static source/caller analysis, not runtime reachability proof or acceptance. No builds/tests/providers, source/docs writes within the checkout, Git/Cargo operations or formal review of the concurrent G1 implementation were performed. Stale .omitnix inventory was used only for navigation, and superseded historical missing-Relays text was reconciled against current source. No startup-performance work or blanket implementation-complete claim is included. G1 branch/project/change-count implementation and its Windows regression tests remain separately owned.

[The JSON companion](G1-OBSERVATION-PAYLOAD-AUDIT.json) preserves structured rows and the complete enum occurrence inventory. The sections below contain the full tables; all source evidence binds the saved baseline rather than current edited files.

---

## G1 observation producer audit

Baseline: public `bec576445a7b74505ddcd95c68f7ad2aca41ff94`; local `87fa9b735c7fa99b8776b8a58bb278290078d94b`; tree `206643f0bdf248ea0d8df05997edbfb16c2dbe08`. The owner confirmed the source snapshot captured at 13:50 UTC preceded G1 edits.

Evidence is static source/caller analysis only. No build, test, provider launch, checkout edit, Git operation, or post-change formal review was performed. `.omitnix/index.json` was read first; its generated commit `5d423dd306761b41bda8fb45be770213afffd9bf` (dirty=true) is stale, so it was used only as navigation context. All line references below are relative to the source root and bind to the captured baseline tree `206643f0bdf248ea0d8df05997edbfb16c2dbe08`.

### Result

Of all 10 SessionObservation variants, 5 have production constructors. Five do not: Messages, Branch, Model, Done, and CrossSessionMessage. Four of those have direct production core paths that bypass the enum. Branch is the sole baseline producer absence without an alternative found, and belongs to the owner’s G1 implementation. Enum-constructor absence is therefore not equivalent to absent product behavior.

Transcript has a real producer but a separate source/publication discrepancy: its growth clock is stored without a growth-triggered session_update. Model has real state producers but effort is omitted from session_update. These payload findings are analyzed in the companion field audit.

### Exhaustive variant inventory

| Variant | Production enum constructors | Test constructors | Consumer references | Classification |
|---|---:|---:|---|---|
| Messages | 0 | 1 | `rust/src/terminal/session/observations.rs:286-301` | Enum constructor test-only; behavior source-wired through direct input path |
| Branch | 0 | 0 | `rust/src/terminal/session/observations.rs:303-306` | Absent baseline producer; G1-owned missing production wiring |
| Model | 0 | 1 | `rust/src/terminal/session/observations.rs:308-320` | Enum constructor test-only; behavior source-wired through registration/output detection |
| Transcript | 1 | 2 | `rust/src/terminal/session/observations.rs:322-345` | Source-wired enum producer; supplemental growth-publication/source divergence |
| Workflow | 1 | 4 | `rust/src/terminal/session/observations.rs:347-360` | Source-wired enum producer |
| Subagents | 1 | 4 | `rust/src/terminal/session/observations.rs:362-375` | Source-wired enum producer |
| Done | 0 | 0 | `rust/src/terminal/session/observations.rs:377-380` | Enum constructor absent; behavior source-wired through direct completion paths |
| CrossSessionMessage | 0 | 4 | `rust/src/terminal/session/observations.rs:273-281`; `rust/src/terminal/session/observations.rs:382-383` | Enum constructor test-only; behavior source-wired through direct VT observation |
| Relays | 1 | 0 | `rust/src/terminal/session/observations.rs:385-387` | Source-wired enum producer; historical missing-owner documentation superseded |
| BoardNotifyPending | 1 | 1 | `rust/src/terminal/session/observations.rs:389-391` | Source-wired enum producer |

The counts are actual `SessionObservation::Variant` occurrences classified as constructor or consumer after source inspection. CrossSessionMessage has two consumer occurrences (early dispatch and unreachable match arm). The enum derives Clone only; no serialization/deserialization constructor path is present. No variant alias/glob import providing an unqualified constructor was found. Complete occurrence inventory and text are in the `observations.occurrences` array in [G1-OBSERVATION-PAYLOAD-AUDIT.json](G1-OBSERVATION-PAYLOAD-AUDIT.json).

### Source-backed producer / consumer table

| Variant | Go production source | Rust production source or alternate path | Rust consumer | Impact / boundary |
|---|---|---|---|---|
| Messages | `internal/hub/input_gate.go:60-124` | `rust/src/hub/websocket.rs:470-479`<br>`rust/src/terminal/session/input.rs:315-373`<br>`rust/src/terminal/session/input.rs:393-403`<br>`rust/src/terminal/session.rs:616-623` | `rust/src/terminal/session/observations.rs:286-301` | Not a missing first/last-message producer. Confirmed UI input updates snapshot.first_message/last_message and summary_update sends them. The dormant enum handler instead calls update_message, which lacks first/last fields; that handler is not the normal producer. Numeric-only last-message and /clear rules are implemented directly. Does not assert every native-terminal input is observed; Go handleInput is likewise the UI pty_input path. No new dynamic test executed. |
| Branch | `internal/hub/branch_refresh.go:31-92`<br>`internal/hub/branch_refresh.go:111-177` | `rust/src/terminal/session/lifecycle.rs:235-284`<br>`rust/src/terminal/session/lifecycle.rs:523`<br>`rust/src/terminal/session.rs:547-556` | `rust/src/terminal/session/observations.rs:303-306` | Fresh sessions initialize git_root=None and branch/project_id default empty. Only this unused handler assigns branch/git_root; reattach merely copies old values. No baseline branch refresher or project_id computation was found. Go refreshes branch, change counts and stable project identity. Owner is implementing G1 separately; this report is baseline evidence. No post-G1 code review or acceptance claim. git_root is not itself a session_update JSON field. |
| Model | `internal/hub/wrapper_loop.go:1084-1086`<br>`internal/hub/model_detect.go:164-204` | `rust/src/hub/websocket.rs:568-575`<br>`rust/src/terminal/session/lifecycle.rs:247-251`<br>`rust/src/terminal/session/observations.rs:18-23`<br>`rust/src/terminal/session/observations.rs:48-67`<br>`rust/src/terminal/session/observations.rs:188-214`<br>`rust/src/terminal/session/observations.rs:748-771` | `rust/src/terminal/session/observations.rs:308-320` | Model and effort do have production state producers. Do not label both absent from an enum-constructor search. Separate payload audit identifies effort omitted from session_update and a conditional initial-route gap. Registration, initial banner and output-change source paths are traced; this is not provider execution or complete detection-format parity proof. |
| Transcript | `internal/hub/agent_chat_handler.go:314-325`<br>`internal/hub/agent_chat_handler.go:410-420`<br>`internal/hub/transcript_stall.go:144-154`<br>`internal/hub/transcript_stall.go:165-199` | `rust/src/application/session_workers.rs:485-500`<br>`rust/src/application/main_program/serve.rs:1356`<br>`rust/src/application/session_workers.rs:257-276`<br>`rust/src/application/session_workers.rs:384-401`<br>`rust/src/application/session_workers.rs:448-527`<br>`rust/src/application/session_workers.rs:582-615` | `rust/src/terminal/session/observations.rs:322-345` | Production polling parses provider transcript and publishes path/agent-session-id/safe-offset/grew_at observation. Consumer changes native path, marker source and growth clock, then only notifies TranscriptChanged. Go separately stats transcript bytes plus subagent mtime and immediately broadcasts growth. Rust activity_update can later carry the clock, so this field is populated, not globally absent. Do not infer runtime provider coverage merely because enum producer exists. Parser/page/poll state remains bounded and failure paths can skip an iteration. |
| Workflow | `internal/hub/wrapper_loop.go:1000-1004`<br>`internal/hub/workflow_scan.go:443-446`<br>`internal/hub/workflow_scan.go:556`<br>`internal/hub/workflow_journal.go:825`<br>`internal/hub/workflow_task_detail.go:497` | `rust/src/application/session_workers/observations.rs:357-374`<br>`rust/src/application/main_program/serve.rs:1356`<br>`rust/src/application/session_workers.rs:598-613`<br>`rust/src/terminal/session/observations.rs:71-80`<br>`rust/src/application/session_workers/observations.rs:202-219`<br>`rust/src/application/session_workers/observations.rs:272-363` | `rust/src/terminal/session/observations.rs:347-360` | Claude output queues VT work; periodic collector composes VT/journal/task-detail progress and applies turn-guarded observations. Consumer retains workflow and emits workflow_progress plus CoreEvent::WorkflowChanged. No missing-constructor finding. Claude-only VT gating is explicit in Go and Rust, not a newly found unsupported-provider omission. This does not prove every parser branch or timer behavior. |
| Subagents | `internal/hub/input_gate.go:112-121`<br>`internal/hub/subagent_tree.go:67-69`<br>`internal/hub/subagent_tree.go:268-279`<br>`internal/hub/subagent_tree.go:298`<br>`internal/hub/subagent_tree.go:349`<br>`internal/hub/subagent_tree.go:469` | `rust/src/application/session_workers/observations.rs:255-269`<br>`rust/src/terminal/session/input.rs:349-355`<br>`rust/src/application/session_workers.rs:598-613`<br>`rust/src/application/session_workers/observations.rs:182-200`<br>`rust/src/application/session_workers/observations/native.rs:68-101`<br>`rust/src/application/session_workers/observations/native.rs:129-219` | `rust/src/terminal/session/observations.rs:362-375` | Confirmed-turn GitTurnCapture or Reattached events arm native polling. Registry adapter selects Claude/Codex/Grok parser, changed tree produces turn-guarded observation; consumer retains nonempty tree and emits subagent_tree. No missing-constructor finding. Adapter-key dispatch is intentionally bounded by the registry and implemented readers; presence is not proof of all native data shapes. Config-disabled/terminal/no-path/no-change paths correctly produce none. |
| Done | `internal/hub/wrapper_loop.go:1104-1112`<br>`internal/hub/approval_native.go:131-162`<br>`internal/hub/done_summary.go:38-68`<br>`internal/hub/done_summary.go:188-225`<br>`internal/hub/done_summary.go:251-266` | `rust/src/hub/websocket.rs:568-575`<br>`rust/src/terminal/session/observations.rs:71-80`<br>`rust/src/terminal/session/completion.rs:650-697`<br>`rust/src/terminal/session/completion.rs:408-464`<br>`rust/src/application/session_workers.rs:528-554`<br>`rust/src/application/session_workers.rs:760-768`<br>`rust/src/terminal/session/completion.rs:723-775` | `rust/src/terminal/session/observations.rs:377-380` | Output DONE markers call publish_completion directly; native Codex completion takes the routine or Git-gated path; standby fallback is Git-gated. publish_completion retains done and emits done_summary and events. No missing-completion producer finding. No SessionObservation::Done constructor anywhere in baseline source/tests, but direct publication is sufficient source evidence against a blanket feature-absence claim. Work predicates and provider behavior not newly executed. |
| CrossSessionMessage | `internal/hub/wrapper_loop.go:989-998`<br>`internal/hub/wrapper_loop.go:1073-1074`<br>`internal/hub/cross_session_message.go:66-107` | `rust/src/hub/websocket.rs:568-575`<br>`rust/src/terminal/session/observations.rs:31-47`<br>`rust/src/terminal/session/observations.rs:179-184`<br>`rust/src/terminal/session/observations.rs:595-653` | `rust/src/terminal/session/observations.rs:273-281`<br>`rust/src/terminal/session/observations.rs:382-383` | Claude orchestration output detects receiver-side header and directly records masked sender/text on conductor, deduplicated and capped at 50, then emits session_update. The enum has no production constructor despite feature being wired. Bodies are not copied; bounded header observation matches Go design. Do not describe test constructors as production. |
| Relays | `internal/hub/relay.go:1733-1763` | `rust/src/application/relay_program.rs:958-993`<br>`rust/src/application/main_program/serve.rs:1401-1407`<br>`rust/src/application/relay_program.rs:995-1000`<br>`rust/src/application/relay_program.rs:1041` | `rust/src/terminal/session/observations.rs:385-387` | Relay transition/finish save builds sorted parent statuses from attached relay metadata and applies Relays, then sends returned effects. Consumer replaces snapshot.relays and emits session_update. No current missing-constructor finding. BEHAVIOR-MATRIX.md:27-41 preserves old missing-Relays finding under explicitly superseded history; lines 9-11 record source closure. Audit does not re-accept full relay lifecycle or external providers. |
| BoardNotifyPending | `internal/hub/orchestration.go:2684-2696`<br>`internal/hub/orchestration.go:2736`<br>`internal/hub/orchestration.go:2754-2764` | `rust/src/application/orchestration_program.rs:582-595`<br>`rust/src/application/main_program/serve.rs:1394-1399`<br>`rust/src/application/orchestration_program.rs:597-647`<br>`rust/src/application/orchestration_program.rs:649-668`<br>`rust/src/application/orchestration_program.rs:775`<br>`rust/src/application/orchestration_program.rs:808` | `rust/src/terminal/session/observations.rs:389-391` | Queue/progress/flush paths call pending_flag, which compares current value, constructs observation and applies effects. Consumer stores flag and emits session_update. No missing-constructor finding. State-dependent publication is intentional; enum existence alone would not establish this, but call sites and normal Hub owner startup are present. |

### Normal composition versus fixtures

- Ordinary `serve` dispatch constructs HubComposition and calls run: `rust/src/bin/many-ai-cli.rs:146-158`.
- HubComposition constructs workers, binds registry and owners, and starts the worker guard: `rust/src/application/main_program/serve.rs:411-420`, `597-599`, `1356`. Worker start subscribes then spawns its run loop: `rust/src/application/session_workers.rs:257-276`.
- The worker run loop delivers events to the collector and polls it every 200 ms; transcript poll is every fifth tick: `rust/src/application/session_workers.rs:582-615`.
- UI input enters the real core at `rust/src/hub/websocket.rs:470-479`; wrapper PTY output enters observe_output at `556-575`. Those paths supply messages, model, DONE and cross-session observations directly.
- Orchestration and relay owners are started by the same Hub run: `rust/src/application/main_program/serve.rs:1394-1407`. Their `pending_flag` and relay `save` paths construct the corresponding variants.

These are source-wiring observations, not proof that a particular user session, OS, provider, config or runtime scenario reached the code. No prior CI fixture success is used to infer missing live producers.

### Known design / documentation boundaries

- Claude-only workflow VT recognition is explicit in both Go (`internal/hub/wrapper_loop.go:1000-1004`) and Rust (`rust/src/application/session_workers/observations.rs:211-213,272-274`). This is not reported as a new omission.
- Native subagent dispatch explicitly supports the registry’s Claude/Codex/Grok keys on both sides (`internal/hub/subagent_tree.go:67-69`; `rust/src/application/session_workers/observations/native.rs:95-101`). Unknown adapters producing no tree are not labeled a new missing-constructor gap.
- Cross-session observations are bounded masked headers attached to a conductor, not transcript bodies (`internal/hub/cross_session_message.go:62-107`; `rust/src/terminal/session/observations.rs:595-653`).
- `docs/bot/rust-recovery-resume/BEHAVIOR-MATRIX.md:27-41` explicitly labels its old missing relay owner/Relays producer discussion historical and superseded. Current source contains a real producer. The older source-closed wording at lines 9-11 cannot establish blanket implementation completeness.
- G1 branch refresh, change-count and project_id fixes/tests remain owned separately. No repair of other findings is performed or requested by this audit artifact. Startup performance is excluded.


---

## Go session_update versus Rust field audit

Read-only static source and call-path inspection. No source/docs/Git/Cargo edits, tests, builds, providers or product runtime execution. The underlying audit collected evidence without modifying the checkout. All relative source refs bind snapshot root. Populated means a non-test production source path exists, not runtime acceptance. Branch/change-count/project_id changes belong exclusively to G1 owner and are not reviewed here.

### Identity

{
  "source_tree": "206643f0bdf248ea0d8df05997edbfb16c2dbe08",
  "source_baseline_public": "bec576445a7b74505ddcd95c68f7ad2aca41ff94",
  "source_baseline_local": "87fa9b735c7fa99b8776b8a58bb278290078d94b",
  "tree": "206643f0bdf248ea0d8df05997edbfb16c2dbe08",
  "identity_basis": "Owner confirmed pristine source at 2026-10-06 13:50 UTC snapshot; no independent Git operation in this audit.",
  "omitnix_generated_commit": "5d423dd306761b41bda8fb45be770213afffd9bf",
  "omitnix_dirty": true,
  "index_caution": "Stale context only; current source snapshot is authoritative."
}

### Findings

- Primary: effort: snapshot has real producers, every Rust session_update shape omits it; Go model detection emits it.
- Primary: subscription_name: Rust DTO/emitter exists, snapshot name has no non-test assignment, so name is never filled.
- Supplemental: transcript_grew_at: populated, but transcript growth alone emits no session_update and Rust source signal differs.
- Supplemental: route: populated after model changes, but native explicit-model registration omits Go fallback inference.
- Supplemental: first_message/last_message: populated and sent by confirmed input; lower-impact model-update packet-shape difference only, because the UI preserves absent summaries.

All top-level fields except type/token_statusbar use omitempty/serde equivalents; bool false and zero/empty values can therefore be absent without proving producer absence. Nested activity/session_meta preserve false/empty values. Broad Message contains many other frame families, which are explicitly excluded below.

Hub rust/src/hub/websocket.rs:266-280 returns registered effects; rust/src/terminal/session.rs:448-464 expands broadcast to exact UI effects; rust/src/hub/sockets.rs:523-529 sends through sockets. Shared web UI web/src/app/ws-client.ts:598-706 consumes updates. This audits static reachability only.

Counts: {"top_level_actual_go_fields": 48, "nested_fields": 33, "go_message_schema_fields": 154, "broad_proto_fields_not_emitted_by_go_session_update": 106, "top_level_statuses": {"populated": 35, "excluded_g1": 6, "never_populated": 1, "shared_constant": 1, "populated_trigger_gap": 1, "absent_from_session_update": 1, "populated_conditional_gap": 1, "populated_packet_difference": 2}}

### Complete top-level inventory

| JSON field | Status | Go producer / schema | Rust producer / emitter | Consumer and practical impact |
|---|---|---|---|---|
| type | populated | All emitter sites in the emitter-shape appendix; schema internal/proto/messages.go:22 | rust/src/terminal/session.rs:550,591,638; DTO rust/src/proto/generated.rs:13 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries session_update discriminator; no missing Rust field producer found.  |
| role | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/orchestration.go:3330-3334; schema internal/proto/messages.go:23 | rust/src/orchestration/child_launch.rs:631-642 → rust/src/terminal/session/lifecycle.rs:262-269 → rust/src/terminal/session.rs:572-579; DTO rust/src/proto/generated.rs:19 | web/src/app/ws-client.ts:685-692; Server-owned orchestration/derivation metadata is supplied to the session and emitted.  |
| session_id | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:24 | rust/src/terminal/session/lifecycle.rs:59-113 allocates → rust/src/terminal/session.rs:551,592,639; DTO rust/src/proto/generated.rs:25 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries canonical session identity; no missing Rust field producer found.  |
| provider | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:25 | rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; DTO rust/src/proto/generated.rs:31 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries provider; no missing Rust field producer found.  |
| display_name | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:30 | rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; DTO rust/src/proto/generated.rs:43 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries display name; no missing Rust field producer found.  |
| cwd | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:31 | rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; DTO rust/src/proto/generated.rs:49 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries working directory; no missing Rust field producer found.  |
| branch | excluded_g1 | internal/hub/branch_refresh.go:148-173; internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:32 | rust/src/terminal/session.rs:555; rust/src/terminal/session/observations.rs:303-306; DTO rust/src/proto/generated.rs:55 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Owned G1 Branch work; inventoried only. No assessment of owner change.  |
| project_id | excluded_g1 | internal/hub/branch_refresh.go:153-173; internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:33 | rust/src/terminal/session.rs:556; DTO rust/src/proto/generated.rs:61 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Owned G1 project_id work; inventoried only. No assessment of owner change.  |
| shell | populated | internal/hub/wrapper_loop.go:341-344,857-860; schema internal/proto/messages.go:42 | rust/src/terminal/session/lifecycle.rs:145-148,559-562 assigns announce.shell; DTO rust/src/proto/generated.rs:85 | web/src/app/ws-client.ts:639,677-678; Registration and reattachment announcements attach this field.  |
| state | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/idle_state.go:54,141; schema internal/proto/messages.go:44 | rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; rust/src/terminal/session/observations.rs:152-161,557-582; DTO rust/src/proto/generated.rs:97 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries compatibility display state; no missing Rust field producer found.  |
| output_idle | populated | internal/hub/idle_state.go:54,141; internal/hub/approval_record.go:193-216; internal/hub/orchestration.go:3369-3373; schema internal/proto/messages.go:48 | rust/src/terminal/session/observations.rs:152-161,567-582,740-743 → rust/src/terminal/session.rs:564-568,608-612; DTO rust/src/proto/generated.rs:103 | web/src/app/ws-client.ts:662-671; Activity axes produced; top-level false values intentionally omitted, nested activity carries false.  |
| workflow_active | populated | internal/hub/idle_state.go:54,141; internal/hub/approval_record.go:193-216; internal/hub/orchestration.go:3369-3373; schema internal/proto/messages.go:49 | rust/src/terminal/session/observations.rs:152-161,567-582,740-743 → rust/src/terminal/session.rs:564-568,608-612; DTO rust/src/proto/generated.rs:109 | web/src/app/ws-client.ts:662-671; Activity axes produced; top-level false values intentionally omitted, nested activity carries false.  |
| awaiting_user | populated | internal/hub/idle_state.go:54,141; internal/hub/approval_record.go:193-216; internal/hub/orchestration.go:3369-3373; schema internal/proto/messages.go:50 | rust/src/terminal/session/observations.rs:152-161,567-582,740-743 → rust/src/terminal/session.rs:564-568,608-612; DTO rust/src/proto/generated.rs:115 | web/src/app/ws-client.ts:662-671; Activity axes produced; top-level false values intentionally omitted, nested activity carries false.  |
| awaiting_approval | populated | internal/hub/idle_state.go:54,141; internal/hub/approval_record.go:193-216; internal/hub/orchestration.go:3369-3373; schema internal/proto/messages.go:51 | rust/src/terminal/session/observations.rs:152-161,567-582,740-743 → rust/src/terminal/session.rs:564-568,608-612; DTO rust/src/proto/generated.rs:121 | web/src/app/ws-client.ts:662-671; Activity axes produced; top-level false values intentionally omitted, nested activity carries false.  |
| activity | populated | internal/hub/orchestration.go:3373; internal/hub/idle_state.go:54,141; schema internal/proto/messages.go:53 | rust/src/terminal/session.rs:568,612; rust/src/terminal/session/observations.rs:152-161,567-582; DTO rust/src/proto/generated.rs:123 | web/src/app/ws-client.ts:662-667; Atomic four-axis object includes false transitions.  |
| subscription_id | populated | internal/hub/subscription.go:28-42 → internal/hub/wrapper_loop.go:165,245 → internal/hub/orchestration.go:3388; schema internal/proto/messages.go:82 | rust/src/wrapper/entry.rs:136-139 → rust/src/terminal/session/lifecycle.rs:259 → rust/src/terminal/session.rs:583; DTO rust/src/proto/generated.rs:181 | web/src/app/ws-client.ts:696-700; Profile ID is populated and emitted. Go normalizes/validates reported ID; Rust initialize copies it. Invalid manual/mixed-version input behavior is a separate validation difference, not a missing producer. |
| subscription_name | never_populated | internal/hub/subscription.go:28-42 resolves configured name → internal/hub/wrapper_loop.go:165,246 and :736 → internal/hub/orchestration.go:3389; schema internal/proto/messages.go:85 | rust/src/proto/core.rs:326-330 declares snapshot field; rust/src/terminal/session/lifecycle.rs:235-272 leaves it Default; rust/src/terminal/session.rs:584 copies it into Message; complete rust/src search finds no assignment; DTO rust/src/proto/generated.rs:187 | web/src/app/ws-client.ts:701 → web/src/app/settings.ts:3025-3029; Profile label remains empty, so serialized subscription_name is omitted and UI falls back to opaque ID. All current non-test subscription_profile_name source references are DTO declaration and emitter. No documented intentional exclusion found. |
| log_path | populated | internal/hub/wrapper_loop.go:341-344,857-860; schema internal/proto/messages.go:100 | rust/src/terminal/session/lifecycle.rs:145-148,559-562 assigns announce.log_path; DTO rust/src/proto/generated.rs:235 | web/src/app/ws-client.ts:639,677-678; Registration and reattachment announcements attach this field.  |
| jsonl_path | populated | internal/hub/wrapper_loop.go:341-344,857-860; schema internal/proto/messages.go:101 | rust/src/terminal/session/lifecycle.rs:145-148,559-562 assigns announce.jsonl_path; DTO rust/src/proto/generated.rs:241 | web/src/app/ws-client.ts:639,677-678; Registration and reattachment announcements attach this field.  |
| approval_source_epoch | populated | internal/hub/orchestration.go:3374 calls ensureApprovalSourceEpochLocked; schema internal/proto/messages.go:120 | rust/src/terminal/session/lifecycle.rs:297 initializes ApprovalState; rust/src/terminal/session/input.rs:336 advances user boundary; rust/src/terminal/session.rs:569 reads epoch; DTO rust/src/proto/generated.rs:277 | Wire compatibility metadata; shared session_update UI branch does not consume this field directly.; Approval generation is emitted; no missing producer found.  |
| token_statusbar | shared_constant | internal/proto/messages.go:128 has no omitempty; all session_update constructors leave default false; schema internal/proto/messages.go:128 | rust/src/proto/generated.rs:278-279 has no skip_serializing_if; session_update constructors use Default; DTO rust/src/proto/generated.rs:279 | Registered ACK wrapper option is outside session_update.; Both serialize token_statusbar:false incidentally on session_update; true is meaningful in registered ACK, not this payload. This is an intentional/shared wire-shape property, not a never-populated business field. |
| last_output_at | populated | internal/hub/idle_state.go:54,141; internal/hub/orchestration.go:3375; schema internal/proto/messages.go:178 | rust/src/terminal/session/observations.rs:68-70 updates output clock → rust/src/terminal/session.rs:570,601; DTO rust/src/proto/generated.rs:345 | web/src/app/ws-client.ts:672; Carries last PTY output time.  |
| transcript_grew_at | populated_trigger_gap | internal/hub/transcript_stall.go:165-199 updates on initial observation, file growth or child-directory mtime and immediately broadcasts; internal/hub/idle_state.go:54,141; schema internal/proto/messages.go:189 | rust/src/application/session_workers.rs:485-500 produces only on path/safe-offset change → rust/src/terminal/session/observations.rs:322-345 stores and Notify only → rust/src/terminal/session.rs:613 emits through activity_update; DTO rust/src/proto/generated.rs:351 | web/src/app/ws-client.ts:673 → web/src/app/session-list.ts:615; Fresh growth time can remain absent/stale in an already-connected UI while activity stays unchanged. Source signal differs from Go file-size/subagent-mtime observation. Not never populated. Idle/output state changes can later emit it (rust/src/terminal/session/observations.rs:160-161,581-582). TranscriptChanged consumer at rust/src/application/session_workers.rs:306-313 only writes handoff information. Snapshot reconnect can also expose it. |
| started_at | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:192 | rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; DTO rust/src/proto/generated.rs:357 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries session start timestamp; no missing Rust field producer found.  |
| label | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/session_meta.go:174-204; schema internal/proto/messages.go:197 | rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; rust/src/terminal/session/observations.rs:503-523; rust/src/terminal/session.rs:627-628; DTO rust/src/proto/generated.rs:363 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries editable label; no missing Rust field producer found.  |
| launch_label | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:203 | rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; DTO rust/src/proto/generated.rs:369 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries immutable launch label; no missing Rust field producer found.  |
| session_meta | populated | internal/hub/session_meta.go:174-205; internal/hub/input_gate.go:106; schema internal/proto/messages.go:207 | rust/src/hub/session_routes/metadata.rs:80-113 → rust/src/terminal/session/observations.rs:503-534 → rust/src/terminal/session.rs:625-646; rust/src/terminal/session/input.rs:340-342,372; DTO rust/src/proto/generated.rs:371 | web/src/app/ws-client.ts:618-637; Editable card metadata and derived title are sent in a nested object so clearing values survives serialization.  |
| model | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/model_detect.go:164-204; schema internal/proto/messages.go:210 | rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; rust/src/terminal/session/observations.rs:18-23,48-63,191-214,748-772; DTO rust/src/proto/generated.rs:377 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries declared or detected model; no missing Rust field producer found.  |
| effort | absent_from_session_update | internal/hub/model_detect.go:178-201 sets and emits ses.Effort; schema internal/proto/messages.go:215 | rust/src/wrapper/entry.rs:119; rust/src/terminal/session/lifecycle.rs:248; rust/src/terminal/session/observations.rs:764-771 populate snapshot then update_message; rust/src/terminal/session.rs:547-647 never assigns Message.effort; DTO rust/src/proto/generated.rs:383 | web/src/app/ws-client.ts:680 → web/src/app/token-statusbar.ts:1224-1230; Live model/banner effort updates do not reach connected session cards via session_update. Snapshot effort and usage_stat.effort_level are separate paths and may show a value after reconnect/from usage cache; this is not total effort-feature absence. |
| execution_mode | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:220 | rust/src/wrapper/entry.rs:120-121 → rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; DTO rust/src/proto/generated.rs:389 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries declared execution mode; no missing Rust field producer found. Field is emitted by both implementations; shared session_update UI branch has no execution_mode assignment. This is a shared consumer behavior, not a Rust producer gap. |
| permission_mode | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:238 | rust/src/wrapper/entry.rs:120-121 → rust/src/terminal/session/lifecycle.rs:235-272 → rust/src/terminal/session.rs:547-586; DTO rust/src/proto/generated.rs:401 | web/src/app/ws-client.ts:598-706 (shared UI); rust/src/terminal/session.rs:448-464 → rust/src/hub/sockets.rs:523-529 (delivery); Carries declared launch permission mode; no missing Rust field producer found.  |
| route | populated_conditional_gap | internal/hub/wrapper_loop.go:150-153 derives empty registration route via internal/hub/server.go:534-541 and internal/hub/env_preset.go:193-216 (nonempty Claude/Codex model gives anthropic/openai unless local-model rules apply); internal/hub/model_detect.go:165-175; internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; schema internal/proto/messages.go:242 | rust/src/wrapper/entry.rs:107-143 omits route; rust/src/terminal/session/lifecycle.rs:208,251 copies m.route; rust/src/terminal/session/observations.rs:760 assigns only on detected model change → rust/src/terminal/session.rs:562,599; DTO rust/src/proto/generated.rs:407 | web/src/app/ws-client.ts:684; Explicit-model launches can retain blank route until a model-changing output; route is not globally never populated. The socket passes the decoded register message unchanged at rust/src/hub/websocket.rs:266-274; banner detection is gated on empty model at rust/src/terminal/session/observations.rs:48-63. This is a static reachable path, not runtime reproduction. |
| parent_session_id | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/orchestration.go:3330-3334; schema internal/proto/messages.go:245 | rust/src/orchestration/child_launch.rs:631-642 → rust/src/terminal/session/lifecycle.rs:262-269 → rust/src/terminal/session.rs:572-579; DTO rust/src/proto/generated.rs:413 | web/src/app/ws-client.ts:685-692; Server-owned orchestration/derivation metadata is supplied to the session and emitted.  |
| handoff_from | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/orchestration.go:3330-3334; schema internal/proto/messages.go:251 | rust/src/application/ordinary_spawn.rs:465 → rust/src/terminal/session/lifecycle.rs:263 → rust/src/terminal/session.rs:573; warm preserve rust/src/terminal/session/lifecycle.rs:869; DTO rust/src/proto/generated.rs:419 | web/src/app/ws-client.ts:685-692; Server-owned orchestration/derivation metadata is supplied to the session and emitted.  |
| auto | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/orchestration.go:3330-3334; schema internal/proto/messages.go:252 | rust/src/orchestration/child_launch.rs:631-642 → rust/src/terminal/session/lifecycle.rs:262-269 → rust/src/terminal/session.rs:572-579; DTO rust/src/proto/generated.rs:425 | web/src/app/ws-client.ts:685-692; Server-owned orchestration/derivation metadata is supplied to the session and emitted.  |
| depth | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/orchestration.go:3330-3334; schema internal/proto/messages.go:253 | rust/src/orchestration/child_launch.rs:631-642 → rust/src/terminal/session/lifecycle.rs:262-269 → rust/src/terminal/session.rs:572-579; DTO rust/src/proto/generated.rs:431 | web/src/app/ws-client.ts:685-692; Server-owned orchestration/derivation metadata is supplied to the session and emitted.  |
| orchestration_id | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/orchestration.go:3330-3334; schema internal/proto/messages.go:254 | rust/src/orchestration/child_launch.rs:631-642 → rust/src/terminal/session/lifecycle.rs:262-269 → rust/src/terminal/session.rs:572-579; rust/src/terminal/session/relay.rs:50-60; rust/src/application/relay_program.rs:772-796; DTO rust/src/proto/generated.rs:437 | web/src/app/ws-client.ts:685-692; Server-owned orchestration/derivation metadata is supplied to the session and emitted.  |
| board_path | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/orchestration.go:3330-3334; schema internal/proto/messages.go:255 | rust/src/orchestration/child_launch.rs:631-642 → rust/src/terminal/session/lifecycle.rs:262-269 → rust/src/terminal/session.rs:572-579; rust/src/terminal/session/relay.rs:50-60; rust/src/application/relay_program.rs:772-796; DTO rust/src/proto/generated.rs:443 | web/src/app/ws-client.ts:685-692; Server-owned orchestration/derivation metadata is supplied to the session and emitted.  |
| worktree_branch | populated | internal/hub/wrapper_loop.go:198-252 → internal/hub/orchestration.go:3353-3390; internal/hub/orchestration.go:3330-3334; schema internal/proto/messages.go:256 | rust/src/orchestration/child_launch.rs:631-642 → rust/src/terminal/session/lifecycle.rs:262-269 → rust/src/terminal/session.rs:572-579; DTO rust/src/proto/generated.rs:449 | web/src/app/ws-client.ts:685-692; Server-owned orchestration/derivation metadata is supplied to the session and emitted.  |
| board_notify_pending | populated | internal/hub/orchestration.go:2754-2764 → 3385; schema internal/proto/messages.go:257 | rust/src/application/orchestration_program.rs:592 → rust/src/terminal/session/observations.rs:389-391 → rust/src/terminal/session.rs:580; DTO rust/src/proto/generated.rs:455 | web/src/app/ws-client.ts:693; Board notification flag has a live producer. Both protocols omit top-level false; that shared clear behavior is not newly a Rust omission.  |
| relays | populated | internal/hub/relay.go:476-500,1736-1763 → internal/hub/orchestration.go:3386,3395-3404; schema internal/proto/messages.go:260 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; DTO rust/src/proto/generated.rs:461 | web/src/app/ws-client.ts:694; Relay snapshots have current production construction and application; historical unsupported entry is superseded.  |
| cross_session_messages | populated | internal/hub/cross_session_message.go:66-107 → internal/hub/orchestration.go:3387; schema internal/proto/messages.go:265 | rust/src/terminal/session/observations.rs:31-44,179-183,595-653 → rust/src/terminal/session.rs:582; DTO rust/src/proto/generated.rs:467 | web/src/app/ws-client.ts:695; Direct PTY producer constructs masked receiver-side headers, bounded to 50.  |
| first_message | populated_packet_difference | internal/hub/input_gate.go:90-106; internal/hub/model_detect.go:200-201; internal/hub/branch_refresh.go:167-168; schema internal/proto/messages.go:332 | rust/src/terminal/session/input.rs:338-346,369-373 → rust/src/terminal/session.rs:616-623; model output uses rust/src/terminal/session/observations.rs:771 → rust/src/terminal/session.rs:547-586 (omits first_message); DTO rust/src/proto/generated.rs:515 | web/src/app/ws-client.ts:675-676; Confirmed Hub input sends summaries. Go model-detection updates also carry summaries; Rust model-detection update does not. The UI preserves summaries when these fields are absent, so this is a lower-impact packet-shape difference, not proof of a visible regression or a missing current summary producer. Do not treat absent Messages observation constructors as absence of the direct input producer. Messages consumer rust/src/terminal/session/observations.rs:286-301 also broadcasts broad update_message without summaries, but reachability is separate from live input. |
| last_message | populated_packet_difference | internal/hub/input_gate.go:90-106; internal/hub/model_detect.go:200-201; internal/hub/branch_refresh.go:167-168; schema internal/proto/messages.go:335 | rust/src/terminal/session/input.rs:338-346,369-373 → rust/src/terminal/session.rs:616-623; model output uses rust/src/terminal/session/observations.rs:771 → rust/src/terminal/session.rs:547-586 (omits last_message); DTO rust/src/proto/generated.rs:521 | web/src/app/ws-client.ts:675-676; Confirmed Hub input sends summaries. Go model-detection updates also carry summaries; Rust model-detection update does not. The UI preserves summaries when these fields are absent, so this is a lower-impact packet-shape difference, not proof of a visible regression or a missing current summary producer. Do not treat absent Messages observation constructors as absence of the direct input producer. Messages consumer rust/src/terminal/session/observations.rs:286-301 also broadcasts broad update_message without summaries, but reachability is separate from live input. |
| git_checked | excluded_g1 | internal/hub/branch_refresh.go:148-173; schema internal/proto/messages.go:364 | Go-compatible Message DTO exists; baseline session_update constructors at rust/src/terminal/session.rs:547-647 do not assign these fields; DTO rust/src/proto/generated.rs:557 | web/src/app/ws-client.ts:702-707; Owned G1 change-count work; inventoried only. No assessment of owner change.  |
| git_files | excluded_g1 | internal/hub/branch_refresh.go:148-173; schema internal/proto/messages.go:365 | Go-compatible Message DTO exists; baseline session_update constructors at rust/src/terminal/session.rs:547-647 do not assign these fields; DTO rust/src/proto/generated.rs:563 | web/src/app/ws-client.ts:702-707; Owned G1 change-count work; inventoried only. No assessment of owner change.  |
| git_added | excluded_g1 | internal/hub/branch_refresh.go:148-173; schema internal/proto/messages.go:366 | Go-compatible Message DTO exists; baseline session_update constructors at rust/src/terminal/session.rs:547-647 do not assign these fields; DTO rust/src/proto/generated.rs:569 | web/src/app/ws-client.ts:702-707; Owned G1 change-count work; inventoried only. No assessment of owner change.  |
| git_deleted | excluded_g1 | internal/hub/branch_refresh.go:148-173; schema internal/proto/messages.go:367 | Go-compatible Message DTO exists; baseline session_update constructors at rust/src/terminal/session.rs:547-647 do not assign these fields; DTO rust/src/proto/generated.rs:575 | web/src/app/ws-client.ts:702-707; Owned G1 change-count work; inventoried only. No assessment of owner change.  |

### Complete nested inventory

| JSON field | Status | Go producer / schema | Rust producer / emitter | Consumer and impact |
|---|---|---|---|---|
| activity.output_idle | populated | internal/hub/approval_record.go:213-216; internal/hub/idle_state.go:54,141; internal/hub/orchestration.go:3369-3373; schema internal/proto/messages.go:477 | rust/src/terminal/session/observations.rs:152-161,567-582,740-743 → rust/src/terminal/session.rs:568,612 | web/src/app/ws-client.ts:662-667; Atomic activity axis; false is retained in nested object. |
| activity.workflow_active | populated | internal/hub/approval_record.go:213-216; internal/hub/idle_state.go:54,141; internal/hub/orchestration.go:3369-3373; schema internal/proto/messages.go:478 | rust/src/terminal/session/observations.rs:152-161,567-582,740-743 → rust/src/terminal/session.rs:568,612 | web/src/app/ws-client.ts:662-667; Atomic activity axis; false is retained in nested object. |
| activity.awaiting_user | populated | internal/hub/approval_record.go:213-216; internal/hub/idle_state.go:54,141; internal/hub/orchestration.go:3369-3373; schema internal/proto/messages.go:479 | rust/src/terminal/session/observations.rs:152-161,567-582,740-743 → rust/src/terminal/session.rs:568,612 | web/src/app/ws-client.ts:662-667; Atomic activity axis; false is retained in nested object. |
| activity.awaiting_approval | populated | internal/hub/approval_record.go:213-216; internal/hub/idle_state.go:54,141; internal/hub/orchestration.go:3369-3373; schema internal/proto/messages.go:480 | rust/src/terminal/session/observations.rs:152-161,567-582,740-743 → rust/src/terminal/session.rs:568,612 | web/src/app/ws-client.ts:662-667; Atomic activity axis; false is retained in nested object. |
| session_meta.label | populated | internal/hub/session_meta.go:174-205; internal/hub/input_gate.go:90-106; schema internal/proto/messages.go:587 | rust/src/terminal/session/observations.rs:503-523,546-553 → rust/src/terminal/session.rs:627-632 | web/src/app/ws-client.ts:618-623; Card metadata; empty/false retained so edits clear values. |
| session_meta.pinned | populated | internal/hub/session_meta.go:174-205; internal/hub/input_gate.go:90-106; schema internal/proto/messages.go:588 | rust/src/terminal/session/observations.rs:503-523,546-553 → rust/src/terminal/session.rs:627-632 | web/src/app/ws-client.ts:618-623; Card metadata; empty/false retained so edits clear values. |
| session_meta.color | populated | internal/hub/session_meta.go:174-205; internal/hub/input_gate.go:90-106; schema internal/proto/messages.go:589 | rust/src/terminal/session/observations.rs:503-523,546-553 → rust/src/terminal/session.rs:627-632 | web/src/app/ws-client.ts:618-623; Card metadata; empty/false retained so edits clear values. |
| session_meta.note | populated | internal/hub/session_meta.go:174-205; internal/hub/input_gate.go:90-106; schema internal/proto/messages.go:590 | rust/src/terminal/session/observations.rs:503-523,546-553 → rust/src/terminal/session.rs:627-632 | web/src/app/ws-client.ts:618-623; Card metadata; empty/false retained so edits clear values. |
| session_meta.auto_title | populated | internal/hub/session_meta.go:174-205; internal/hub/input_gate.go:90-106; schema internal/proto/messages.go:591 | rust/src/terminal/session/input.rs:340-342 → rust/src/terminal/session.rs:632; rust/src/terminal/session/lifecycle.rs:246 restores persisted title | web/src/app/ws-client.ts:618-623; Card metadata; empty/false retained so edits clear values. |
| relays[].orchestration_id | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:646 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].plan_path | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:647 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].mode | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:648 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].state | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:649 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].reason | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:650 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].completed_cs | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:651 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].round | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:652 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].max_rounds | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:653 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].final_seen | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:654 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].implementation_session_id | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:655 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].strong_session_id | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:660 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].active_implementer | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:661 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].escalate_after | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:662 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].review_session_id | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:663 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].review_path | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:664 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].worktree_path | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:665 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].branch | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:666 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].base_commit | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:667 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| relays[].updated_at | populated | internal/hub/relay.go:476-500 → :1736-1763 → internal/hub/orchestration.go:3386; schema internal/proto/messages.go:668 | rust/src/orchestration/relay.rs:275-296 → rust/src/application/relay_program.rs:928-991 → rust/src/terminal/session/observations.rs:385-387 → rust/src/terminal/session.rs:581; nondefault sources rust/src/application/relay_program.rs:654-680, rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383 and rust/src/orchestration/relay.rs:235-250 | web/src/app/ws-client.ts:694 (stores whole relay object); Live relay status member; omitted when zero/empty, same as Go. |
| cross_session_messages[].at | populated | internal/hub/cross_session_message.go:93-107 → internal/hub/orchestration.go:3387; schema internal/proto/messages.go:449 | rust/src/terminal/session/observations.rs:633-648 → rust/src/terminal/session.rs:582 | web/src/app/ws-client.ts:695 (stores whole array); Masked cross-session header metadata; production PTY path exists. |
| cross_session_messages[].receiver_session_id | populated | internal/hub/cross_session_message.go:93-107 → internal/hub/orchestration.go:3387; schema internal/proto/messages.go:450 | rust/src/terminal/session/observations.rs:633-648 → rust/src/terminal/session.rs:582 | web/src/app/ws-client.ts:695 (stores whole array); Masked cross-session header metadata; production PTY path exists. |
| cross_session_messages[].receiver_role | populated | internal/hub/cross_session_message.go:93-107 → internal/hub/orchestration.go:3387; schema internal/proto/messages.go:451 | rust/src/terminal/session/observations.rs:633-648 → rust/src/terminal/session.rs:582 | web/src/app/ws-client.ts:695 (stores whole array); Masked cross-session header metadata; production PTY path exists. |
| cross_session_messages[].sender | populated | internal/hub/cross_session_message.go:93-107 → internal/hub/orchestration.go:3387; schema internal/proto/messages.go:452 | rust/src/terminal/session/observations.rs:633-648 → rust/src/terminal/session.rs:582 | web/src/app/ws-client.ts:695 (stores whole array); Masked cross-session header metadata; production PTY path exists. |
| cross_session_messages[].text | populated | internal/hub/cross_session_message.go:93-107 → internal/hub/orchestration.go:3387; schema internal/proto/messages.go:453 | rust/src/terminal/session/observations.rs:633-648 → rust/src/terminal/session.rs:582 | web/src/app/ws-client.ts:695 (stores whole array); Masked cross-session header metadata; production PTY path exists. |

### Go emitter shapes

The union below, including announcement extensions and non-omitempty token_statusbar, is the actual 48-field scope. Entries describe fields assigned or incidentally serialized, not a claim that empty values appear.

#### common_full

internal/hub/orchestration.go:3354-3390

type, session_id, provider, display_name, cwd, branch, project_id, label, launch_label, model, execution_mode, permission_mode, route, state, output_idle, workflow_active, awaiting_user, awaiting_approval, activity, approval_source_epoch, last_output_at, started_at, parent_session_id, handoff_from, role, auto, depth, orchestration_id, board_path, worktree_branch, board_notify_pending, relays, cross_session_messages, subscription_id, subscription_name, token_statusbar

#### model_detection

internal/hub/model_detect.go:187-202

type, session_id, provider, display_name, cwd, branch, label, model, effort, route, state, last_output_at, first_message, last_message, token_statusbar

#### transcript_growth

internal/hub/transcript_stall.go:189-197

type, session_id, provider, display_name, cwd, branch, label, model, route, state, output_idle, workflow_active, awaiting_user, awaiting_approval, activity, last_output_at, transcript_grew_at, token_statusbar

#### metadata_patch

internal/hub/session_meta.go:204-204

type, session_id, provider, display_name, cwd, branch, state, session_meta, token_statusbar

#### input_clear

internal/hub/input_gate.go:77-77

type, session_id, provider, display_name, cwd, branch, label, model, route, state, last_output_at, token_statusbar

#### input_confirmed

internal/hub/input_gate.go:106-106

type, session_id, provider, display_name, cwd, branch, label, model, route, state, last_output_at, first_message, last_message, session_meta, token_statusbar

#### mark_running

internal/hub/idle_state.go:54-54

type, session_id, provider, display_name, cwd, branch, label, model, route, state, output_idle, workflow_active, awaiting_user, awaiting_approval, activity, last_output_at, transcript_grew_at, token_statusbar

#### evaluate_idle

internal/hub/idle_state.go:141-141

type, session_id, provider, display_name, cwd, branch, label, model, route, state, output_idle, workflow_active, awaiting_user, awaiting_approval, activity, last_output_at, transcript_grew_at, token_statusbar

#### branch_git_project_refresh

internal/hub/branch_refresh.go:153-173

type, session_id, provider, display_name, cwd, branch, project_id, label, model, route, state, last_output_at, started_at, first_message, last_message, git_checked, git_files, git_added, git_deleted, token_statusbar

#### history_reset

internal/hub/server.go:2448-2448

type, session_id, provider, display_name, cwd, branch, label, model, route, state, last_output_at, started_at, token_statusbar

#### registration

internal/hub/wrapper_loop.go:339-344

type, session_id, provider, display_name, cwd, branch, project_id, label, launch_label, model, execution_mode, permission_mode, route, state, output_idle, workflow_active, awaiting_user, awaiting_approval, activity, approval_source_epoch, last_output_at, started_at, parent_session_id, handoff_from, role, auto, depth, orchestration_id, board_path, worktree_branch, board_notify_pending, relays, cross_session_messages, subscription_id, subscription_name, token_statusbar, shell, log_path, jsonl_path

#### reattach

internal/hub/wrapper_loop.go:846-860

type, session_id, provider, display_name, cwd, branch, project_id, label, launch_label, model, execution_mode, permission_mode, route, state, output_idle, workflow_active, awaiting_user, awaiting_approval, activity, approval_source_epoch, last_output_at, started_at, parent_session_id, handoff_from, role, auto, depth, orchestration_id, board_path, worktree_branch, board_notify_pending, relays, cross_session_messages, subscription_id, subscription_name, token_statusbar, shell, log_path, jsonl_path

Common helper callers: registration internal/hub/wrapper_loop.go:339; reattach helper internal/hub/wrapper_loop.go:934; approval refresh internal/hub/approval_record.go:204; board pending internal/hub/orchestration.go:2762; conductor internal/hub/orchestration.go:3332; child state internal/hub/orchestration.go:3345; relay internal/hub/relay.go:1761; cross-session header internal/hub/cross_session_message.go:105. Registration/reattach append shell/log paths after construction.

### Broad proto fields excluded from session_update

These fields exist in Go Message but no inspected production Go session_update constructor/extension assigns them. They can be populated by other frame families and must not be reported as session_update gaps. This includes permission_preset, provider_revision, workflow_progress, subagent_tree, done_summary, agent_chat messages, usage metrics, and ordinary approval records.

| JSON field | Go schema | Classification |
|---|---|---|
| provider_revision | internal/proto/messages.go:29 | broad_proto_only_for_this_audit |
| pid | internal/proto/messages.go:34 | broad_proto_only_for_this_audit |
| input_seq | internal/proto/messages.go:37 | broad_proto_only_for_this_audit |
| input_seq_high_watermark | internal/proto/messages.go:41 | broad_proto_only_for_this_audit |
| version | internal/proto/messages.go:43 | broad_proto_only_for_this_audit |
| workflow_progress | internal/proto/messages.go:54 | broad_proto_only_for_this_audit |
| subagent_tree | internal/proto/messages.go:61 | broad_proto_only_for_this_audit |
| exit_code | internal/proto/messages.go:62 | broad_proto_only_for_this_audit |
| signal | internal/proto/messages.go:66 | broad_proto_only_for_this_audit |
| token | internal/proto/messages.go:67 | broad_proto_only_for_this_audit |
| home_dir | internal/proto/messages.go:68 | broad_proto_only_for_this_audit |
| codex_home | internal/proto/messages.go:69 | broad_proto_only_for_this_audit |
| claude_dir | internal/proto/messages.go:70 | broad_proto_only_for_this_audit |
| grok_home | internal/proto/messages.go:73 | broad_proto_only_for_this_audit |
| agent_session_id | internal/proto/messages.go:76 | broad_proto_only_for_this_audit |
| usage_probe | internal/proto/messages.go:88 | broad_proto_only_for_this_audit |
| subscription_login | internal/proto/messages.go:92 | broad_proto_only_for_this_audit |
| data | internal/proto/messages.go:93 | broad_proto_only_for_this_audit |
| text | internal/proto/messages.go:94 | broad_proto_only_for_this_audit |
| messages | internal/proto/messages.go:95 | broad_proto_only_for_this_audit |
| cols | internal/proto/messages.go:96 | broad_proto_only_for_this_audit |
| rows | internal/proto/messages.go:97 | broad_proto_only_for_this_audit |
| replay_b64 | internal/proto/messages.go:102 | broad_proto_only_for_this_audit |
| reason | internal/proto/messages.go:103 | broad_proto_only_for_this_audit |
| pty_bytes | internal/proto/messages.go:111 | broad_proto_only_for_this_audit |
| replay | internal/proto/messages.go:118 | broad_proto_only_for_this_audit |
| replay_epoch | internal/proto/messages.go:119 | broad_proto_only_for_this_audit |
| approval_sig | internal/proto/messages.go:139 | broad_proto_only_for_this_audit |
| approval_source | internal/proto/messages.go:140 | broad_proto_only_for_this_audit |
| approval_summary | internal/proto/messages.go:141 | broad_proto_only_for_this_audit |
| approval_candidate_key | internal/proto/messages.go:145 | broad_proto_only_for_this_audit |
| approval_candidate_shape | internal/proto/messages.go:154 | broad_proto_only_for_this_audit |
| approval_consumed | internal/proto/messages.go:155 | broad_proto_only_for_this_audit |
| approval_consumed_epoch | internal/proto/messages.go:156 | broad_proto_only_for_this_audit |
| done_summary | internal/proto/messages.go:159 | broad_proto_only_for_this_audit |
| sent_text | internal/proto/messages.go:160 | broad_proto_only_for_this_audit |
| detected_at | internal/proto/messages.go:161 | broad_proto_only_for_this_audit |
| approval_state | internal/proto/messages.go:169 | broad_proto_only_for_this_audit |
| approval_snapshot | internal/proto/messages.go:174 | broad_proto_only_for_this_audit |
| permission_preset | internal/proto/messages.go:221 | broad_proto_only_for_this_audit |
| spawn_confirmation_id | internal/proto/messages.go:273 | broad_proto_only_for_this_audit |
| initial_prompt | internal/proto/messages.go:274 | broad_proto_only_for_this_audit |
| spawn_requested_at_ms | internal/proto/messages.go:278 | broad_proto_only_for_this_audit |
| remember_permission | internal/proto/messages.go:287 | broad_proto_only_for_this_audit |
| spawn_child_approval | internal/proto/messages.go:298 | broad_proto_only_for_this_audit |
| trust_grant_providers | internal/proto/messages.go:306 | broad_proto_only_for_this_audit |
| spawn_child_session_id | internal/proto/messages.go:321 | broad_proto_only_for_this_audit |
| inject | internal/proto/messages.go:344 | broad_proto_only_for_this_audit |
| image_data | internal/proto/messages.go:347 | broad_proto_only_for_this_audit |
| filename | internal/proto/messages.go:348 | broad_proto_only_for_this_audit |
| providers | internal/proto/messages.go:354 | broad_proto_only_for_this_audit |
| ui_active_session_id | internal/proto/messages.go:359 | broad_proto_only_for_this_audit |
| commit_subject | internal/proto/messages.go:372 | broad_proto_only_for_this_audit |
| commit_body | internal/proto/messages.go:373 | broad_proto_only_for_this_audit |
| cost_usd | internal/proto/messages.go:378 | broad_proto_only_for_this_audit |
| cost_known | internal/proto/messages.go:379 | broad_proto_only_for_this_audit |
| tokens_in | internal/proto/messages.go:380 | broad_proto_only_for_this_audit |
| tokens_out | internal/proto/messages.go:381 | broad_proto_only_for_this_audit |
| tokens_cache | internal/proto/messages.go:382 | broad_proto_only_for_this_audit |
| tokens_total | internal/proto/messages.go:383 | broad_proto_only_for_this_audit |
| ctx_window | internal/proto/messages.go:384 | broad_proto_only_for_this_audit |
| ctx_used_pct | internal/proto/messages.go:385 | broad_proto_only_for_this_audit |
| usage_model | internal/proto/messages.go:386 | broad_proto_only_for_this_audit |
| usage_started_at | internal/proto/messages.go:387 | broad_proto_only_for_this_audit |
| rl_5h_pct | internal/proto/messages.go:391 | broad_proto_only_for_this_audit |
| rl_5h_reset | internal/proto/messages.go:392 | broad_proto_only_for_this_audit |
| rl_7d_pct | internal/proto/messages.go:393 | broad_proto_only_for_this_audit |
| rl_7d_reset | internal/proto/messages.go:394 | broad_proto_only_for_this_audit |
| claude_rate_limits_present | internal/proto/messages.go:395 | broad_proto_only_for_this_audit |
| claude_5h_field_present | internal/proto/messages.go:396 | broad_proto_only_for_this_audit |
| claude_5h_present | internal/proto/messages.go:397 | broad_proto_only_for_this_audit |
| claude_7d_field_present | internal/proto/messages.go:398 | broad_proto_only_for_this_audit |
| claude_7d_present | internal/proto/messages.go:399 | broad_proto_only_for_this_audit |
| codex_rate_limits_present | internal/proto/messages.go:400 | broad_proto_only_for_this_audit |
| codex_primary_present | internal/proto/messages.go:401 | broad_proto_only_for_this_audit |
| codex_primary_used_pct | internal/proto/messages.go:402 | broad_proto_only_for_this_audit |
| codex_primary_window_minutes | internal/proto/messages.go:403 | broad_proto_only_for_this_audit |
| codex_primary_reset | internal/proto/messages.go:404 | broad_proto_only_for_this_audit |
| codex_secondary_used_pct | internal/proto/messages.go:405 | broad_proto_only_for_this_audit |
| codex_secondary_present | internal/proto/messages.go:406 | broad_proto_only_for_this_audit |
| codex_secondary_window_minutes | internal/proto/messages.go:407 | broad_proto_only_for_this_audit |
| codex_secondary_reset | internal/proto/messages.go:408 | broad_proto_only_for_this_audit |
| codex_credits_present | internal/proto/messages.go:409 | broad_proto_only_for_this_audit |
| codex_has_credits | internal/proto/messages.go:410 | broad_proto_only_for_this_audit |
| codex_credits_unlimited | internal/proto/messages.go:411 | broad_proto_only_for_this_audit |
| codex_credits_balance | internal/proto/messages.go:412 | broad_proto_only_for_this_audit |
| codex_plan_type | internal/proto/messages.go:413 | broad_proto_only_for_this_audit |
| usage_observed_at | internal/proto/messages.go:414 | broad_proto_only_for_this_audit |
| lines_added | internal/proto/messages.go:415 | broad_proto_only_for_this_audit |
| lines_removed | internal/proto/messages.go:416 | broad_proto_only_for_this_audit |
| effort_level | internal/proto/messages.go:417 | broad_proto_only_for_this_audit |
| thinking | internal/proto/messages.go:418 | broad_proto_only_for_this_audit |
| exceeds_200k | internal/proto/messages.go:419 | broad_proto_only_for_this_audit |
| duration_ms | internal/proto/messages.go:420 | broad_proto_only_for_this_audit |
| api_duration_ms | internal/proto/messages.go:421 | broad_proto_only_for_this_audit |
| output_style | internal/proto/messages.go:422 | broad_proto_only_for_this_audit |
| vim_mode | internal/proto/messages.go:423 | broad_proto_only_for_this_audit |
| agent_name | internal/proto/messages.go:424 | broad_proto_only_for_this_audit |
| repo_host | internal/proto/messages.go:425 | broad_proto_only_for_this_audit |
| repo_owner | internal/proto/messages.go:426 | broad_proto_only_for_this_audit |
| repo_name | internal/proto/messages.go:427 | broad_proto_only_for_this_audit |
| remaining_pct | internal/proto/messages.go:428 | broad_proto_only_for_this_audit |
| reasoning_output_tokens | internal/proto/messages.go:429 | broad_proto_only_for_this_audit |
| note_ok | internal/proto/messages.go:435 | broad_proto_only_for_this_audit |
| note_path | internal/proto/messages.go:436 | broad_proto_only_for_this_audit |
| binary_stale | internal/proto/messages.go:442 | broad_proto_only_for_this_audit |

### Documentation reconciliation

- docs/bot/rust-recovery-resume/BEHAVIOR-MATRIX.md:27-41: Historical absent Relays producer is explicitly superseded. Current rust/src/application/relay_program.rs:978-991 proves an application producer and effect application; do not list Relays as unsupported.
- docs/bot/rust-migration/PROGRESS.md:488: Historical claim about preserving wrapper model/effort/execution fields refers to state/launch preservation, not complete session_update emission; actual effort serializer omission remains independently evidenced.
- docs/bot/rust-recovery-resume/BEHAVIOR-MATRIX.md:153: Docs correctly retain WS/native acceptance limits; not evidence of runtime success.
- docs/bot search of effort/subscription_name/subscription_profile_name/transcript_grew_at: No explicit deliberate exclusion for the primary gaps found. Do not infer intent from a missing field or broad DTO existence.

### Independent nested cross-check

A separate source cross-check confirmed all 19 relay and 5 cross-message member producers; no nested omission found. Relay nondefault sources: rust/src/application/relay_program.rs:654-680; rust/src/application/relay_program/transitions.rs:88,137-138,190,267,383; rust/src/orchestration/relay.rs:235-250; real lifecycle set_child caller rust/src/application/relay_program/lifecycle.rs:925. Cross direct producer rust/src/terminal/session/observations.rs:31-47,179-184,633-643.

### Boundaries

No full parity claim: field presence does not establish all validation, trigger timing, clearing, restore, persistence or delivery semantics. No changes to the six G1-owned branch/project/git rows were assessed. Consumer-only omissions shared by the existing TypeScript frontend are not assigned to the Rust producer. Static absence findings were checked across rust/src, with tests distinguished from production. No test or provider was run.
