//! Trusted wrapper owner observer. Bootstrap is server-owned metadata, stripped
//! before provider exec. The fixed operator manifest and durable Hub admission
//! are both required; neither an environment value nor a spawn proof grants work.
use crate::{
    hub::{
        iris_execution_grants::{ApprovedGrant, GrantLoader},
        iris_execution_journal::Journal,
        iris_execution_mcp::parse_json,
        iris_execution_owner::{OwnerStore, OwnerTicket},
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
        let mut store = self.store.lock().map_err(|_| failure())?;
        let ticket = store
            .begin_owner(
                &self.ticket.binding,
                &self.ticket.owner_id,
                self.ticket.owner_epoch,
                at,
            )
            .map_err(|_| failure())?;
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
        if let Some(proof) = cessation {
            let next = store.query(&self.ticket.binding).map_err(|_| failure())?["sequence"]
                .as_u64()
                .ok_or_else(failure)?
                .checked_add(1)
                .ok_or_else(failure)?;
            store
                .append_cessation_with_exit(&self.ticket, next, proof, exit)
                .map_err(|_| failure())?;
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
