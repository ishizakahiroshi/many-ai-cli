# External notice candidate: explicit runtime opt-in

Base: `841aaee14f2784f02cdd6dbb491c5beb860d4db0`. This isolated candidate adds no Go changes and does not activate an existing Hub. Runtime acceptance and independent review are separate gates.

The default is off. A future operator-controlled launch of the candidate binary enables the routes only when the **Hub process** has `MANY_AI_CLI_EXTERNAL_NOTICE_STATE` set to an absolute private local directory outside Git and sync folders. An example of the shape is:

```powershell
$env:MANY_AI_CLI_EXTERNAL_NOTICE_STATE = 'C:\private-local\external-notices'
& '<reviewed-candidate-binary>' serve
```

This is a launch example, not an instruction to restart the current Hub. Use the existing authorized Hub runtime/profile selection. Confirm local storage, restrictive user ACLs, no mapped network or sync redirection, and no other Hub using the same state directory. The setting does not change configuration files. Removing the process setting disables the routes on the next launch. The SQLite file contains only event metadata, identity tuples, digests and technical results; never tokens or source comment bodies.

All routes use existing Hub authentication (`Authorization: Bearer …`) and method/Host/Origin/PIN guards. They neither spawn nor force interrupt.

- `GET /api/external-notices/target/{live_session_id}` returns `{target: {hub_instance,session_id,incarnation,wrapper,auth_epoch,started_at,cwd,codex_session_id}}` only for a connected live target. Codex requires a known internal agent session ID. The caller must explicitly compare its expected Hub/session/start/cwd/Codex session and retain the full returned tuple. Labels are not identities.
- `POST /api/external-notices` accepts exactly `{event_id,target,repo,issue,comment_id,revision,kind,body_digest}`. The two digest fields are lowercase 64-character hexadecimal; IDs/revision are positive; repo is a lowercase `owner/repo`; kind is `created`, `edited`, `deleted` or `restored`. There is no free-form text, body, shell, interrupt or Enter flag.
- `GET /api/external-notices/receipt/{event_id}` returns `{event_id,status}`. Technical states are `queued`, `transport_written`, `invalid_target`, `unknown`. HTTP success never proves AI observation or business completion.

Event IDs must derive from a durable client inbox instance ID plus repository/Issue/comment/revision/kind and the explicitly registered full target tuple, not a local receipt counter alone. A repeated event ID with identical payload returns its existing receipt. A changed payload returns 409. No more than 256 pending events are admitted. FIFO admission is preserved **within a target registration**, identified by a SHA256 of the full tuple. A busy target or old Hub registration never blocks another target's queue. `queued` is retained without typing when idle/composer/approval/input conditions are not ready. This first candidate has no automatic Hub poll daemon: an authorized client may explicitly retry the same **unchanged** queued event. It must not mint a new event ID or retarget a saved event automatically. `invalid_target` requires explicit re-registration/new event identity. `unknown` and `transport_written` must never be automatically replayed. Older experimental store schemas are refused without deletion or guessed migration.

The core rechecks the full tuple and auth epoch **after taking the input ticket**, under the same session state lock as frame reservation. It checks connected/live status, startup gate, pending/resend/inflight input, idle state, approval/user waits and a known empty composer. This pilot supports Codex only: its empty input placeholder must be visible. Side-thread/blocking screens fail closed. Claude and all other providers fail closed because a footer alone cannot prove there is no unsent draft. The notice is a fixed neutral template containing validated metadata and an untrusted-source reminder. Frame delivery uses one bracketed paste plus Enter with no forced interrupt, delayed Enter retry or reconnect queue. Activity becomes non-idle at admission to prevent another notification from using an old idle observation.

SQLite records `sending` before any transport invocation. Reopening a store converts interrupted `sending` rows to `unknown`; readback also projects live `sending` conservatively as `unknown`. Any transport or final result-save uncertainty remains non-replayable. An operator cannot infer non-delivery from an HTTP timeout. Restarting the Hub changes the target Hub instance; preserved queued rows cannot silently bind to a reused session ID. Old queued targets eventually become `invalid_target` when explicitly retried.

Generated verification artifacts in this isolated worktree are `web/node_modules/`, `web/dist/`, `rust/target/` (including native synthetic wrapper fixture). They are ignored outputs, not release artifacts. Keep them while required focused verification is pending; the worktree owner must inspect and remove or retain them explicitly before cleanup. No actual Hub startup, deployment, runtime replacement or commit/push is part of this candidate implementation.
