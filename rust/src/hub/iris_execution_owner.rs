//! Trusted local owner evidence. This is a library API, never an HTTP/MCP writer.
//! File integrity, local process cessation and reported task outcome remain separate.
use super::{iris_execution_journal::Binding, iris_execution_mcp::parse_json};
use crate::process::pty::{LocalCessation, PtyExit};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone)]
pub struct OwnerError {
    pub code: String,
    pub detail: String,
}
impl std::fmt::Display for OwnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}
impl std::error::Error for OwnerError {}
pub(crate) fn fail(code: &str) -> OwnerError {
    OwnerError {
        code: code.into(),
        detail: "Trusted owner evidence conditions were not satisfied.".into(),
    }
}
pub(crate) type Result<T> = std::result::Result<T, OwnerError>;
pub(crate) fn sql<T>(r: rusqlite::Result<T>) -> Result<T> {
    r.map_err(|_| fail("STORAGE"))
}
pub(crate) fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(crate) fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub(crate) fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn check_binding(b: &Binding) -> Result<()> {
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
    .any(|s| !id(s))
        || [
            b.plan_revision,
            b.instruction_revision,
            b.approval_revision,
            b.fence,
        ]
        .iter()
        .any(|n| *n == 0 || *n > i64::MAX as u64)
        || !hash(&b.instruction_hash)
        || !hash(&b.scope_hash)
    {
        return Err(fail("INVALID_BINDING"));
    }
    Ok(())
}
pub(crate) fn static_binding(b: &Binding) -> Value {
    let mut v = json!(b);
    for k in ["attempt_id", "executor_instance_id", "fence"] {
        v.as_object_mut().unwrap().remove(k);
    }
    v
}
fn ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as u64
}

/// Identity only, not an authorization credential. Caller JSON cannot create a
/// native LocalCessation proof or an approved ArtifactPermit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerTicket {
    pub binding: Binding,
    pub owner_id: String,
    pub owner_epoch: u64,
}

#[derive(Clone)]
pub struct ArtifactPermit {
    root: PathBuf,
    relative: PathBuf,
    binding: Binding,
    max_bytes: u64,
    effects: Vec<String>,
}
pub struct VerifiedArtifact {
    bytes: Vec<u8>,
    sha256: String,
}
impl std::fmt::Debug for VerifiedArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedArtifact")
            .field("sha256", &self.sha256)
            .field("size", &self.bytes.len())
            .finish()
    }
}
impl VerifiedArtifact {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
}
impl ArtifactPermit {
    pub(crate) fn binding(&self) -> &Binding {
        &self.binding
    }
    /// Destination selected by the operator manifest; reads must still use verify.
    pub fn approved_path(&self) -> PathBuf {
        self.root.join(&self.relative)
    }
    pub(crate) fn approved(
        root: &Path,
        relative: &str,
        binding: &Binding,
        max_bytes: u64,
        effects: Vec<String>,
    ) -> Result<Self> {
        check_binding(binding)?;
        let relative = relative_path(relative)?;
        if max_bytes == 0
            || max_bytes > 1024 * 1024
            || effects.is_empty()
            || effects
                .iter()
                .any(|s| !matches!(s.as_str(), "none" | "local-files" | "unknown"))
        {
            return Err(fail("INVALID_PERMIT"));
        }
        let root = root.canonicalize().map_err(|_| fail("ARTIFACT_ROOT"))?;
        if !root.is_dir() {
            return Err(fail("ARTIFACT_ROOT"));
        }
        Ok(Self {
            root,
            relative,
            binding: binding.clone(),
            max_bytes,
            effects,
        })
    }
    pub fn verify(&self, expected_sha256: Option<&str>) -> Result<VerifiedArtifact> {
        if expected_sha256.is_some_and(|s| !hash(s)) {
            return Err(fail("ARTIFACT_HASH"));
        }
        let mut file = secure_open(&self.root, &self.relative)?;
        let before = file.metadata().map_err(|_| fail("ARTIFACT_READ"))?;
        if !before.is_file() || before.len() > self.max_bytes {
            return Err(fail("ARTIFACT_LIMIT"));
        }
        let mut bytes = Vec::with_capacity(before.len() as usize);
        file.by_ref()
            .take(self.max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| fail("ARTIFACT_READ"))?;
        let after = file.metadata().map_err(|_| fail("ARTIFACT_READ"))?;
        if bytes.len() as u64 > self.max_bytes
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || after.len() != bytes.len() as u64
        {
            return Err(fail("ARTIFACT_CHANGED"));
        }
        let actual = sha(&bytes);
        if expected_sha256.is_some_and(|s| s != actual) {
            return Err(fail("ARTIFACT_HASH"));
        }
        Ok(VerifiedArtifact {
            bytes,
            sha256: actual,
        })
    }
}
pub(crate) fn relative_path(raw: &str) -> Result<PathBuf> {
    if raw.is_empty() || raw.contains([':', '\0']) || raw.len() > 1024 {
        return Err(fail("ARTIFACT_PATH"));
    }
    let path = PathBuf::from(raw);
    for c in path.components() {
        let Component::Normal(name) = c else {
            return Err(fail("ARTIFACT_PATH"));
        };
        let s = name.to_str().ok_or_else(|| fail("ARTIFACT_PATH"))?;
        let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
        if s.ends_with(['.', ' '])
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(fail("ARTIFACT_PATH"));
        }
    }
    Ok(path)
}
pub(crate) fn secure_open(root: &Path, relative: &Path) -> Result<File> {
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(fail("ARTIFACT_PATH"));
        };
        cursor.push(name);
        let meta = std::fs::symlink_metadata(&cursor).map_err(|_| fail("ARTIFACT_NOT_FOUND"))?;
        if meta.file_type().is_symlink() {
            return Err(fail("ARTIFACT_ALIAS"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(fail("ARTIFACT_ALIAS"));
            }
        }
    }
    let canonical = cursor.canonicalize().map_err(|_| fail("ARTIFACT_PATH"))?;
    if !canonical.starts_with(root) {
        return Err(fail("ARTIFACT_PATH"));
    }
    secure_file(root, relative, &canonical)
}
#[cfg(windows)]
fn secure_file(root: &Path, _relative: &Path, canonical: &Path) -> Result<File> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        GetFileInformationByHandle, GetFinalPathNameByHandleW,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(canonical)
        .map_err(|_| fail("ARTIFACT_READ"))?;
    // Native structs are integer-only; the owned File retains the handle.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
        || info.nNumberOfLinks != 1
        || info.dwFileAttributes & 0x400 != 0
    {
        return Err(fail("ARTIFACT_ALIAS"));
    }
    let mut buffer = vec![0u16; 32768];
    let count = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    };
    if count == 0 || count as usize >= buffer.len() {
        return Err(fail("ARTIFACT_PATH"));
    }
    let final_path = PathBuf::from(
        String::from_utf16(&buffer[..count as usize]).map_err(|_| fail("ARTIFACT_PATH"))?,
    );
    if !final_path.starts_with(root) || final_path != canonical {
        return Err(fail("ARTIFACT_PATH"));
    }
    Ok(file)
}
#[cfg(unix)]
fn secure_file(root: &Path, relative: &Path, _canonical: &Path) -> Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{MetadataExt, OpenOptionsExt},
        },
    };
    let mut current = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(root)
        .map_err(|_| fail("ARTIFACT_ROOT"))?;
    let parts = relative.components().collect::<Vec<_>>();
    for (i, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            return Err(fail("ARTIFACT_PATH"));
        };
        let name = CString::new(name.as_encoded_bytes()).map_err(|_| fail("ARTIFACT_PATH"))?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if i + 1 < parts.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // openat resolves one validated component from an owned directory handle.
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(fail("ARTIFACT_ALIAS"));
        }
        current = unsafe { File::from_raw_fd(fd) };
    }
    if current
        .metadata()
        .map_err(|_| fail("ARTIFACT_READ"))?
        .nlink()
        != 1
    {
        return Err(fail("ARTIFACT_ALIAS"));
    }
    Ok(current)
}
#[cfg(not(any(windows, unix)))]
fn secure_file(_root: &Path, _relative: &Path, _canonical: &Path) -> Result<File> {
    Err(fail("UNSUPPORTED_PLATFORM"))
}
pub(crate) fn check_store_path(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| fail("STORAGE"))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(fail("STORE_ALIAS"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        if meta.file_attributes() & 0x400 != 0 {
            return Err(fail("STORE_ALIAS"));
        }
        let file = File::open(path).map_err(|_| fail("STORAGE"))?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
            || info.nNumberOfLinks != 1
        {
            return Err(fail("STORE_ALIAS"));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(fail("STORE_ALIAS"));
        }
    }
    Ok(())
}

/// Multiple trusted local processes may open this store. Every state change and
/// immutable receipt use one IMMEDIATE/FULL transaction. No startup execution.
pub struct OwnerStore {
    db: Connection,
}
impl OwnerStore {
    /// Hold the owner database writer lane until an external policy commit has
    /// completed. Another connection cannot create this intent between the
    /// absence check and durable policy pin. No owner schema is changed.
    pub(crate) fn before_owner_intent<T>(
        &mut self,
        binding: &Binding,
        commit_policy: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        check_binding(binding)?;
        let tx = sql(self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        if load(&tx, binding)?.is_some() {
            return Err(fail("VERIFIER_LATE_POLICY"));
        }
        let result = commit_policy()?;
        sql(tx.commit())?;
        Ok(result)
    }
    pub fn open(path: &Path) -> Result<Self> {
        let existed = path.exists();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| fail("STORAGE"))?;
        }
        if existed {
            check_store_path(path)?;
        }
        let mut db = sql(Connection::open(path))?;
        sql(db.busy_timeout(Duration::from_millis(2500)))?;
        if sql(db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)))? != "ok" {
            return Err(fail("STORAGE"));
        }
        sql(db.execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;",
        ))?;
        {
            let tx = sql(db.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let version = sql(tx.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)))?;
            if version == 0 && !existed {
                sql(tx.execute_batch("CREATE TABLE owners(principal TEXT NOT NULL,workspace TEXT NOT NULL,attempt TEXT NOT NULL,document TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(principal,workspace,attempt)); CREATE TABLE receipts(principal TEXT NOT NULL,workspace TEXT NOT NULL,attempt TEXT NOT NULL,sequence INTEGER NOT NULL CHECK(sequence>0),payload TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(principal,workspace,attempt,sequence),FOREIGN KEY(principal,workspace,attempt) REFERENCES owners(principal,workspace,attempt)); PRAGMA user_version=1;"))?;
            } else if version != 1 {
                return Err(fail("SCHEMA"));
            }
            sql(tx.commit())?;
        }
        let records = sql(db.prepare("SELECT document,digest FROM owners"))?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|_| fail("STORAGE"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| fail("STORAGE"))?;
        for (document, digest) in records {
            let record = checked_document(&document, &digest)?;
            validate_owner(&record)?;
            let ticket: OwnerTicket =
                serde_json::from_value(record["ticket"].clone()).map_err(|_| fail("STORAGE"))?;
            load(&db, &ticket.binding)?.ok_or_else(|| fail("STORAGE"))?;
        }
        let receipts = sql(db.prepare("SELECT payload,digest FROM receipts"))?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|_| fail("STORAGE"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| fail("STORAGE"))?;
        for (raw, digest) in receipts {
            let v = checked_document(&raw, &digest)?;
            if !v["sequence"].is_u64() || !v["ticket"].is_object() {
                return Err(fail("STORAGE"));
            }
        }
        Ok(Self { db })
    }
    /// Trusted application only: save intent before spawning the native owner.
    pub fn begin_owner(
        &mut self,
        b: &Binding,
        owner_id: &str,
        owner_epoch: u64,
        started_at_ms: u64,
    ) -> Result<OwnerTicket> {
        check_binding(b)?;
        if !id(owner_id)
            || owner_epoch == 0
            || owner_epoch > i64::MAX as u64
            || started_at_ms > ms() + 60000
        {
            return Err(fail("INVALID_OWNER"));
        }
        let ticket = OwnerTicket {
            binding: b.clone(),
            owner_id: owner_id.into(),
            owner_epoch,
        };
        let tx = sql(self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        if let Some(record) = load(&tx, b)? {
            if record["ticket"] != json!(ticket) {
                return Err(fail("OWNER_CONFLICT"));
            }
            return Ok(ticket);
        }
        let record = json!({"ticket":ticket,"phase":"intent","intent_at_ms":started_at_ms,"claimed_session":null,"claimed_at_ms":null,"root_pid":null,"root_started_at_ms":null,"sequence":0,"stop_request_id":null,"stop_requested_at_ms":null,"local_tree_ceased":false,"cessation_receipt":null,"outcome_receipt":null,"verified_task_outcome":"unknown","task_outcome_verified":false,"effects_verified":false});
        save(&tx, &record)?;
        sql(tx.commit())?;
        Ok(ticket)
    }
    /// One-use durable claim before factory.spawn. An interrupted claim is never
    /// replayed, even with the same session identity, after this method succeeds.
    pub fn claim_spawn(&mut self, t: &OwnerTicket, session_id: i64, at_ms: u64) -> Result<()> {
        let tx = sql(self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        let mut r = bound(&tx, t)?;
        if !r["claimed_session"].is_null() {
            return Err(fail("OWNER_ALREADY_CLAIMED"));
        }
        if session_id <= 0 || at_ms < r["intent_at_ms"].as_u64().unwrap() || at_ms > ms() + 60000 {
            return Err(fail("INVALID_OWNER"));
        }
        r["claimed_session"] = json!(session_id);
        r["claimed_at_ms"] = json!(at_ms);
        r["phase"] = json!("claimed");
        save(&tx, &r)?;
        sql(tx.commit())?;
        Ok(())
    }
    pub fn bind_root(
        &mut self,
        t: &OwnerTicket,
        root_pid: u32,
        started_at_ms: u64,
    ) -> Result<Value> {
        let tx = sql(self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        let mut r = bound(&tx, t)?;
        if r["claimed_session"].is_null()
            || root_pid == 0
            || started_at_ms < r["claimed_at_ms"].as_u64().unwrap_or(u64::MAX)
            || started_at_ms > ms() + 60000
        {
            return Err(fail("ROOT_BINDING"));
        }
        if !r["root_pid"].is_null() {
            if r["root_pid"] != root_pid || r["root_started_at_ms"] != started_at_ms {
                return Err(fail("ROOT_CONFLICT"));
            }
            return Ok(
                json!({"ticket":t,"root_pid":root_pid,"root_started_at_ms":started_at_ms,"process_started_verified":false}),
            );
        }
        r["root_pid"] = json!(root_pid);
        r["root_started_at_ms"] = json!(started_at_ms);
        r["phase"] = json!("root-bound");
        save(&tx, &r)?;
        sql(tx.commit())?;
        Ok(
            json!({"ticket":t,"root_pid":root_pid,"root_started_at_ms":started_at_ms,"process_started_verified":false}),
        )
    }
    pub fn request_stop(
        &mut self,
        t: &OwnerTicket,
        seq: u64,
        stop_request_id: &str,
        requested_at_ms: u64,
    ) -> Result<Value> {
        if !id(stop_request_id) {
            return Err(fail("STOP_BINDING"));
        }
        let payload = json!({"kind":"stop_requested","ticket":t,"sequence":seq,"stop_request_id":stop_request_id,"requested_at_ms":requested_at_ms});
        let tx = sql(self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        let mut r = bound(&tx, t)?;
        if let Some(v) = replay(&tx, t, seq, &payload)? {
            return Ok(v);
        }
        sequence(&r, seq)?;
        if r["root_pid"].is_null()
            || r["local_tree_ceased"] == true
            || r["stop_request_id"].is_string()
            || requested_at_ms < r["root_started_at_ms"].as_u64().unwrap_or(u64::MAX)
            || requested_at_ms > ms() + 60000
        {
            return Err(fail("STOP_BINDING"));
        }
        r["stop_request_id"] = json!(stop_request_id);
        r["stop_requested_at_ms"] = json!(requested_at_ms);
        r["sequence"] = json!(seq);
        persist_receipt(&tx, &r, &payload)?;
        sql(tx.commit())?;
        Ok(payload)
    }
    pub fn append_cessation(
        &mut self,
        t: &OwnerTicket,
        seq: u64,
        proof: &LocalCessation,
    ) -> Result<Value> {
        self.append_cessation_with_exit(t, seq, proof, None)
    }

    /// The typed exit observation is diagnostic only. Even code zero never
    /// changes task/effects acceptance or substitutes for native Job proof.
    pub fn append_cessation_with_exit(
        &mut self,
        t: &OwnerTicket,
        seq: u64,
        proof: &LocalCessation,
        exit: Option<&PtyExit>,
    ) -> Result<Value> {
        if exit.is_some_and(|e| {
            e.code < 0
                || e.code > u32::MAX as i64
                || e.signal.len() > 128
                || e.signal.chars().any(char::is_control)
        }) {
            return Err(fail("NATIVE_EXIT"));
        }
        let tx = sql(self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        let mut r = bound(&tx, t)?;
        let minimum = r["stop_requested_at_ms"]
            .as_u64()
            .or_else(|| r["root_started_at_ms"].as_u64())
            .unwrap_or(u64::MAX);
        if r["root_pid"] != proof.root_pid()
            || proof.scope() != "windows-owned-job"
            || !proof.root_waited()
            || proof.active_processes() != 0
            || proof.checked_at_unix_ms() < minimum
            || proof.checked_at_unix_ms() > ms() + 60000
        {
            return Err(fail("CESSATION_PROOF"));
        }
        let mut payload = json!({"kind":if r["stop_request_id"].is_null(){"local_exit"}else{"local_cessation"},"ticket":t,"sequence":seq,"stop_request_id":r["stop_request_id"],"root_pid":proof.root_pid(),"observed_at_ms":proof.checked_at_unix_ms(),"scope":proof.scope(),"local_tree_ceased":true,"owned_execution_ceased":false,"remote_jobs_stopped":false,"task_outcome_verified":false,"effects_verified":false});
        if let Some(exit) = exit {
            payload["process_exit"] = json!({"code":exit.code,"signal":exit.signal,"forced":exit.forced,"verification_scope":"native-exit-observation-only"});
        }
        if let Some(v) = replay(&tx, t, seq, &payload)? {
            return Ok(v);
        }
        sequence(&r, seq)?;
        r["local_tree_ceased"] = json!(true);
        r["cessation_receipt"] = payload.clone();
        r["sequence"] = json!(seq);
        r["phase"] = json!("local-tree-ceased");
        persist_receipt(&tx, &r, &payload)?;
        sql(tx.commit())?;
        Ok(payload)
    }
    pub fn append_outcome(
        &mut self,
        t: &OwnerTicket,
        seq: u64,
        permit: &ArtifactPermit,
        claimed_artifact_sha256: &str,
    ) -> Result<Value> {
        if static_binding(&t.binding) != static_binding(&permit.binding) {
            return Err(fail("ARTIFACT_BINDING"));
        }
        let verified = permit.verify(Some(claimed_artifact_sha256))?;
        let value = parse_json(verified.bytes()).map_err(|_| fail("ARTIFACT_JSON"))?;
        let outcome: OutcomeDocument =
            serde_json::from_value(value).map_err(|_| fail("ARTIFACT_JSON"))?;
        if outcome.binding != t.binding
            || outcome.owner_id != t.owner_id
            || outcome.owner_epoch != t.owner_epoch
            || outcome.instruction_hash != t.binding.instruction_hash
            || !matches!(
                outcome.outcome.as_str(),
                "succeeded" | "failed" | "rejected" | "unknown"
            )
            || !permit.effects.contains(&outcome.effects)
        {
            return Err(fail("ARTIFACT_BINDING"));
        }
        if outcome.synthetic
            && (t.binding.provider != "synthetic" || t.binding.profile_id != "synthetic")
        {
            return Err(fail("SYNTHETIC_FOR_REAL"));
        }
        let payload = json!({"kind":"artifact_outcome","ticket":t,"sequence":seq,"artifact_sha256":verified.sha256(),"artifact_bytes":verified.size(),"instruction_hash":outcome.instruction_hash,"reported_outcome":outcome.outcome,"reported_effects":outcome.effects,"synthetic":outcome.synthetic,"verification_scope":"artifact-integrity-only","task_outcome_verified":false,"effects_verified":false,"remote_jobs_stopped":false,"verified_task_outcome":"unknown"});
        let tx = sql(self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        let mut r = bound(&tx, t)?;
        if let Some(v) = replay(&tx, t, seq, &payload)? {
            return Ok(v);
        }
        sequence(&r, seq)?;
        if r["root_pid"].is_null() {
            return Err(fail("ROOT_BINDING"));
        }
        if !r["outcome_receipt"].is_null() {
            return Err(fail("OUTCOME_CONFLICT"));
        }
        r["outcome_receipt"] = payload.clone();
        r["sequence"] = json!(seq);
        persist_receipt(&tx, &r, &payload)?;
        sql(tx.commit())?;
        Ok(payload)
    }
    pub fn query(&self, b: &Binding) -> Result<Value> {
        check_binding(b)?;
        load(&self.db, b)?.ok_or_else(|| fail("NOT_FOUND"))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeDocument {
    binding: Binding,
    owner_id: String,
    owner_epoch: u64,
    instruction_hash: String,
    outcome: String,
    effects: String,
    synthetic: bool,
    // Optional data is integrity-bound only. Independent comparison is stored
    // separately and never changes this owner's task/effects authority flags.
    #[serde(default, rename = "payload")]
    _payload: Option<Value>,
}
fn checked_document(raw: &str, digest: &str) -> Result<Value> {
    if sha(raw.as_bytes()) != digest {
        return Err(fail("STORAGE"));
    }
    parse_json(raw.as_bytes()).map_err(|_| fail("STORAGE"))
}
fn validate_owner(v: &Value) -> Result<()> {
    let t: OwnerTicket =
        serde_json::from_value(v["ticket"].clone()).map_err(|_| fail("STORAGE"))?;
    check_binding(&t.binding).map_err(|_| fail("STORAGE"))?;
    if !id(&t.owner_id)
        || t.owner_epoch == 0
        || !v["sequence"].is_u64()
        || !v["intent_at_ms"].is_u64()
        || v["verified_task_outcome"] != "unknown"
        || v["task_outcome_verified"] != false
        || v["effects_verified"] != false
    {
        return Err(fail("STORAGE"));
    }
    match v["phase"].as_str() {
        Some("intent")
            if v["claimed_session"].is_null() && v["root_pid"].is_null() && v["sequence"] == 0 => {}
        Some("claimed")
            if v["claimed_session"].as_i64().is_some_and(|n| n > 0)
                && v["claimed_at_ms"]
                    .as_u64()
                    .is_some_and(|n| n >= v["intent_at_ms"].as_u64().unwrap())
                && v["root_pid"].is_null() => {}
        Some("root-bound" | "local-tree-ceased")
            if v["claimed_session"].as_i64().is_some_and(|n| n > 0)
                && v["root_pid"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= u32::MAX as u64)
                && v["root_started_at_ms"]
                    .as_u64()
                    .is_some_and(|n| n >= v["claimed_at_ms"].as_u64().unwrap_or(u64::MAX)) => {}
        _ => return Err(fail("STORAGE")),
    }
    if v["local_tree_ceased"] == true
        && (v["cessation_receipt"]["local_tree_ceased"] != true
            || v["cessation_receipt"]["ticket"] != v["ticket"]
            || v["cessation_receipt"]["root_pid"] != v["root_pid"]
            || v["cessation_receipt"]["owned_execution_ceased"] != false)
    {
        return Err(fail("STORAGE"));
    }
    Ok(())
}
fn load(db: &Connection, b: &Binding) -> Result<Option<Value>> {
    if db.is_autocommit() {
        let tx = sql(db.unchecked_transaction())?;
        let result = load_snapshot(&tx, b)?;
        sql(tx.commit())?;
        Ok(result)
    } else {
        load_snapshot(db, b)
    }
}
fn load_snapshot(db: &Connection, b: &Binding) -> Result<Option<Value>> {
    let row: Option<(String, String)> = sql(db
        .query_row(
            "SELECT document,digest FROM owners WHERE principal=? AND workspace=? AND attempt=?",
            params![b.principal_id, b.workspace_id, b.attempt_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional())?;
    let Some((raw, digest)) = row else {
        return Ok(None);
    };
    let v = checked_document(&raw, &digest)?;
    validate_owner(&v)?;
    if v["ticket"]["binding"] != json!(b) {
        return Err(fail("STALE_BINDING"));
    }
    let (count,max):(i64,i64)=sql(db.query_row("SELECT COUNT(*),COALESCE(MAX(sequence),0) FROM receipts WHERE principal=? AND workspace=? AND attempt=?",params![b.principal_id,b.workspace_id,b.attempt_id],|r|Ok((r.get(0)?,r.get(1)?))))?;
    if count != max || v["sequence"].as_u64() != Some(max as u64) {
        return Err(fail("STORAGE"));
    }
    Ok(Some(v))
}
fn bound(db: &Connection, t: &OwnerTicket) -> Result<Value> {
    let v = load(db, &t.binding)?.ok_or_else(|| fail("NOT_FOUND"))?;
    if v["ticket"] != json!(t) {
        return Err(fail("OWNER_CONFLICT"));
    }
    Ok(v)
}
fn save(db: &Connection, r: &Value) -> Result<()> {
    validate_owner(r)?;
    let t: OwnerTicket =
        serde_json::from_value(r["ticket"].clone()).map_err(|_| fail("STORAGE"))?;
    let raw = r.to_string();
    sql(db.execute("INSERT INTO owners VALUES(?,?,?,?,?) ON CONFLICT(principal,workspace,attempt) DO UPDATE SET document=excluded.document,digest=excluded.digest",params![t.binding.principal_id,t.binding.workspace_id,t.binding.attempt_id,raw,sha(raw.as_bytes())]))?;
    Ok(())
}
fn sequence(r: &Value, seq: u64) -> Result<()> {
    if seq == 0
        || seq > i64::MAX as u64
        || r["sequence"].as_u64().and_then(|n| n.checked_add(1)) != Some(seq)
    {
        return Err(fail("SEQUENCE"));
    }
    Ok(())
}
fn replay(db: &Connection, t: &OwnerTicket, seq: u64, payload: &Value) -> Result<Option<Value>> {
    if seq == 0 || seq > i64::MAX as u64 {
        return Err(fail("SEQUENCE"));
    }
    let row:Option<(String,String)>=sql(db.query_row("SELECT payload,digest FROM receipts WHERE principal=? AND workspace=? AND attempt=? AND sequence=?",params![t.binding.principal_id,t.binding.workspace_id,t.binding.attempt_id,seq as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional())?;
    let Some((raw, digest)) = row else {
        return Ok(None);
    };
    let old = checked_document(&raw, &digest)?;
    if old != *payload {
        return Err(fail("RECEIPT_CONFLICT"));
    }
    Ok(Some(old))
}
fn persist_receipt(db: &Connection, r: &Value, payload: &Value) -> Result<()> {
    save(db, r)?;
    let t: OwnerTicket =
        serde_json::from_value(r["ticket"].clone()).map_err(|_| fail("STORAGE"))?;
    let raw = payload.to_string();
    sql(db.execute(
        "INSERT INTO receipts VALUES(?,?,?,?,?,?)",
        params![
            t.binding.principal_id,
            t.binding.workspace_id,
            t.binding.attempt_id,
            payload["sequence"].as_u64().unwrap() as i64,
            raw,
            sha(raw.as_bytes())
        ],
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn binding() -> Binding {
        Binding {
            principal_id: "iris".into(),
            workspace_id: "workspace".into(),
            plan_id: "plan".into(),
            plan_revision: 1,
            task_id: "task".into(),
            instruction_revision: 1,
            instruction_hash: sha(b"instruction"),
            approval_id: "approval".into(),
            approval_revision: 1,
            scope_hash: sha(b"scope"),
            execution_host_id: "host".into(),
            executor_instance_id: "instance".into(),
            attempt_id: "attempt".into(),
            fence: 1,
            provider: "synthetic".into(),
            profile_id: "synthetic".into(),
        }
    }
    #[test]
    fn durable_multi_connection_identity_and_seq() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("owner.db");
        let mut a = OwnerStore::open(&p).unwrap();
        let b = binding();
        let t = a.begin_owner(&b, "owner", 1, 100).unwrap();
        a.claim_spawn(&t, 1, 100).unwrap();
        a.bind_root(&t, 123, 100).unwrap();
        let mut second = OwnerStore::open(&p).unwrap();
        let receipt = second.request_stop(&t, 1, "stop", 100).unwrap();
        assert_eq!(a.request_stop(&t, 1, "stop", 100).unwrap(), receipt);
        assert_eq!(
            a.request_stop(&t, 1, "other", 100).unwrap_err().code,
            "RECEIPT_CONFLICT"
        );
        drop(a);
        drop(second);
        let store = OwnerStore::open(&p).unwrap();
        assert_eq!(store.query(&b).unwrap()["stop_request_id"], "stop");
        assert_eq!(store.query(&b).unwrap()["local_tree_ceased"], false);
    }
    #[test]
    fn artifact_binding_hash_and_no_success_inference() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let b = binding();
        let mut s = OwnerStore::open(&d.path().join("owner.db")).unwrap();
        let t = s.begin_owner(&b, "owner", 1, 100).unwrap();
        s.claim_spawn(&t, 1, 100).unwrap();
        s.bind_root(&t, 123, 100).unwrap();
        let body=json!({"binding":b,"owner_id":"owner","owner_epoch":1,"instruction_hash":b.instruction_hash,"outcome":"succeeded","effects":"none","synthetic":true}).to_string();
        std::fs::write(d.path().join("result.json"), &body).unwrap();
        let permit =
            ArtifactPermit::approved(&root, "result.json", &b, 65536, vec!["none".into()]).unwrap();
        assert_eq!(
            s.append_outcome(&t, 1, &permit, &sha(b"wrong"))
                .unwrap_err()
                .code,
            "ARTIFACT_HASH"
        );
        let receipt = s
            .append_outcome(&t, 1, &permit, &sha(body.as_bytes()))
            .unwrap();
        assert_eq!(receipt["reported_outcome"], "succeeded");
        assert_eq!(receipt["task_outcome_verified"], false);
        assert_eq!(s.query(&b).unwrap()["verified_task_outcome"], "unknown");
    }
    #[test]
    fn traversal_hardlink_and_file_tamper_refused() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let b = binding();
        assert!(
            ArtifactPermit::approved(&root, "../outside", &b, 100, vec!["none".into()]).is_err()
        );
        std::fs::write(d.path().join("one"), b"instruction").unwrap();
        let p = ArtifactPermit::approved(&root, "one", &b, 100, vec!["none".into()]).unwrap();
        p.verify(Some(&b.instruction_hash)).unwrap();
        std::fs::write(d.path().join("one"), b"changed").unwrap();
        assert_eq!(
            p.verify(Some(&b.instruction_hash)).unwrap_err().code,
            "ARTIFACT_HASH"
        );
        std::fs::hard_link(d.path().join("one"), d.path().join("alias")).unwrap();
        assert_eq!(p.verify(None).unwrap_err().code, "ARTIFACT_ALIAS");
    }
    #[test]
    fn corrupt_db_refused_without_reinitialization() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("owner.db");
        let mut s = OwnerStore::open(&path).unwrap();
        s.begin_owner(&binding(), "owner", 1, 100).unwrap();
        s.db.execute("UPDATE owners SET document='{}'", []).unwrap();
        drop(s);
        assert_eq!(OwnerStore::open(&path).err().unwrap().code, "STORAGE");
    }
    #[test]
    fn one_use_claim_survives_restart_and_races() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("owner.db");
        let mut a = OwnerStore::open(&p).unwrap();
        let t = a.begin_owner(&binding(), "owner", 1, 100).unwrap();
        let mut b = OwnerStore::open(&p).unwrap();
        a.claim_spawn(&t, 8, 100).unwrap();
        assert_eq!(
            b.claim_spawn(&t, 8, 100).unwrap_err().code,
            "OWNER_ALREADY_CLAIMED"
        );
        drop(a);
        drop(b);
        let mut c = OwnerStore::open(&p).unwrap();
        assert_eq!(c.begin_owner(&t.binding, "owner", 1, 100).unwrap(), t);
        assert_eq!(
            c.claim_spawn(&t, 9, 101).unwrap_err().code,
            "OWNER_ALREADY_CLAIMED"
        );
    }
    #[test]
    fn policy_commit_guard_excludes_other_owner_writers() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("owner.db");
        let mut first = OwnerStore::open(&path).unwrap();
        let mut second = OwnerStore::open(&path).unwrap();
        second.db.busy_timeout(Duration::from_millis(30)).unwrap();
        first
            .before_owner_intent(&binding(), || {
                assert!(second.begin_owner(&binding(), "owner", 1, 100).is_err());
                Ok(())
            })
            .unwrap();
        second.begin_owner(&binding(), "owner", 1, 100).unwrap();
        assert_eq!(
            first
                .before_owner_intent(&binding(), || Ok(()))
                .unwrap_err()
                .code,
            "VERIFIER_LATE_POLICY"
        );
    }
    #[test]
    fn concurrent_claims_have_exactly_one_winner() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("owner.db");
        let mut initial = OwnerStore::open(&p).unwrap();
        let t = initial.begin_owner(&binding(), "owner", 1, 100).unwrap();
        drop(initial);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut workers = Vec::new();
        for session in 1..=2 {
            let path = p.clone();
            let ticket = t.clone();
            let gate = barrier.clone();
            workers.push(std::thread::spawn(move || {
                let mut store = OwnerStore::open(&path).unwrap();
                gate.wait();
                store.claim_spawn(&ticket, session, 100).map_err(|e| e.code)
            }));
        }
        let results = workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| r.as_ref().is_err_and(|e| e == "OWNER_ALREADY_CLAIMED"))
                .count(),
            1
        );
    }
}
