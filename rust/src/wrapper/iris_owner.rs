//! Trusted wrapper owner observer. Bootstrap is server-owned metadata, stripped
//! before provider exec. The fixed operator manifest and durable Hub admission
//! are both required; neither an environment value nor a spawn proof grants work.
use crate::{
    hub::{
        iris_execution_folder as folder,
        iris_execution_grants::{ApprovedGrant, GrantLoader},
        iris_execution_journal::Journal,
        iris_execution_mcp::parse_json,
        iris_execution_owner::{OwnerStore, OwnerTicket, sha},
    },
    process::{
        ProcessPlan,
        pty::{LocalCessation, PtyExit},
    },
    wrapper::runtime::ExecutionOwnerObserver,
};
use serde_json::{Value, json};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub const OWNER_ENV: &str = "MANY_AI_CLI_IRIS_OWNER";
fn failure() -> io::Error {
    io::Error::other("trusted execution owner conditions were not satisfied")
}
fn now_ms() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| failure())?
        .as_millis()
        .try_into()
        .map_err(|_| failure())
}
fn scope(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        for field in ["attempt_id", "executor_instance_id", "fence"] {
            map.remove(field);
        }
    }
    value
}
/// The first reason recorded for a collection stands. A later step may fail as
/// well, but it never rewrites why the folder was already judged unexpected
/// (for example `PAYLOAD_CHANGED`); nothing is recorded for the later failure.
fn note_reason(reason: &mut Value, detail: &mut Value, code: &str, because: Value) {
    if reason.is_null() {
        *reason = json!(code);
        *detail = because;
    }
}

pub struct DurableExecutionOwner {
    root: PathBuf,
    ticket: OwnerTicket,
    grant: ApprovedGrant,
    store: Mutex<OwnerStore>,
}
impl DurableExecutionOwner {
    pub fn instruction_hash(&self) -> &str {
        &self.ticket.binding.instruction_hash
    }
    /// Trusted local collector for the real bounded profile. It records the
    /// after snapshot and comparison, then builds the envelope itself from
    /// the payload bytes: outcome is always unknown and synthetic is false.
    /// Collection failures are recorded as fixed reason codes; only storage
    /// failures are errors. No flag here ever verifies task or effects.
    fn collect(&self, store: &mut OwnerStore) -> io::Result<()> {
        let Some(payload) = &self.grant.payload else {
            return Ok(());
        };
        let t = &self.ticket;
        let record = store.query(&t.binding).map_err(|_| failure())?;
        if record["local_tree_ceased"] != true || record["cessation_receipt"]["ticket"] != json!(t)
        {
            return Err(failure());
        }
        let at = now_ms()?;
        let after = folder::snapshot(&self.grant.approved_cwd);
        let after_document = folder::snapshot_document(t, "after", &after, at);
        store
            .record_folder(t, "after", &after_document)
            .map_err(|_| failure())?;
        let before = store.query_folder(&t.binding).map_err(|_| failure())?["before"].clone();
        let baseline: Option<Vec<folder::FolderEntry>> = (before["complete"] == true)
            .then(|| serde_json::from_value(before["entries"].clone()).ok())
            .flatten();
        let key = folder::relative_key(payload.relative());
        let mut comparison_error = Value::Null;
        let (mut effects, changes) = match (&baseline, &after, &key) {
            (Some(old), Ok(new), Some(key)) => folder::compare(old, new, key),
            (_, Err(error), _) => {
                comparison_error = json!(error.code);
                (folder::UNEXPECTED_CHANGE, vec![])
            }
            _ => {
                comparison_error = json!("BEFORE_UNAVAILABLE");
                (folder::UNEXPECTED_CHANGE, vec![])
            }
        };
        let mut reason = Value::Null;
        let mut reason_detail = Value::Null;
        let mut payload_sha256 = Value::Null;
        let mut result_sha256 = Value::Null;
        let mut outcome_sequence = Value::Null;
        if std::fs::symlink_metadata(self.grant.result.approved_path()).is_ok() {
            // Never overwrite or adopt an existing result, whoever wrote it.
            effects = folder::UNEXPECTED_CHANGE;
            reason = json!("RESULT_PREEXISTING");
        } else {
            match payload.verify(None) {
                Err(error) => {
                    reason = json!("PAYLOAD_UNAVAILABLE");
                    reason_detail = json!(error.code);
                }
                Ok(artifact) => {
                    payload_sha256 = json!(artifact.sha256());
                    // The bytes put in the envelope must be the ones the
                    // after snapshot compared; a later rewrite is a change.
                    // A failed after snapshot is already unexpected and keeps
                    // its own comparison_error code.
                    if after.is_ok()
                        && key.as_deref().is_none_or(|key| {
                            !folder::payload_matches(&after, key, artifact.sha256())
                        })
                    {
                        effects = folder::UNEXPECTED_CHANGE;
                        reason = json!("PAYLOAD_CHANGED");
                    }
                    // From here on a reason is only added when none stands
                    // yet: `PAYLOAD_CHANGED` is never overwritten.
                    match parse_json(artifact.bytes()) {
                        Err(_) => note_reason(
                            &mut reason,
                            &mut reason_detail,
                            "PAYLOAD_JSON",
                            json!(null),
                        ),
                        Ok(answer) => {
                            let envelope = json!({"binding":t.binding,"owner_id":t.owner_id,
                                "owner_epoch":t.owner_epoch,
                                "instruction_hash":t.binding.instruction_hash,
                                "outcome":"unknown","effects":effects,"synthetic":false,
                                "payload":answer});
                            let bytes = serde_json::to_vec(&envelope).map_err(|_| failure())?;
                            match self.grant.result.create_exclusive(&bytes) {
                                Err(error) if error.code == "ARTIFACT_EXISTS" => {
                                    effects = folder::UNEXPECTED_CHANGE;
                                    note_reason(
                                        &mut reason,
                                        &mut reason_detail,
                                        "RESULT_PREEXISTING",
                                        json!(null),
                                    );
                                }
                                Err(error) => note_reason(
                                    &mut reason,
                                    &mut reason_detail,
                                    "RESULT_WRITE",
                                    json!(error.code),
                                ),
                                Ok(()) => {
                                    let digest = sha(&bytes);
                                    result_sha256 = json!(digest);
                                    let next =
                                        store.query(&t.binding).map_err(|_| failure())?["sequence"]
                                            .as_u64()
                                            .ok_or_else(failure)?
                                            .checked_add(1)
                                            .ok_or_else(failure)?;
                                    match store.append_outcome(t, next, &self.grant.result, &digest)
                                    {
                                        Ok(receipt) => {
                                            outcome_sequence = receipt["sequence"].clone()
                                        }
                                        Err(error) => note_reason(
                                            &mut reason,
                                            &mut reason_detail,
                                            "OUTCOME_REJECTED",
                                            json!(error.code),
                                        ),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let truncated = changes.len() > 100;
        let collection = json!({"kind":"folder_collection","phase":"collection","ticket":t,
            "effects":effects,"changes":changes.into_iter().take(100).collect::<Vec<_>>(),
            "changes_truncated":truncated,"comparison_error":comparison_error,
            "payload_path":key,"payload_sha256":payload_sha256,"reason":reason,
            "reason_detail":reason_detail,"result_sha256":result_sha256,
            "outcome_sequence":outcome_sequence,"envelope_outcome":"unknown",
            "envelope_synthetic":false,"observed_at_ms":at,"verification_scope":folder::SCOPE,
            "effects_verified":false,"task_outcome_verified":false,"remote_jobs_stopped":false});
        store
            .record_folder(t, "collection", &collection)
            .map_err(|_| failure())?;
        Ok(())
    }
    /// Called only for a core-issued wrapper spawn; does not launch anything.
    pub fn adopt(root: &Path, encoded: &str, provider: &str, profile: &str) -> io::Result<Self> {
        let value = parse_json(encoded.as_bytes()).map_err(|_| failure())?;
        let ticket: OwnerTicket = serde_json::from_value(value).map_err(|_| failure())?;
        if ticket.binding.provider != provider
            || ticket.binding.profile_id != profile
            || ticket.owner_id.is_empty()
            || ticket.owner_epoch == 0
        {
            return Err(failure());
        }
        let mut loader = GrantLoader::open(root).map_err(|_| failure())?;
        let grant = loader
            .load_approved()
            .map_err(|_| failure())?
            .into_iter()
            .find(|g| scope(json!(g.grant.binding)) == scope(json!(ticket.binding)))
            .ok_or_else(failure)?;
        if grant.grant.revoked || grant.grant.expires_at <= (now_ms()? / 1000) as i64 {
            return Err(failure());
        }
        let store = OwnerStore::open(&root.join("iris-execution").join("owner.sqlite3"))
            .map_err(|_| failure())?;
        Ok(Self {
            root: root.to_path_buf(),
            ticket,
            grant,
            store: Mutex::new(store),
        })
    }
}
impl ExecutionOwnerObserver for DurableExecutionOwner {
    fn before_spawn(&self, session_id: i64, process: &ProcessPlan) -> io::Result<()> {
        if process.cwd.canonicalize().map_err(|_| failure())? != self.grant.approved_cwd {
            return Err(failure());
        }
        // Fresh revocation/revision and instruction checks at the actual exec
        // boundary, not only when the wrapper originally loaded configuration.
        let mut loader = GrantLoader::open(&self.root).map_err(|_| failure())?;
        let live = loader
            .load_approved()
            .map_err(|_| failure())?
            .into_iter()
            .find(|g| scope(json!(g.grant.binding)) == scope(json!(self.ticket.binding)))
            .ok_or_else(failure)?;
        let at = now_ms()?;
        if live.grant.revoked
            || live.grant.expires_at <= (at / 1000) as i64
            || live.approved_cwd != self.grant.approved_cwd
        {
            return Err(failure());
        }
        live.instruction
            .verify(Some(&self.ticket.binding.instruction_hash))
            .map_err(|_| failure())?;
        Journal::inspect_dispatch(
            &self.root.join("iris-execution/journal.sqlite3"),
            &self.ticket.binding,
            (at / 1000) as i64,
        )
        .map_err(|_| failure())?;
        if live.payload.is_some() != self.grant.payload.is_some() {
            return Err(failure());
        }
        let mut store = self.store.lock().map_err(|_| failure())?;
        let ticket = store
            .begin_owner(
                &self.ticket.binding,
                &self.ticket.owner_id,
                self.ticket.owner_epoch,
                at,
            )
            .map_err(|_| failure())?;
        if self.grant.payload.is_some() {
            // Collector grants persist the whole approved folder baseline
            // before the claim. An oversized, unreadable or aliased folder
            // refuses the launch; nothing is spawned without a baseline.
            let entries = folder::snapshot(&self.grant.approved_cwd)
                .and_then(|entries| folder::require_plain(&entries).map(|_| entries))
                .map_err(|_| failure())?;
            let document = folder::snapshot_document(&ticket, "before", &Ok(entries), at);
            store
                .record_folder(&ticket, "before", &document)
                .map_err(|_| failure())?;
        }
        // The persisted one-use claim closes the crash/replay boundary before
        // factory.spawn. Re-entering with the same ticket is never a retry.
        store
            .claim_spawn(&ticket, session_id, at)
            .map_err(|_| failure())
    }
    fn root_started(&self, pid: u32) -> io::Result<()> {
        self.store
            .lock()
            .map_err(|_| failure())?
            .bind_root(&self.ticket, pid, now_ms()?)
            .map(|_| ())
            .map_err(|_| failure())
    }
    fn finished(
        &self,
        exit: Option<&PtyExit>,
        cessation: Option<&LocalCessation>,
    ) -> io::Result<()> {
        let mut store = self.store.lock().map_err(|_| failure())?;
        let mut ceased = false;
        if let Some(proof) = cessation {
            let next = store.query(&self.ticket.binding).map_err(|_| failure())?["sequence"]
                .as_u64()
                .ok_or_else(failure)?
                .checked_add(1)
                .ok_or_else(failure)?;
            store
                .append_cessation_with_exit(&self.ticket, next, proof, exit)
                .map_err(|_| failure())?;
            ceased = true;
        }
        if self.grant.payload.is_some() {
            // Collector grants never read a provider-written result file. The
            // collector runs only after this call persisted native cessation.
            return if ceased {
                self.collect(&mut store)
            } else {
                Ok(())
            };
        }
        // Missing output is not success. A present output is independently
        // hashed and bound, but remains an artifact report, not task acceptance.
        match self.grant.result.verify(None) {
            Ok(artifact) => {
                let next = store.query(&self.ticket.binding).map_err(|_| failure())?["sequence"]
                    .as_u64()
                    .ok_or_else(failure)?
                    .checked_add(1)
                    .ok_or_else(failure)?;
                store
                    .append_outcome(&self.ticket, next, &self.grant.result, artifact.sha256())
                    .map_err(|_| failure())?;
            }
            Err(error) if ["ARTIFACT_READ", "ARTIFACT_PATH"].contains(&error.code.as_str()) => {}
            Err(_) => return Err(failure()),
        }
        Ok(())
    }
}
