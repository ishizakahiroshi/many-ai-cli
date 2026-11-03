//! Independent durable admission journal for an explicitly authorized Iris peer.
//! No process launch, credentials, provider output, or arbitrary file reads live here.
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CONTRACT: &str = "many-ai-cli-iris-execution/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub principal_id: String,
    pub workspace_id: String,
    pub plan_id: String,
    pub plan_revision: u64,
    pub task_id: String,
    pub instruction_revision: u64,
    pub instruction_hash: String,
    pub approval_id: String,
    pub approval_revision: u64,
    pub scope_hash: String,
    pub execution_host_id: String,
    pub executor_instance_id: String,
    pub attempt_id: String,
    pub fence: u64,
    pub provider: String,
    pub profile_id: String,
}
/// Registration is a local Rust trust boundary, never an MCP operation. Dynamic
/// attempt/instance/fence values are omitted when the pinned scope is compared.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub binding: Binding,
    pub expires_at: i64,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalError {
    pub code: String,
    pub detail: String,
}
impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}
impl std::error::Error for JournalError {}
type Result<T> = std::result::Result<T, JournalError>;
#[derive(Debug, Clone, PartialEq)]
pub enum Acceptance {
    New(Value),
    Replay(Value),
}
fn error(code: &str) -> JournalError {
    JournalError {
        code: code.into(),
        detail: match code {
            "STORAGE" => "Execution state could not be safely read or persisted.",
            "OWNER_BUSY" => "Another owner holds the execution journal.",
            "NO_GRANT" | "STALE_GRANT" => {
                "No current independently registered grant covers this binding."
            }
            "REQUEST_CONFLICT" => "The request identity already belongs to different content.",
            "RESTORE_HOLD" => "Restored execution state is inspection-only.",
            _ => "The execution contract conditions were not satisfied.",
        }
        .into(),
    }
}
fn sql<T>(value: rusqlite::Result<T>) -> Result<T> {
    value.map_err(|_| error("STORAGE"))
}
fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn bounded(value: u64) -> bool {
    value > 0 && value <= i64::MAX as u64
}
fn validate_binding(b: &Binding) -> Result<()> {
    if [
        &b.principal_id,
        &b.workspace_id,
        &b.plan_id,
        &b.task_id,
        &b.approval_id,
        &b.execution_host_id,
        &b.executor_instance_id,
        &b.attempt_id,
        &b.provider,
        &b.profile_id,
    ]
    .iter()
    .any(|v| !valid_id(v))
        || ![
            b.plan_revision,
            b.instruction_revision,
            b.approval_revision,
            b.fence,
        ]
        .iter()
        .all(|v| bounded(*v))
        || !valid_hash(&b.instruction_hash)
        || !valid_hash(&b.scope_hash)
    {
        return Err(error("INVALID"));
    }
    Ok(())
}
fn scope(b: &Binding) -> Value {
    json!({"principal_id":b.principal_id,"workspace_id":b.workspace_id,"plan_id":b.plan_id,"plan_revision":b.plan_revision,"task_id":b.task_id,"instruction_revision":b.instruction_revision,"instruction_hash":b.instruction_hash,"approval_id":b.approval_id,"approval_revision":b.approval_revision,"scope_hash":b.scope_hash,"execution_host_id":b.execution_host_id,"provider":b.provider,"profile_id":b.profile_id})
}
fn decode(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| error("STORAGE"))
}
fn stored_binding(v: &Value) -> Result<Binding> {
    serde_json::from_value(v["binding"].clone()).map_err(|_| error("STORAGE"))
}
fn timestamp(v: &Value, key: &str) -> Result<i64> {
    v[key]
        .as_i64()
        .filter(|n| *n >= 0)
        .ok_or_else(|| error("INVALID_EVENT"))
}
fn wall_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

/// Tables are independent of SessionJournal/history pruning and async queues.
pub struct Journal {
    db: Connection,
    _owner: Connection,
    host: String,
    boot: String,
    restore_hold: bool,
}
impl Journal {
    /// Child-side read-only admission inspection. Opening another Journal would
    /// claim Hub ownership and recover rows, so a wrapper must never do that.
    /// This snapshot is only a prerequisite: OwnerStore's one-use spawn claim
    /// still has to commit immediately before native creation.
    pub fn inspect_dispatch(path: &Path, b: &Binding, now: i64) -> Result<Value> {
        validate_binding(b)?;
        let db = sql(Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ))?;
        sql(db.busy_timeout(Duration::from_secs(1)))?;
        sql(db.execute_batch("PRAGMA query_only=ON"))?;
        let tx = sql(db.unchecked_transaction())?;
        let version: i64 = sql(tx.query_row("PRAGMA user_version", [], |r| r.get(0)))?;
        if version != 1 || now < 0 {
            return Err(error("SCHEMA"));
        }
        let (host, boot, hold): (String, String, bool) = sql(tx.query_row(
            "SELECT host,boot,restore_hold FROM meta WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ))?;
        if hold {
            return Err(error("RESTORE_HOLD"));
        }
        if host != b.execution_host_id {
            return Err(error("HOST_MISMATCH"));
        }
        let raw: String = sql(tx.query_row(
            "SELECT document FROM attempts WHERE principal=? AND workspace=? AND attempt=?",
            params![b.principal_id, b.workspace_id, b.attempt_id],
            |r| r.get(0),
        ))?;
        let record =
            super::iris_execution_mcp::parse_json(raw.as_bytes()).map_err(|_| error("STORAGE"))?;
        validate_record(&record)?;
        if stored_binding(&record)? != *b
            || record["execution_state"] != "accepted"
            || record["accepted_boot"] != boot
        {
            return Err(error("STATE"));
        }
        let latest:i64=sql(tx.query_row("SELECT COALESCE(MAX(revision),0) FROM grants WHERE principal=? AND workspace=? AND approval=? AND plan=? AND task=?",params![b.principal_id,b.workspace_id,b.approval_id,b.plan_id,b.task_id],|r|r.get(0)))?;
        if latest != b.approval_revision as i64 {
            return Err(error("STALE_GRANT"));
        }
        let (pinned, expires, revoked):(String,i64,bool)=sql(tx.query_row("SELECT scope,expires,revoked FROM grants WHERE principal=? AND workspace=? AND approval=? AND revision=? AND plan=? AND task=?",params![b.principal_id,b.workspace_id,b.approval_id,b.approval_revision as i64,b.plan_id,b.task_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))))?;
        if revoked || expires <= now || decode(&pinned)? != scope(b) {
            return Err(error("STALE_GRANT"));
        }
        Ok(record)
    }
    pub fn open(path: &Path, host_id: &str, boot_id: &str) -> Result<Self> {
        Self::open_inner(path, host_id, boot_id, false)
    }
    pub fn open_restored(path: &Path, host_id: &str, boot_id: &str) -> Result<Self> {
        if !path.is_file() {
            return Err(error("NOT_FOUND"));
        }
        Self::open_inner(path, host_id, boot_id, true)
    }
    fn open_inner(path: &Path, host_id: &str, boot_id: &str, restored: bool) -> Result<Self> {
        if !valid_id(host_id) || !valid_id(boot_id) {
            return Err(error("INVALID"));
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|_| error("STORAGE"))?;
        let parent = parent.canonicalize().map_err(|_| error("STORAGE"))?;
        let path = parent.join(path.file_name().ok_or_else(|| error("INVALID"))?);
        let existed = path.exists();
        let path = if existed {
            path.canonicalize().map_err(|_| error("STORAGE"))?
        } else {
            path
        };
        if existed {
            single_link(&path)?;
        }
        let mut owner_name = path.as_os_str().to_os_string();
        owner_name.push(".owner.sqlite3");
        let owner = sql(Connection::open(std::path::PathBuf::from(owner_name)))?;
        sql(owner.busy_timeout(Duration::ZERO))?;
        owner
            .execute_batch("BEGIN EXCLUSIVE")
            .map_err(|_| error("OWNER_BUSY"))?;
        let mut db = sql(Connection::open(&path))?;
        sql(db.busy_timeout(Duration::from_millis(1000)))?;
        if sql(db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)))? != "ok" {
            return Err(error("STORAGE"));
        }
        let version = sql(db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)))?;
        if (existed && version != 1) || (!existed && version != 0) {
            return Err(error("SCHEMA"));
        }
        sql(db.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA journal_mode=WAL;",
        ))?;
        if !existed {
            let tx = sql(db.transaction())?;
            sql(tx.execute_batch("CREATE TABLE meta(id INTEGER PRIMARY KEY CHECK(id=1),host TEXT NOT NULL,boot TEXT NOT NULL,epoch INTEGER NOT NULL,restore_hold INTEGER NOT NULL CHECK(restore_hold IN(0,1))); CREATE TABLE grants(principal TEXT NOT NULL,workspace TEXT NOT NULL,approval TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),plan TEXT NOT NULL,task TEXT NOT NULL,scope TEXT NOT NULL,expires INTEGER NOT NULL,revoked INTEGER NOT NULL CHECK(revoked IN(0,1)),PRIMARY KEY(principal,workspace,approval,revision,plan,task)); CREATE TABLE attempts(principal TEXT NOT NULL,workspace TEXT NOT NULL,attempt TEXT NOT NULL,document TEXT NOT NULL,PRIMARY KEY(principal,workspace,attempt)); CREATE TABLE requests(principal TEXT NOT NULL,workspace TEXT NOT NULL,request TEXT NOT NULL,operation TEXT NOT NULL,payload TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(principal,workspace,request)); CREATE TABLE fences(principal TEXT NOT NULL,workspace TEXT NOT NULL,plan TEXT NOT NULL,task TEXT NOT NULL,fence INTEGER NOT NULL CHECK(fence>0),attempt TEXT NOT NULL,PRIMARY KEY(principal,workspace,plan,task),FOREIGN KEY(principal,workspace,attempt) REFERENCES attempts(principal,workspace,attempt)); CREATE TABLE events(principal TEXT NOT NULL,workspace TEXT NOT NULL,attempt TEXT NOT NULL,sequence INTEGER NOT NULL CHECK(sequence>0),payload TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(principal,workspace,attempt,sequence),FOREIGN KEY(principal,workspace,attempt) REFERENCES attempts(principal,workspace,attempt)); CREATE TABLE audit(id INTEGER PRIMARY KEY AUTOINCREMENT,kind TEXT NOT NULL,document TEXT NOT NULL); PRAGMA user_version=1;"))?;
            sql(tx.execute(
                "INSERT INTO meta VALUES(1,?,?,1,?)",
                params![host_id, boot_id, restored],
            ))?;
            sql(tx.commit())?;
        }
        let (host, _old_boot, epoch, old_hold): (String, String, i64, bool) = sql(db.query_row(
            "SELECT host,boot,epoch,restore_hold FROM meta WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ))?;
        if host != host_id || epoch <= 0 {
            return Err(error("HOST_MISMATCH"));
        }
        // Every open is a new lifetime, even if a caller accidentally reuses boot_id.
        let rows = sql(db.prepare("SELECT document FROM attempts"))?
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| error("STORAGE"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| error("STORAGE"))?;
        let mut recovered = Vec::new();
        for row in rows {
            let mut record = decode(&row)?;
            validate_record(&record)?;
            if stored_binding(&record)?.execution_host_id != host_id {
                return Err(error("STORAGE"));
            }
            if matches!(
                record["execution_state"].as_str(),
                Some("accepted" | "running" | "stop_requested" | "stop_unconfirmed")
            ) {
                record["execution_state"] = json!("unknown");
                record["outcome"] = json!("unknown");
                record["recovered_on_boot"] = json!(boot_id);
                let b = stored_binding(&record)?;
                record["latest_receipt"] = receipt(
                    "recovery",
                    boot_id,
                    &b,
                    "unknown",
                    "unknown",
                    record["event_seq"]
                        .as_u64()
                        .ok_or_else(|| error("STORAGE"))?,
                    Value::Null,
                );
                recovered.push(record);
            }
        }
        let hold = old_hold || restored;
        let tx = sql(db.transaction())?;
        sql(tx.execute(
            "UPDATE meta SET boot=?,epoch=?,restore_hold=? WHERE id=1",
            params![
                boot_id,
                epoch.checked_add(1).ok_or_else(|| error("STORAGE"))?,
                hold
            ],
        ))?;
        for record in recovered {
            save_record(&tx, &record)?;
            sql(tx.execute(
                "INSERT INTO audit(kind,document) VALUES('recovery',?)",
                [record.to_string()],
            ))?;
        }
        sql(tx.commit())?;
        Ok(Self {
            db,
            _owner: owner,
            host: host_id.into(),
            boot: boot_id.into(),
            restore_hold: hold,
        })
    }
    /// A trusted local caller pins the grant. A revoked revision cannot be reactivated.
    pub fn register_grant(&mut self, grant: Grant) -> Result<()> {
        validate_binding(&grant.binding)?;
        if self.restore_hold {
            return Err(error("RESTORE_HOLD"));
        }
        if grant.binding.execution_host_id != self.host || grant.expires_at < 0 {
            return Err(error("INVALID_GRANT"));
        }
        if (grant.binding.approval_revision as i64) < self.latest_revision(&grant.binding)? {
            return Err(error("STALE_GRANT"));
        }
        let b = &grant.binding;
        let current:Option<(String,i64,bool)>=sql(self.db.query_row("SELECT scope,expires,revoked FROM grants WHERE principal=? AND workspace=? AND approval=? AND revision=? AND plan=? AND task=?",params![b.principal_id,b.workspace_id,b.approval_id,b.approval_revision as i64,b.plan_id,b.task_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional())?;
        let pinned = scope(b);
        if let Some((s, expires, revoked)) = &current
            && (decode(s)? != pinned
                || *expires != grant.expires_at
                || (*revoked && !grant.revoked))
        {
            return Err(error("GRANT_CONFLICT"));
        }
        let tx = sql(self.db.transaction())?;
        sql(tx.execute("INSERT INTO grants VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(principal,workspace,approval,revision,plan,task) DO UPDATE SET revoked=excluded.revoked",params![b.principal_id,b.workspace_id,b.approval_id,b.approval_revision as i64,b.plan_id,b.task_id,pinned.to_string(),grant.expires_at,grant.revoked]))?;
        sql(tx.execute(
            "INSERT INTO audit(kind,document) VALUES('grant',?)",
            [
                json!({"scope":pinned,"expires_at":grant.expires_at,"revoked":grant.revoked})
                    .to_string(),
            ],
        ))?;
        sql(tx.commit())?;
        Ok(())
    }
    fn latest_revision(&self, b: &Binding) -> Result<i64> {
        sql(self.db.query_row("SELECT COALESCE(MAX(revision),0) FROM grants WHERE principal=? AND workspace=? AND approval=? AND plan=? AND task=?",params![b.principal_id,b.workspace_id,b.approval_id,b.plan_id,b.task_id],|r|r.get(0)))
    }
    fn authorize(&self, b: &Binding, now: i64) -> Result<()> {
        if now < 0 {
            return Err(error("INVALID"));
        }
        if b.execution_host_id != self.host {
            return Err(error("HOST_MISMATCH"));
        }
        let latest = self.latest_revision(b)?;
        if latest == 0 {
            return Err(error("NO_GRANT"));
        }
        if latest != b.approval_revision as i64 {
            return Err(error("STALE_GRANT"));
        }
        let row:Option<(String,i64,bool)>=sql(self.db.query_row("SELECT scope,expires,revoked FROM grants WHERE principal=? AND workspace=? AND approval=? AND revision=? AND plan=? AND task=?",params![b.principal_id,b.workspace_id,b.approval_id,b.approval_revision as i64,b.plan_id,b.task_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional())?;
        let Some((pinned, expires, revoked)) = row else {
            return Err(error("NO_GRANT"));
        };
        if revoked || expires <= now || decode(&pinned)? != scope(b) {
            return Err(error("STALE_GRANT"));
        }
        Ok(())
    }
    fn prior(&self, request: &str, b: &Binding, operation: &str) -> Result<Option<Value>> {
        let row:Option<(String,String,String)>=sql(self.db.query_row("SELECT operation,payload,receipt FROM requests WHERE principal=? AND workspace=? AND request=?",params![b.principal_id,b.workspace_id,request],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional())?;
        if let Some((op, payload, receipt)) = row {
            if op != operation || decode(&payload)? != json!(b) {
                return Err(error("REQUEST_CONFLICT"));
            }
            return Ok(Some(decode(&receipt)?));
        }
        Ok(None)
    }
    pub fn start(&mut self, request_id: &str, b: &Binding, now: i64) -> Result<Acceptance> {
        validate_binding(b)?;
        if !valid_id(request_id) {
            return Err(error("INVALID"));
        }
        if let Some(receipt) = self.prior(request_id, b, "start")? {
            return Ok(Acceptance::Replay(receipt));
        }
        if self.restore_hold {
            return Err(error("RESTORE_HOLD"));
        }
        self.authorize(b, now)?;
        if self.load(b)?.is_some() {
            return Err(error("ATTEMPT_CONFLICT"));
        }
        let previous:Option<(i64,String)>=sql(self.db.query_row("SELECT fence,attempt FROM fences WHERE principal=? AND workspace=? AND plan=? AND task=?",params![b.principal_id,b.workspace_id,b.plan_id,b.task_id],|r|Ok((r.get(0)?,r.get(1)?))).optional())?;
        if let Some((fence, attempt)) = previous {
            if b.fence <= fence as u64 {
                return Err(error("STALE_FENCE"));
            }
            let record = self
                .load_id(&b.principal_id, &b.workspace_id, &attempt)?
                .ok_or_else(|| error("STORAGE"))?;
            let old = stored_binding(&record)?;
            match record["outcome"].as_str() {
                Some("failed")
                    if record["effects"] == "none"
                        && record["execution_state"] == "stopped_confirmed" => {}
                Some("succeeded") if b.approval_revision > old.approval_revision => {}
                Some("rejected") => return Err(error("SAFETY_REFUSAL")),
                _ => return Err(error("OUTCOME_UNRESOLVED")),
            }
        }
        let receipt = receipt(
            "start",
            request_id,
            b,
            "accepted",
            "pending",
            0,
            json!(null),
        );
        let record = json!({"binding":b,"execution_state":"accepted","outcome":"pending","event_seq":0,"accepted_at":now,"accepted_boot":self.boot,"stop_request_id":null,"stop_requested_at":null,"effects":null,"evidence":null,"synthetic":null,"latest_receipt":receipt,"start_receipt":receipt});
        let tx = sql(self.db.transaction())?;
        save_record(&tx, &record)?;
        sql(tx.execute("INSERT INTO fences VALUES(?,?,?,?,?,?) ON CONFLICT(principal,workspace,plan,task) DO UPDATE SET fence=excluded.fence,attempt=excluded.attempt",params![b.principal_id,b.workspace_id,b.plan_id,b.task_id,b.fence as i64,b.attempt_id]))?;
        save_request(&tx, request_id, b, "start", &receipt)?;
        sql(tx.commit())?;
        Ok(Acceptance::New(receipt))
    }
    /// Revalidate an accepted intent immediately before the owned backend effect.
    /// The caller must keep its journal ownership lane through effect admission;
    /// a receipt is an identity, not a perpetual authorization capability.
    pub fn authorize_dispatch(&self, b: &Binding, now: i64) -> Result<()> {
        validate_binding(b)?;
        if self.restore_hold {
            return Err(error("RESTORE_HOLD"));
        }
        let record = self.bound(b)?;
        if record["execution_state"] != "accepted" || record["accepted_boot"] != self.boot {
            return Err(error("STATE"));
        }
        self.authorize(b, now)
    }
    pub fn query(&self, b: &Binding) -> Result<Value> {
        validate_binding(b)?;
        let record = self.bound(b)?;
        Ok(
            json!({"ok":true,"contract_version":CONTRACT,"binding":b,"execution_state":record["execution_state"],"outcome":record["outcome"],"event_seq":record["event_seq"],"synthetic":record["synthetic"],"restore_hold":self.restore_hold,"receipt":record["latest_receipt"],"start_receipt":record["start_receipt"],"stop_request_id":record["stop_request_id"],"stop_receipt":record["stop_receipt"],"evidence":record["evidence"],"proof_scope":if record["synthetic"]==true{"synthetic-only"}else{"unverified"}}),
        )
    }
    pub fn query_request(
        &self,
        principal: &str,
        workspace: &str,
        request_id: &str,
    ) -> Result<Value> {
        if !valid_id(principal) || !valid_id(workspace) || !valid_id(request_id) {
            return Err(error("INVALID"));
        }
        let raw: Option<String> = sql(self
            .db
            .query_row(
                "SELECT receipt FROM requests WHERE principal=? AND workspace=? AND request=?",
                params![principal, workspace, request_id],
                |r| r.get(0),
            )
            .optional())?;
        let receipt = decode(&raw.ok_or_else(|| error("NOT_FOUND"))?)?;
        let binding: Binding =
            serde_json::from_value(receipt["binding"].clone()).map_err(|_| error("STORAGE"))?;
        Ok(json!({"ok":true,"request_receipt":receipt,"current":self.query(&binding)?}))
    }
    pub fn stop(&mut self, request_id: &str, b: &Binding, now: i64) -> Result<Acceptance> {
        validate_binding(b)?;
        if !valid_id(request_id) || now < 0 {
            return Err(error("INVALID"));
        }
        if let Some(receipt) = self.prior(request_id, b, "stop")? {
            return Ok(Acceptance::Replay(receipt));
        }
        if self.restore_hold {
            return Err(error("RESTORE_HOLD"));
        }
        let mut record = self.bound(b)?;
        if !matches!(
            record["execution_state"].as_str(),
            Some("accepted" | "running" | "unknown" | "stop_unconfirmed")
        ) {
            return Err(error("STATE"));
        }
        if record["stop_request_id"].is_string() {
            return Err(error("STOP_ALREADY_REQUESTED"));
        }
        record["execution_state"] = json!("stop_requested");
        record["stop_request_id"] = json!(request_id);
        record["stop_requested_at"] = json!(now);
        let receipt = receipt(
            "stop",
            request_id,
            b,
            "stop_requested",
            record["outcome"].as_str().unwrap_or("unknown"),
            record["event_seq"].as_u64().unwrap_or(0),
            Value::Null,
        );
        record["latest_receipt"] = receipt.clone();
        let tx = sql(self.db.transaction())?;
        save_record(&tx, &record)?;
        save_request(&tx, request_id, b, "stop", &receipt)?;
        sql(tx.commit())?;
        Ok(Acceptance::New(receipt))
    }
    /// Only a trusted backend observer calls this method. The currently supported
    /// terminal proof is explicitly synthetic; no real provider completion is inferred.
    pub fn observe(&mut self, b: &Binding, event: Value) -> Result<Value> {
        validate_binding(b)?;
        if self.restore_hold {
            return Err(error("RESTORE_HOLD"));
        }
        if event.to_string().len() > 65536 || !event.is_object() {
            return Err(error("INVALID_EVENT"));
        }
        let fields = [
            "binding",
            "event_seq",
            "event_kind",
            "observed_at",
            "synthetic",
            "owned_execution_ceased",
            "outcome",
            "effects",
            "stop_request_id",
            "evidence",
        ];
        if event
            .as_object()
            .unwrap()
            .keys()
            .any(|k| !fields.contains(&k.as_str()))
            || event["binding"] != json!(b)
        {
            return Err(error("STALE_BINDING"));
        }
        let seq = event["event_seq"]
            .as_u64()
            .filter(|n| bounded(*n))
            .ok_or_else(|| error("INVALID_EVENT"))?;
        let kind = event["event_kind"]
            .as_str()
            .ok_or_else(|| error("INVALID_EVENT"))?;
        let observed = timestamp(&event, "observed_at")?;
        let mut record = self.bound(b)?;
        let old:Option<(String,String)>=sql(self.db.query_row("SELECT payload,receipt FROM events WHERE principal=? AND workspace=? AND attempt=? AND sequence=?",params![b.principal_id,b.workspace_id,b.attempt_id,seq as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional())?;
        if let Some((payload, receipt)) = old {
            if decode(&payload)? != event {
                return Err(error("RECEIPT_CONFLICT"));
            }
            return decode(&receipt);
        }
        let current = record["event_seq"]
            .as_u64()
            .ok_or_else(|| error("STORAGE"))?;
        if seq <= current {
            return Err(error("STALE_EVENT"));
        }
        if observed < timestamp(&record, "accepted_at")? || observed > wall_time() + 60 {
            return Err(error("INVALID_EVENT"));
        }
        let terminal = matches!(kind, "result" | "stop_observed");
        let proof = terminal && synthetic_proof(b, &event, kind == "stop_observed");
        if seq != current + 1
            && !(proof
                && matches!(
                    record["execution_state"].as_str(),
                    Some("unknown" | "stop_requested" | "stop_unconfirmed")
                ))
        {
            return Err(error("SEQUENCE_GAP"));
        }
        match kind {
            "accepted" | "running" => {
                if !matches!(
                    record["execution_state"].as_str(),
                    Some("accepted" | "running")
                ) || record["outcome"] != "pending"
                {
                    return Err(error("STATE"));
                }
                if kind == "running" {
                    record["execution_state"] = json!("running");
                }
            }
            "disconnected" => {
                if !matches!(
                    record["execution_state"].as_str(),
                    Some("accepted" | "running" | "stop_requested")
                ) {
                    return Err(error("STATE"));
                }
                record["execution_state"] = json!("unknown");
                record["outcome"] = json!("unknown");
            }
            "result" => {
                if !proof {
                    return Err(error("UNSUPPORTED_PROOF"));
                }
                if !matches!(
                    record["execution_state"].as_str(),
                    Some(
                        "accepted" | "running" | "unknown" | "stop_requested" | "stop_unconfirmed"
                    )
                ) {
                    return Err(error("STATE"));
                }
                record["execution_state"] = json!("stopped_confirmed");
                record["outcome"] = json!("succeeded");
                record["evidence"] = event["evidence"].clone();
                record["synthetic"] = json!(true);
            }
            "stop_observed" => {
                if !proof
                    || event["stop_request_id"] != record["stop_request_id"]
                    || !record["stop_request_id"].is_string()
                    || observed < timestamp(&record, "stop_requested_at")?
                {
                    return Err(error("INVALID_STOP_PROOF"));
                }
                if !matches!(
                    record["execution_state"].as_str(),
                    Some("stop_requested" | "stop_unconfirmed" | "unknown")
                ) {
                    return Err(error("STATE"));
                }
                record["execution_state"] = json!("stopped_confirmed");
                record["outcome"] = json!("failed");
                record["effects"] = json!("none");
                record["evidence"] = event["evidence"].clone();
                record["synthetic"] = json!(true);
            }
            _ => return Err(error("INVALID_EVENT")),
        }
        record["event_seq"] = json!(seq);
        let receipt = receipt(
            kind,
            &format!("event-{seq}"),
            b,
            record["execution_state"].as_str().unwrap_or("unknown"),
            record["outcome"].as_str().unwrap_or("unknown"),
            seq,
            event["synthetic"].clone(),
        );
        let mut receipt = receipt;
        receipt["evidence"] = event["evidence"].clone();
        receipt["observed_at"] = json!(observed);
        receipt["stop_request_id"] = event["stop_request_id"].clone();
        if seq != current + 1 {
            receipt["reconciled_gap"] = json!({"after_event_seq":current,"terminal_event_seq":seq});
        }
        if kind == "stop_observed" {
            record["stop_receipt"] = receipt.clone();
        }
        record["latest_receipt"] = receipt.clone();
        validate_record(&record)?;
        let tx = sql(self.db.transaction())?;
        save_record(&tx, &record)?;
        sql(tx.execute(
            "INSERT INTO events VALUES(?,?,?,?,?,?)",
            params![
                b.principal_id,
                b.workspace_id,
                b.attempt_id,
                seq as i64,
                event.to_string(),
                receipt.to_string()
            ],
        ))?;
        sql(tx.execute(
            "INSERT INTO audit(kind,document) VALUES('observe',?)",
            [receipt.to_string()],
        ))?;
        sql(tx.commit())?;
        Ok(receipt)
    }
    fn load_id(&self, principal: &str, workspace: &str, attempt: &str) -> Result<Option<Value>> {
        let raw: Option<String> = sql(self
            .db
            .query_row(
                "SELECT document FROM attempts WHERE principal=? AND workspace=? AND attempt=?",
                params![principal, workspace, attempt],
                |r| r.get(0),
            )
            .optional())?;
        raw.as_deref().map(decode).transpose()
    }
    fn load(&self, b: &Binding) -> Result<Option<Value>> {
        self.load_id(&b.principal_id, &b.workspace_id, &b.attempt_id)
    }
    fn bound(&self, b: &Binding) -> Result<Value> {
        let record = self.load(b)?.ok_or_else(|| error("NOT_FOUND"))?;
        validate_record(&record)?;
        if stored_binding(&record)? != *b {
            return Err(error("STALE_BINDING"));
        }
        Ok(record)
    }
}
fn receipt(
    op: &str,
    request: &str,
    b: &Binding,
    state: &str,
    outcome: &str,
    seq: u64,
    synthetic: Value,
) -> Value {
    json!({"ok":true,"contract_version":CONTRACT,"receipt_id":format!("{}-{}-{}",op,b.attempt_id,request),"request_id":request,"operation":op,"binding":b,"attempt_id":b.attempt_id,"fence":b.fence,"execution_state":state,"outcome":outcome,"event_seq":seq,"synthetic":synthetic,"proof_scope":if synthetic==true{"synthetic-only"}else{"unverified"}})
}
fn save_record(db: &Connection, record: &Value) -> Result<()> {
    validate_record(record)?;
    let b = stored_binding(record)?;
    sql(db.execute("INSERT INTO attempts VALUES(?,?,?,?) ON CONFLICT(principal,workspace,attempt) DO UPDATE SET document=excluded.document",params![b.principal_id,b.workspace_id,b.attempt_id,record.to_string()]))?;
    Ok(())
}
fn save_request(
    db: &Connection,
    request: &str,
    b: &Binding,
    op: &str,
    receipt: &Value,
) -> Result<()> {
    sql(db.execute(
        "INSERT INTO requests VALUES(?,?,?,?,?,?)",
        params![
            b.principal_id,
            b.workspace_id,
            request,
            op,
            json!(b).to_string(),
            receipt.to_string()
        ],
    ))?;
    sql(db.execute(
        "INSERT INTO audit(kind,document) VALUES(?,?)",
        params![op, receipt.to_string()],
    ))?;
    Ok(())
}
fn synthetic_proof(b: &Binding, event: &Value, stopped: bool) -> bool {
    if b.provider != "synthetic"
        || b.profile_id != "synthetic"
        || event["synthetic"] != true
        || event["owned_execution_ceased"] != true
        || event["evidence"]["kind"] != "synthetic-sha256"
    {
        return false;
    }
    let expected = if stopped {
        format!("synthetic:stopped:no-effects:{}:{}", b.attempt_id, b.fence)
    } else {
        format!(
            "synthetic:completed:{}:{}:{}",
            b.attempt_id, b.fence, b.instruction_hash
        )
    };
    event["evidence"]["payload"] == expected
        && event["evidence"]["sha256"] == digest(&expected)
        && if stopped {
            event["outcome"] == "failed" && event["effects"] == "none"
        } else {
            event["outcome"] == "succeeded"
        }
}
fn validate_record(record: &Value) -> Result<()> {
    let b = stored_binding(record)?;
    validate_binding(&b).map_err(|_| error("STORAGE"))?;
    if timestamp(record, "accepted_at").is_err()
        || record["event_seq"]
            .as_u64()
            .is_none_or(|n| n > i64::MAX as u64)
        || record["start_receipt"]["binding"] != json!(b)
        || record["latest_receipt"]["binding"] != json!(b)
        || record["latest_receipt"]["execution_state"] != record["execution_state"]
        || record["latest_receipt"]["outcome"] != record["outcome"]
    {
        return Err(error("STORAGE"));
    }
    match (
        record["execution_state"].as_str(),
        record["outcome"].as_str(),
    ) {
        (Some("accepted" | "running"), Some("pending")) => {}
        (Some("unknown"), Some("unknown")) => {}
        (Some("stop_requested" | "stop_unconfirmed"), Some("pending" | "unknown"))
            if record["stop_request_id"].is_string()
                && timestamp(record, "stop_requested_at").is_ok() => {}
        (Some("stopped_confirmed"), Some("succeeded")) => {
            let e = json!({"synthetic":record["synthetic"],"owned_execution_ceased":true,"outcome":"succeeded","evidence":record["evidence"]});
            if !synthetic_proof(&b, &e, false) {
                return Err(error("STORAGE"));
            }
        }
        (Some("stopped_confirmed"), Some("failed")) => {
            let e = json!({"synthetic":record["synthetic"],"owned_execution_ceased":true,"outcome":"failed","effects":record["effects"],"evidence":record["evidence"]});
            if !synthetic_proof(&b, &e, true) {
                return Err(error("STORAGE"));
            }
        }
        _ => return Err(error("STORAGE")),
    }
    Ok(())
}
#[cfg(windows)]
fn single_link(path: &Path) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = std::fs::File::open(path).map_err(|_| error("STORAGE"))?;
    // All-zero is a valid initial integer-only Windows structure. The owned File
    // keeps its read-only handle live and the API writes exactly this structure.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(error("STORAGE"));
    }
    if info.nNumberOfLinks != 1 {
        return Err(error("UNSAFE_DB_ALIAS"));
    }
    Ok(())
}
#[cfg(unix)]
fn single_link(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if std::fs::metadata(path)
        .map_err(|_| error("STORAGE"))?
        .nlink()
        != 1
    {
        return Err(error("UNSAFE_DB_ALIAS"));
    }
    Ok(())
}
#[cfg(not(any(windows, unix)))]
fn single_link(_path: &Path) -> Result<()> {
    Err(error("UNSUPPORTED_PLATFORM"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> Binding {
        Binding {
            principal_id: "iris".into(),
            workspace_id: "workspace".into(),
            plan_id: "plan".into(),
            plan_revision: 1,
            task_id: "task".into(),
            instruction_revision: 1,
            instruction_hash: digest("synthetic instruction"),
            approval_id: "approval".into(),
            approval_revision: 1,
            scope_hash: digest("pinned scope"),
            execution_host_id: "host".into(),
            executor_instance_id: "instance".into(),
            attempt_id: "attempt".into(),
            fence: 1,
            provider: "synthetic".into(),
            profile_id: "synthetic".into(),
        }
    }
    fn grant(j: &mut Journal, b: &Binding) {
        j.register_grant(Grant {
            binding: b.clone(),
            expires_at: 1000,
            revoked: false,
        })
        .unwrap();
    }
    fn event(b: &Binding, kind: &str, seq: u64) -> Value {
        json!({"binding":b,"event_seq":seq,"event_kind":kind,"observed_at":100})
    }
    fn terminal(b: &Binding, stop: bool, seq: u64) -> Value {
        let payload = if stop {
            format!("synthetic:stopped:no-effects:{}:{}", b.attempt_id, b.fence)
        } else {
            format!(
                "synthetic:completed:{}:{}:{}",
                b.attempt_id, b.fence, b.instruction_hash
            )
        };
        let mut e = event(b, if stop { "stop_observed" } else { "result" }, seq);
        e["synthetic"] = json!(true);
        e["owned_execution_ceased"] = json!(true);
        e["outcome"] = json!(if stop { "failed" } else { "succeeded" });
        e["evidence"] =
            json!({"kind":"synthetic-sha256","payload":payload,"sha256":digest(&payload)});
        if stop {
            e["stop_request_id"] = json!("stop");
            e["effects"] = json!("none");
        }
        e
    }
    #[test]
    fn independent_grant_and_payload_binding() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::open(&d.path().join("db"), "host", "boot").unwrap();
        let b = binding();
        assert_eq!(j.start("start", &b, 100).unwrap_err().code, "NO_GRANT");
        grant(&mut j, &b);
        let mut changed = b.clone();
        changed.profile_id = "another".into();
        assert_eq!(
            j.start("start", &changed, 100).unwrap_err().code,
            "STALE_GRANT"
        );
        assert!(matches!(
            j.start("start", &b, 100).unwrap(),
            Acceptance::New(_)
        ));
        assert!(matches!(
            j.start("start", &b, 110).unwrap(),
            Acceptance::Replay(_)
        ));
        changed = b.clone();
        changed.instruction_hash = digest("different");
        assert_eq!(
            j.start("start", &changed, 100).unwrap_err().code,
            "REQUEST_CONFLICT"
        );
    }
    #[test]
    fn restart_unknown_no_replay_and_overlap() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("db");
        let b = binding();
        let mut j = Journal::open(&p, "host", "boot").unwrap();
        grant(&mut j, &b);
        let original = j.start("start", &b, 100).unwrap();
        assert_eq!(
            Journal::open(&p, "host", "other").err().unwrap().code,
            "OWNER_BUSY"
        );
        drop(j);
        let mut j = Journal::open(&p, "host", "boot-two").unwrap();
        assert_eq!(j.query(&b).unwrap()["execution_state"], "unknown");
        if let Acceptance::New(v) = original {
            assert_eq!(j.start("start", &b, 101).unwrap(), Acceptance::Replay(v));
        }
        let mut next = b.clone();
        next.attempt_id = "next".into();
        next.fence = 2;
        assert_eq!(
            j.start("next-start", &next, 101).unwrap_err().code,
            "OUTCOME_UNRESOLVED"
        );
    }
    #[test]
    fn stop_and_synthetic_evidence_and_handoff() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::open(&d.path().join("db"), "host", "boot").unwrap();
        let b = binding();
        grant(&mut j, &b);
        j.start("start", &b, 100).unwrap();
        j.stop("stop", &b, 100).unwrap();
        let mut fake = terminal(&b, true, 1);
        fake["stop_request_id"] = json!("wrong");
        assert_eq!(j.observe(&b, fake).unwrap_err().code, "INVALID_STOP_PROOF");
        let e = terminal(&b, true, 1);
        let proof = j.observe(&b, e.clone()).unwrap();
        assert_eq!(j.observe(&b, e).unwrap(), proof);
        assert_eq!(proof["proof_scope"], "synthetic-only");
        let mut next = b.clone();
        next.attempt_id = "next".into();
        next.fence = 2;
        assert!(matches!(
            j.start("next-start", &next, 100).unwrap(),
            Acceptance::New(_)
        ));
    }
    #[test]
    fn terminal_hash_and_sequence_and_race() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::open(&d.path().join("db"), "host", "boot").unwrap();
        let b = binding();
        grant(&mut j, &b);
        j.start("start", &b, 100).unwrap();
        let mut fake = terminal(&b, false, 1);
        fake["evidence"]["sha256"] = json!(digest("forged"));
        assert_eq!(j.observe(&b, fake).unwrap_err().code, "UNSUPPORTED_PROOF");
        assert_eq!(
            j.observe(&b, event(&b, "running", 2)).unwrap_err().code,
            "SEQUENCE_GAP"
        );
        j.stop("stop", &b, 100).unwrap();
        j.observe(&b, terminal(&b, false, 1)).unwrap();
        assert_eq!(j.query(&b).unwrap()["outcome"], "succeeded");
    }
    #[test]
    fn recovery_terminal_gap_requires_full_proof() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("db");
        let b = binding();
        let mut j = Journal::open(&p, "host", "boot").unwrap();
        grant(&mut j, &b);
        j.start("start", &b, 100).unwrap();
        j.observe(&b, event(&b, "accepted", 1)).unwrap();
        drop(j);
        let mut j = Journal::open(&p, "host", "boot-two").unwrap();
        assert!(j.observe(&b, event(&b, "running", 3)).is_err());
        let mut forged = terminal(&b, false, 3);
        forged["binding"]["profile_id"] = json!("different");
        assert_eq!(j.observe(&b, forged).unwrap_err().code, "STALE_BINDING");
        let result = j.observe(&b, terminal(&b, false, 3)).unwrap();
        assert_eq!(result["reconciled_gap"]["after_event_seq"], 1);
    }
    #[test]
    fn expired_revoked_grant_and_restore_hold() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("db");
        let b = binding();
        let mut j = Journal::open(&p, "host", "boot").unwrap();
        grant(&mut j, &b);
        assert_eq!(
            j.start("expired", &b, 1000).unwrap_err().code,
            "STALE_GRANT"
        );
        j.register_grant(Grant {
            binding: b.clone(),
            expires_at: 1000,
            revoked: true,
        })
        .unwrap();
        assert_eq!(j.start("revoked", &b, 100).unwrap_err().code, "STALE_GRANT");
        assert_eq!(
            j.register_grant(Grant {
                binding: b.clone(),
                expires_at: 1000,
                revoked: false
            })
            .unwrap_err()
            .code,
            "GRANT_CONFLICT"
        );
        drop(j);
        let mut j = Journal::open_restored(&p, "host", "boot-restored").unwrap();
        assert_eq!(j.start("held", &b, 100).unwrap_err().code, "RESTORE_HOLD");
    }
    #[test]
    fn transaction_failure_does_not_accept() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::open(&d.path().join("db"), "host", "boot").unwrap();
        let b = binding();
        grant(&mut j, &b);
        j.db.execute_batch("CREATE TRIGGER fail_request BEFORE INSERT ON requests BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
        assert_eq!(j.start("start", &b, 100).unwrap_err().code, "STORAGE");
        assert_eq!(j.query(&b).unwrap_err().code, "NOT_FOUND");
        assert_eq!(
            j.db.query_row("SELECT COUNT(*) FROM fences", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        j.db.execute_batch("DROP TRIGGER fail_request").unwrap();
        assert!(matches!(
            j.start("start", &b, 100).unwrap(),
            Acceptance::New(_)
        ));
    }
    #[test]
    fn corrupt_future_and_false_terminal_fail_closed() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("db");
        let mut j = Journal::open(&p, "host", "boot").unwrap();
        let b = binding();
        grant(&mut j, &b);
        j.start("start", &b, 100).unwrap();
        let mut record = j.bound(&b).unwrap();
        record["execution_state"] = json!("stopped_confirmed");
        record["outcome"] = json!("succeeded");
        j.db.execute("UPDATE attempts SET document=?", [record.to_string()])
            .unwrap();
        drop(j);
        assert_eq!(
            Journal::open(&p, "host", "other").err().unwrap().code,
            "STORAGE"
        );
        let future = d.path().join("future");
        let db = Connection::open(&future).unwrap();
        db.execute_batch("PRAGMA user_version=99").unwrap();
        drop(db);
        assert_eq!(
            Journal::open(&future, "host", "boot").err().unwrap().code,
            "SCHEMA"
        );
    }
    #[test]
    fn synthetic_proof_never_confirms_real_provider() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::open(&d.path().join("db"), "host", "boot").unwrap();
        let mut b = binding();
        b.provider = "codex".into();
        b.profile_id = "registered-profile".into();
        grant(&mut j, &b);
        j.start("real-intent", &b, 100).unwrap();
        assert_eq!(
            j.observe(&b, terminal(&b, false, 1)).unwrap_err().code,
            "UNSUPPORTED_PROOF"
        );
        assert_eq!(j.query(&b).unwrap()["outcome"], "pending");
        let lookup = j.query_request("iris", "workspace", "real-intent").unwrap();
        assert_eq!(lookup["request_receipt"]["request_id"], "real-intent");
        assert_eq!(
            j.query_request("other", "workspace", "real-intent")
                .unwrap_err()
                .code,
            "NOT_FOUND"
        );
    }
    #[test]
    fn same_plan_approval_can_pin_two_independent_tasks() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::open(&d.path().join("db"), "host", "boot").unwrap();
        let a = binding();
        let mut b = a.clone();
        b.task_id = "task-two".into();
        b.attempt_id = "attempt-two".into();
        grant(&mut j, &a);
        grant(&mut j, &b);
        assert!(matches!(
            j.start("start-a", &a, 100).unwrap(),
            Acceptance::New(_)
        ));
        assert!(matches!(
            j.start("start-b", &b, 100).unwrap(),
            Acceptance::New(_)
        ));
        assert_eq!(
            j.db.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            j.db.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            j.db.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
    }
    #[test]
    fn dispatch_rechecks_expiry_and_revocation_after_admission() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::open(&d.path().join("db"), "host", "boot").unwrap();
        let b = binding();
        grant(&mut j, &b);
        j.start("start", &b, 100).unwrap();
        j.authorize_dispatch(&b, 100).unwrap();
        assert_eq!(
            j.authorize_dispatch(&b, 1000).unwrap_err().code,
            "STALE_GRANT"
        );
        j.register_grant(Grant {
            binding: b.clone(),
            expires_at: 1000,
            revoked: true,
        })
        .unwrap();
        assert_eq!(
            j.authorize_dispatch(&b, 100).unwrap_err().code,
            "STALE_GRANT"
        );
    }
    #[test]
    fn higher_grant_revision_invalidates_old_authority_without_breaking_replay() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::open(&d.path().join("db"), "host", "boot").unwrap();
        let old = binding();
        grant(&mut j, &old);
        let original = j.start("accepted-old", &old, 100).unwrap();
        let mut current = old.clone();
        current.approval_revision = 2;
        current.scope_hash = digest("new pinned scope");
        j.register_grant(Grant {
            binding: current.clone(),
            expires_at: 1000,
            revoked: false,
        })
        .unwrap();
        assert_eq!(
            j.start("new-request-old-authority", &old, 100)
                .unwrap_err()
                .code,
            "STALE_GRANT"
        );
        assert_eq!(
            j.authorize_dispatch(&old, 100).unwrap_err().code,
            "STALE_GRANT"
        );
        assert_eq!(
            j.register_grant(Grant {
                binding: old.clone(),
                expires_at: 1000,
                revoked: false
            })
            .unwrap_err()
            .code,
            "STALE_GRANT"
        );
        if let Acceptance::New(receipt) = original {
            assert_eq!(
                j.start("accepted-old", &old, 100).unwrap(),
                Acceptance::Replay(receipt)
            );
        }
        assert_eq!(
            j.query_request("iris", "workspace", "accepted-old")
                .unwrap()["request_receipt"]["binding"]["approval_revision"],
            1
        );
        j.register_grant(Grant {
            binding: current,
            expires_at: 1000,
            revoked: true,
        })
        .unwrap();
        assert_eq!(
            j.start("revoked-newer-no-fallback", &old, 100)
                .unwrap_err()
                .code,
            "STALE_GRANT"
        );
        assert_eq!(
            j.start("expired-newer-no-fallback", &old, 1001)
                .unwrap_err()
                .code,
            "STALE_GRANT"
        );
    }
}
