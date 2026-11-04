//! Fixed operator manifest. Local registration only; never an MCP grant endpoint.
//! Credentials live separately and are deliberately not opened by this module.
use super::{
    iris_execution_journal::{Binding, Grant},
    iris_execution_mcp::parse_json,
    iris_execution_owned::real_bounded_binding,
    iris_execution_owner::{
        ArtifactPermit, Result, check_binding, check_store_path, fail, relative_path, secure_open,
        sha, sql, static_binding,
    },
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;
use serde_json::json;
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

/// Launch permission shape. `Default` is the existing behavior. `BoundedEdit`
/// exists only for the real bounded profile's single provider/profile pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    Default,
    BoundedEdit,
}
const BOUNDED_EDIT: &str = "bounded-edit";

pub struct ApprovedGrant {
    pub grant: Grant,
    pub approved_cwd: PathBuf,
    pub instruction: ArtifactPermit,
    pub result: ArtifactPermit,
    pub effect_scope: Vec<String>,
    pub allow_remote_jobs: bool,
    pub launch_mode: LaunchMode,
    /// Present only for the real bounded pair: the provider writes this file in
    /// approved_cwd and the trusted collector builds the result envelope.
    pub payload: Option<ArtifactPermit>,
    /// Present only for a real pair `collector: "folder-diff"` grant (real
    /// project profile): the full change list artifact beside the result. The
    /// trusted collector writes it once; it is never provider-written.
    pub changes: Option<ArtifactPermit>,
}
impl ApprovedGrant {
    /// True for a real project (`folder-diff`) collector grant.
    pub fn folder_diff(&self) -> bool {
        self.changes.is_some()
    }
}
const FOLDER_DIFF: &str = "folder-diff";
/// Suffix of the change list artifact beside the grant's result path.
pub const CHANGES_SUFFIX: &str = ".changes.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    manifest_revision: u64,
    approval_state: String,
    grants: Vec<Entry>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    binding: Binding,
    expires_at: i64,
    revoked: bool,
    approved_cwd: String,
    instruction_path: String,
    result_path: String,
    max_artifact_bytes: u64,
    effect_scope: Vec<String>,
    allow_remote_jobs: bool,
    // Optional and omitted when absent, so existing entries keep their exact
    // serialized bytes and pinned digests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    launch_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    collector: Option<String>,
}

fn folder_diff(e: &Entry) -> Result<bool> {
    match e.collector.as_deref() {
        None => Ok(false),
        Some(FOLDER_DIFF) => Ok(true),
        Some(_) => Err(fail("MANIFEST_POLICY")),
    }
}

/// No `.git` entry at the folder or any ancestor: the folder is not inside a
/// repository, so a provider's project lookup cannot reach above it.
fn no_repository_above(cwd: &Path) -> Result<()> {
    for ancestor in cwd.ancestors() {
        if std::fs::symlink_metadata(ancestor.join(".git")).is_ok() {
            return Err(fail("APPROVED_CWD"));
        }
    }
    Ok(())
}

/// Real project folders may neither repeat nor contain one another, and may
/// not overlap another real pair collector folder.
fn no_overlapping_folders(grants: &[ApprovedGrant]) -> Result<()> {
    for (i, a) in grants.iter().enumerate() {
        for b in &grants[i + 1..] {
            let collector = |g: &ApprovedGrant| g.payload.is_some() || g.folder_diff();
            if (a.folder_diff() || b.folder_diff())
                && collector(a)
                && collector(b)
                && (a.approved_cwd.starts_with(&b.approved_cwd)
                    || b.approved_cwd.starts_with(&a.approved_cwd))
            {
                return Err(fail("MANIFEST_POLICY"));
            }
        }
    }
    Ok(())
}

fn launch_mode(e: &Entry) -> Result<LaunchMode> {
    match e.launch_mode.as_deref() {
        None => Ok(LaunchMode::Default),
        Some(BOUNDED_EDIT) if real_bounded_binding(&e.binding) => Ok(LaunchMode::BoundedEdit),
        Some(_) => Err(fail("MANIFEST_POLICY")),
    }
}

/// Only the explicit runtime root chooses the manifest. No caller-supplied file
/// names, credentials or MCP payload can select the source of approval authority.
pub struct GrantLoader {
    root: PathBuf,
    db: Connection,
}
impl GrantLoader {
    pub fn open(runtime_root: &Path) -> Result<Self> {
        let root = runtime_root
            .canonicalize()
            .map_err(|_| fail("TRUST_ROOT"))?;
        if !root.is_dir() {
            return Err(fail("TRUST_ROOT"));
        }
        let path = root.join(TRUST_STORE);
        let existed = path.exists();
        if existed {
            check_store_path(&path)?;
        }
        let mut db = sql(Connection::open(path))?;
        sql(db.busy_timeout(Duration::from_millis(2500)))?;
        if sql(db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)))? != "ok" {
            return Err(fail("STORAGE"));
        }
        sql(db.execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;",
        ))?;
        let tx = sql(db.transaction_with_behavior(TransactionBehavior::Immediate))?;
        let version = sql(tx.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)))?;
        if version == 0 && !existed {
            sql(tx.execute_batch("CREATE TABLE manifests(revision INTEGER PRIMARY KEY CHECK(revision>0),digest TEXT NOT NULL); CREATE TABLE pins(scope_key TEXT PRIMARY KEY,revision INTEGER NOT NULL CHECK(revision>0),digest TEXT NOT NULL,entry TEXT NOT NULL); PRAGMA user_version=1;"))?;
        } else if version != 1 {
            return Err(fail("SCHEMA"));
        }
        sql(tx.commit())?;
        let rows = sql(db.prepare("SELECT entry,digest FROM pins"))?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|_| fail("STORAGE"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| fail("STORAGE"))?;
        for (raw, digest) in rows {
            let value = parse_json(raw.as_bytes()).map_err(|_| fail("STORAGE"))?;
            let entry: Entry = serde_json::from_value(value).map_err(|_| fail("STORAGE"))?;
            if entry_digest(&entry)? != digest {
                return Err(fail("STORAGE"));
            }
        }
        Ok(Self { root, db })
    }
    pub fn load_approved(&mut self) -> Result<Vec<ApprovedGrant>> {
        let manifest = read_manifest(&self.root)?;
        let artifacts = artifacts_root(&self.root)?;
        let mut approved = Vec::new();
        let mut seen = BTreeSet::new();
        for e in &manifest.grants {
            let launch_mode = entry_policy(e, &mut seen)?;
            approved.push(approve_entry(&self.root, &artifacts, e, launch_mode)?);
        }
        no_overlapping_folders(&approved)?;
        let digest = manifest_digest(&manifest)?;
        let tx = sql(self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        let latest: i64 = sql(tx.query_row(
            "SELECT COALESCE(MAX(revision),0) FROM manifests",
            [],
            |r| r.get(0),
        ))?;
        if manifest.manifest_revision < (latest as u64) {
            return Err(fail("MANIFEST_DOWNGRADE"));
        }
        let old: Option<String> = sql(tx
            .query_row(
                "SELECT digest FROM manifests WHERE revision=?",
                [manifest.manifest_revision as i64],
                |r| r.get(0),
            )
            .optional())?;
        if old.is_some_and(|old| old != digest) {
            return Err(fail("MANIFEST_CONFLICT"));
        }
        let known = sql(tx.prepare("SELECT scope_key FROM pins"))?
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| fail("STORAGE"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| fail("STORAGE"))?;
        // Omitting an old scope must not silently leave its durable journal grant
        // active. Require an explicit revoked entry; callers disable admission on error.
        if known.iter().any(|key| !seen.contains(key)) {
            return Err(fail("EXPLICIT_REVOCATION_REQUIRED"));
        }
        for entry in &manifest.grants {
            let key = scope_key(&entry.binding);
            let digest = entry_digest(entry)?;
            let prior: Option<(i64, String, String)> = sql(tx
                .query_row(
                    "SELECT revision,digest,entry FROM pins WHERE scope_key=?",
                    [&key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional())?;
            if let Some((revision, old_digest, raw)) = prior {
                if entry.binding.approval_revision < revision as u64 {
                    return Err(fail("GRANT_DOWNGRADE"));
                }
                if entry.binding.approval_revision == revision as u64 && old_digest != digest {
                    return Err(fail("GRANT_CONFLICT"));
                }
                let old: Entry = serde_json::from_str(&raw).map_err(|_| fail("STORAGE"))?;
                if old.revoked
                    && !entry.revoked
                    && entry.binding.approval_revision == old.binding.approval_revision
                {
                    return Err(fail("GRANT_CONFLICT"));
                }
            }
            sql(tx.execute("INSERT INTO pins VALUES(?,?,?,?) ON CONFLICT(scope_key) DO UPDATE SET revision=excluded.revision,digest=excluded.digest,entry=excluded.entry",params![key,entry.binding.approval_revision as i64,digest,serde_json::to_string(entry).map_err(|_|fail("MANIFEST_JSON"))?]))?;
        }
        sql(tx.execute(
            "INSERT OR IGNORE INTO manifests VALUES(?,?)",
            params![manifest.manifest_revision as i64, digest],
        ))?;
        sql(tx.commit())?;
        Ok(approved)
    }

    /// Read-only result permit of the one collector grant bound to `binding`
    /// (C10 observation path). Unlike `load_approved` it never writes the grant
    /// trust store and never inspects the files or folders of other entries.
    ///
    /// Kept from `load_approved`: the fixed manifest source, size, schema and
    /// policy checks over every entry (no file access), downgrade and
    /// same-revision tamper detection against the recorded manifest digests,
    /// explicit revocation of known scopes, and, for the bound entry only, the
    /// instruction hash, approved folder, payload and artifact root checks
    /// that build the identical permit. Added: the bound entry must equal the
    /// entry pinned at admission (same approval revision and digest), and
    /// must not be revoked or expired at `now_s`.
    pub fn pinned_result_permit(
        runtime_root: &Path,
        binding: &Binding,
        now_s: i64,
    ) -> Result<ArtifactPermit> {
        check_binding(binding)?;
        let root = runtime_root
            .canonicalize()
            .map_err(|_| fail("TRUST_ROOT"))?;
        if !root.is_dir() {
            return Err(fail("TRUST_ROOT"));
        }
        let manifest = read_manifest(&root)?;
        let mut seen = BTreeSet::new();
        let mut target = None;
        for e in &manifest.grants {
            let launch_mode = entry_policy(e, &mut seen)?;
            if e.payload_path.is_some() && static_binding(&e.binding) == static_binding(binding) {
                target = Some((e, launch_mode));
            }
        }
        let Some((entry, launch_mode)) = target else {
            return Err(fail("GRANT_UNAVAILABLE"));
        };
        let digest = manifest_digest(&manifest)?;
        // Admission state, read in one snapshot without any write. A missing
        // store means nothing was pinned; it is never created here.
        let path = root.join(TRUST_STORE);
        if !path.exists() {
            return Err(fail("GRANT_NOT_PINNED"));
        }
        check_store_path(&path)?;
        let mut db = sql(Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ))?;
        sql(db.busy_timeout(Duration::from_millis(2500)))?;
        sql(db.execute_batch("PRAGMA query_only=ON;"))?;
        if sql(db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)))? != "ok" {
            return Err(fail("STORAGE"));
        }
        let tx = sql(db.transaction_with_behavior(TransactionBehavior::Deferred))?;
        if sql(tx.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)))? != 1 {
            return Err(fail("SCHEMA"));
        }
        let latest: i64 = sql(tx.query_row(
            "SELECT COALESCE(MAX(revision),0) FROM manifests",
            [],
            |r| r.get(0),
        ))?;
        if manifest.manifest_revision < (latest as u64) {
            return Err(fail("MANIFEST_DOWNGRADE"));
        }
        let recorded: Option<String> = sql(tx
            .query_row(
                "SELECT digest FROM manifests WHERE revision=?",
                [manifest.manifest_revision as i64],
                |r| r.get(0),
            )
            .optional())?;
        if recorded.is_some_and(|old| old != digest) {
            return Err(fail("MANIFEST_CONFLICT"));
        }
        let pins = sql(tx.prepare("SELECT scope_key,revision,digest,entry FROM pins"))?
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| fail("STORAGE"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| fail("STORAGE"))?;
        drop(tx);
        let key = scope_key(&entry.binding);
        let mut pinned = None;
        for (scope, revision, pin_digest, raw) in pins {
            // Same integrity check `open` applies to every pin row.
            let value = parse_json(raw.as_bytes()).map_err(|_| fail("STORAGE"))?;
            let old: Entry = serde_json::from_value(value).map_err(|_| fail("STORAGE"))?;
            if entry_digest(&old)? != pin_digest {
                return Err(fail("STORAGE"));
            }
            if !seen.contains(&scope) {
                return Err(fail("EXPLICIT_REVOCATION_REQUIRED"));
            }
            if scope == key {
                pinned = Some((revision, pin_digest));
            }
        }
        let Some((revision, pin_digest)) = pinned else {
            return Err(fail("GRANT_NOT_PINNED"));
        };
        if revision != entry.binding.approval_revision as i64 || pin_digest != entry_digest(entry)?
        {
            return Err(fail("GRANT_PIN_MISMATCH"));
        }
        if entry.revoked {
            return Err(fail("GRANT_REVOKED"));
        }
        if entry.expires_at <= now_s {
            return Err(fail("GRANT_EXPIRED"));
        }
        let artifacts = artifacts_root(&root)?;
        Ok(approve_entry(&root, &artifacts, entry, launch_mode)?.result)
    }
}
const TRUST_STORE: &str = "iris-execution-grant-trust.sqlite3";
fn read_manifest(root: &Path) -> Result<Manifest> {
    let mut file = secure_open(root, Path::new("iris-execution-approved-grants.json"))?;
    if file.metadata().map_err(|_| fail("MANIFEST_READ"))?.len() > 65536 {
        return Err(fail("MANIFEST_LIMIT"));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("MANIFEST_READ"))?;
    let value = parse_json(&bytes).map_err(|_| fail("MANIFEST_JSON"))?;
    let manifest: Manifest = serde_json::from_value(value).map_err(|_| fail("MANIFEST_JSON"))?;
    if manifest.schema_version != 1
        || manifest.manifest_revision == 0
        || manifest.manifest_revision > i64::MAX as u64
        || manifest.approval_state != "approved"
        || manifest.grants.len() > 128
    {
        return Err(fail("MANIFEST_POLICY"));
    }
    Ok(manifest)
}
fn manifest_digest(manifest: &Manifest) -> Result<String> {
    let canonical = serde_json::to_vec(manifest).map_err(|_| fail("MANIFEST_JSON"))?;
    Ok(sha(&canonical))
}
fn artifacts_root(root: &Path) -> Result<PathBuf> {
    let artifact_path = root.join("iris-execution-artifacts");
    let meta = std::fs::symlink_metadata(&artifact_path).map_err(|_| fail("ARTIFACT_ROOT"))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(fail("ARTIFACT_ROOT"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(fail("ARTIFACT_ROOT"));
        }
    }
    let artifacts = artifact_path
        .canonicalize()
        .map_err(|_| fail("ARTIFACT_ROOT"))?;
    if !artifacts.starts_with(root) {
        return Err(fail("ARTIFACT_ROOT"));
    }
    Ok(artifacts)
}
/// Manifest policy of one entry; no file or folder is opened.
fn entry_policy(e: &Entry, seen: &mut BTreeSet<String>) -> Result<LaunchMode> {
    check_binding(&e.binding)?;
    if e.expires_at < 0 || !seen.insert(scope_key(&e.binding)) {
        return Err(fail("MANIFEST_POLICY"));
    }
    let launch_mode = launch_mode(e)?;
    // A collector grant is real-pair only, and automatic edits are
    // only allowed when the trusted folder comparison will run. The
    // real pair is collector-only: it never reaches the path that
    // adopts a provider-written result file as an outcome. The two
    // collectors (payload, folder-diff) are exclusive, and folder-diff
    // (real project) always runs with bounded-edit.
    let collected = e.payload_path.is_some();
    let diff = folder_diff(e)?;
    if (collected && diff)
        || (collected || diff) != real_bounded_binding(&e.binding)
        || (launch_mode == LaunchMode::BoundedEdit && !(collected || diff))
        || (diff && launch_mode != LaunchMode::BoundedEdit)
    {
        return Err(fail("MANIFEST_POLICY"));
    }
    Ok(launch_mode)
}
/// Instruction, approved folder and payload checks of one entry, and its permits.
fn approve_entry(
    root: &Path,
    artifacts: &Path,
    e: &Entry,
    launch_mode: LaunchMode,
) -> Result<ApprovedGrant> {
    let collected = e.payload_path.is_some();
    let diff = folder_diff(e)?;
    let permit = |root: &Path, path: &str| {
        if diff {
            ArtifactPermit::folder_diff(
                root,
                path,
                &e.binding,
                e.max_artifact_bytes,
                e.effect_scope.clone(),
            )
        } else if collected {
            ArtifactPermit::collected(
                root,
                path,
                &e.binding,
                e.max_artifact_bytes,
                e.effect_scope.clone(),
            )
        } else {
            ArtifactPermit::approved(
                root,
                path,
                &e.binding,
                e.max_artifact_bytes,
                e.effect_scope.clone(),
            )
        }
    };
    let instruction = permit(artifacts, &e.instruction_path)?;
    let result = permit(artifacts, &e.result_path)?;
    if !e.revoked {
        instruction.verify(Some(&e.binding.instruction_hash))?;
    }
    let cwd = Path::new(&e.approved_cwd);
    let cwd = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        root.join(relative_path(&e.approved_cwd)?)
    };
    let cwd = cwd.canonicalize().map_err(|_| fail("APPROVED_CWD"))?;
    if !cwd.is_dir() {
        return Err(fail("APPROVED_CWD"));
    }
    let payload = match &e.payload_path {
        // The compared folder must not hold or sit inside trusted
        // state: nothing at or below the runtime root (artifacts,
        // owner databases, grant manifests), and not an ancestor.
        Some(path) => {
            if root.starts_with(&cwd) || cwd.starts_with(root) || cwd.starts_with(artifacts) {
                return Err(fail("APPROVED_CWD"));
            }
            Some(permit(&cwd, path)?)
        }
        None => None,
    };
    let changes = if diff {
        // Same folder rules as the payload collector, plus no repository
        // at or above the folder. The change list lives beside the result.
        if root.starts_with(&cwd) || cwd.starts_with(root) || cwd.starts_with(artifacts) {
            return Err(fail("APPROVED_CWD"));
        }
        no_repository_above(&cwd)?;
        Some(ArtifactPermit::folder_diff(
            artifacts,
            &format!("{}{CHANGES_SUFFIX}", e.result_path),
            &e.binding,
            super::iris_execution_folder::MAX_CHANGES_BYTES,
            e.effect_scope.clone(),
        )?)
    } else {
        None
    };
    Ok(ApprovedGrant {
        grant: Grant {
            binding: e.binding.clone(),
            expires_at: e.expires_at,
            revoked: e.revoked,
        },
        approved_cwd: cwd,
        instruction,
        result,
        effect_scope: e.effect_scope.clone(),
        allow_remote_jobs: e.allow_remote_jobs,
        launch_mode,
        payload,
        changes,
    })
}
fn scope_key(b: &Binding) -> String {
    json!([
        b.principal_id,
        b.workspace_id,
        b.approval_id,
        b.plan_id,
        b.task_id
    ])
    .to_string()
}
fn entry_digest(e: &Entry) -> Result<String> {
    let mut v = serde_json::to_value(e).map_err(|_| fail("MANIFEST_JSON"))?;
    v["binding"] = static_binding(&e.binding);
    Ok(sha(v.to_string().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(root: &Path, revision: u64, approval_revision: u64) -> Value {
        let b = Binding {
            principal_id: "iris".into(),
            workspace_id: "workspace".into(),
            plan_id: "plan".into(),
            plan_revision: 1,
            task_id: "task".into(),
            instruction_revision: 1,
            instruction_hash: sha(b"instruction"),
            approval_id: "approval".into(),
            approval_revision,
            scope_hash: sha(b"scope"),
            execution_host_id: "host".into(),
            executor_instance_id: "instance".into(),
            attempt_id: "attempt".into(),
            fence: 1,
            provider: "synthetic".into(),
            profile_id: "synthetic".into(),
        };
        std::fs::create_dir_all(root.join("iris-execution-artifacts")).unwrap();
        std::fs::create_dir_all(root.join("workspace")).unwrap();
        std::fs::write(
            root.join("iris-execution-artifacts/instruction.txt"),
            b"instruction",
        )
        .unwrap();
        json!({"schema_version":1,"manifest_revision":revision,"approval_state":"approved","grants":[{"binding":b,"expires_at":10000000000i64,"revoked":false,"approved_cwd":"workspace","instruction_path":"instruction.txt","result_path":"result.json","max_artifact_bytes":65536,"effect_scope":["none"],"allow_remote_jobs":false}]})
    }
    fn write(root: &Path, value: &Value) {
        std::fs::write(
            root.join("iris-execution-approved-grants.json"),
            value.to_string(),
        )
        .unwrap();
    }
    #[test]
    fn trusted_fixed_source_and_instruction_tamper() {
        let d = tempfile::tempdir().unwrap();
        let v = manifest(d.path(), 1, 1);
        write(d.path(), &v);
        let mut l = GrantLoader::open(d.path()).unwrap();
        let grants = l.load_approved().unwrap();
        assert_eq!(grants.len(), 1);
        std::fs::write(
            d.path().join("iris-execution-artifacts/instruction.txt"),
            b"tamper",
        )
        .unwrap();
        assert_eq!(l.load_approved().err().unwrap().code, "ARTIFACT_HASH");
    }
    #[test]
    fn immutable_revisions_and_no_downgrade_across_restart() {
        let d = tempfile::tempdir().unwrap();
        let v1 = manifest(d.path(), 1, 1);
        write(d.path(), &v1);
        let mut l = GrantLoader::open(d.path()).unwrap();
        l.load_approved().unwrap();
        let v2 = manifest(d.path(), 2, 2);
        write(d.path(), &v2);
        l.load_approved().unwrap();
        drop(l);
        let mut l = GrantLoader::open(d.path()).unwrap();
        write(d.path(), &v1);
        assert_eq!(l.load_approved().err().unwrap().code, "MANIFEST_DOWNGRADE");
        let mut v3 = manifest(d.path(), 3, 1);
        write(d.path(), &v3);
        assert_eq!(l.load_approved().err().unwrap().code, "GRANT_DOWNGRADE");
        v3["grants"][0]["binding"]["approval_revision"] = json!(2);
        v3["grants"][0]["allow_remote_jobs"] = json!(true);
        write(d.path(), &v3);
        assert_eq!(l.load_approved().err().unwrap().code, "GRANT_CONFLICT");
    }
    #[test]
    fn duplicate_unknown_and_traversal_rejected() {
        let d = tempfile::tempdir().unwrap();
        let mut v = manifest(d.path(), 1, 1);
        v["extra"] = json!(true);
        write(d.path(), &v);
        let mut l = GrantLoader::open(d.path()).unwrap();
        assert_eq!(l.load_approved().err().unwrap().code, "MANIFEST_JSON");
        std::fs::write(
            d.path().join("iris-execution-approved-grants.json"),
            "{\"schema_version\":1,\"schema_version\":1}",
        )
        .unwrap();
        assert_eq!(l.load_approved().err().unwrap().code, "MANIFEST_JSON");
        let mut v = manifest(d.path(), 1, 1);
        v["grants"][0]["instruction_path"] = json!("../outside");
        write(d.path(), &v);
        assert_eq!(l.load_approved().err().unwrap().code, "ARTIFACT_PATH");
    }
    fn real(v: &mut Value) {
        v["grants"][0]["binding"]["provider"] =
            json!(super::super::iris_execution_owned::REAL_BOUNDED_PROVIDER);
        v["grants"][0]["binding"]["profile_id"] =
            json!(super::super::iris_execution_owned::REAL_BOUNDED_PROFILE_ID);
    }
    #[test]
    fn bounded_edit_is_only_for_the_real_pair_and_absent_mode_keeps_bytes() {
        let d = tempfile::tempdir().unwrap();
        let mut v = manifest(d.path(), 1, 1);
        write(d.path(), &v);
        let mut l = GrantLoader::open(d.path()).unwrap();
        let grants = l.load_approved().unwrap();
        assert_eq!(grants[0].launch_mode, LaunchMode::Default);
        let entry: Entry = serde_json::from_value(v["grants"][0].clone()).unwrap();
        let encoded = serde_json::to_value(&entry).unwrap();
        assert!(encoded.get("launch_mode").is_none());
        assert_eq!(encoded, v["grants"][0]);
        for (provider, profile, mode) in [
            ("synthetic", "synthetic", "bounded-edit"),
            ("codex", "codex", "bounded-edit"),
            (
                "codex",
                super::super::iris_execution_owned::REAL_BOUNDED_PROFILE_ID,
                "bounded-edit",
            ),
            (
                super::super::iris_execution_owned::REAL_BOUNDED_PROVIDER,
                "claude-other",
                "bounded-edit",
            ),
            (
                super::super::iris_execution_owned::REAL_BOUNDED_PROVIDER,
                super::super::iris_execution_owned::REAL_BOUNDED_PROFILE_ID,
                "acceptEdits",
            ),
            (
                super::super::iris_execution_owned::REAL_BOUNDED_PROVIDER,
                super::super::iris_execution_owned::REAL_BOUNDED_PROFILE_ID,
                "default",
            ),
        ] {
            let d = tempfile::tempdir().unwrap();
            let mut v = manifest(d.path(), 1, 1);
            v["grants"][0]["binding"]["provider"] = json!(provider);
            v["grants"][0]["binding"]["profile_id"] = json!(profile);
            v["grants"][0]["launch_mode"] = json!(mode);
            write(d.path(), &v);
            let mut l = GrantLoader::open(d.path()).unwrap();
            assert_eq!(
                l.load_approved().err().unwrap().code,
                "MANIFEST_POLICY",
                "{provider}/{profile}/{mode}"
            );
        }
        let d = tempfile::tempdir().unwrap();
        v = manifest(d.path(), 1, 1);
        real(&mut v);
        v["grants"][0]["launch_mode"] = json!("bounded-edit");
        // Automatic edits require the trusted folder comparison (payload).
        write(d.path(), &v);
        let mut l = GrantLoader::open(d.path()).unwrap();
        assert_eq!(l.load_approved().err().unwrap().code, "MANIFEST_POLICY");
        let outside = tempfile::tempdir().unwrap();
        collected(&mut v, outside.path());
        write(d.path(), &v);
        let grant = l.load_approved().unwrap().pop().unwrap();
        assert_eq!(grant.launch_mode, LaunchMode::BoundedEdit);
        assert!(grant.payload.is_some());
    }
    /// Collector grants compare a folder outside the runtime root.
    fn collected(v: &mut Value, outside: &Path) {
        let work = outside.join("workspace");
        std::fs::create_dir_all(&work).unwrap();
        v["grants"][0]["approved_cwd"] = json!(work.canonicalize().unwrap());
        v["grants"][0]["payload_path"] = json!("answer.json");
        v["grants"][0]["effect_scope"] = json!([
            super::super::iris_execution_folder::PAYLOAD_ONLY,
            super::super::iris_execution_folder::UNEXPECTED_CHANGE
        ]);
    }
    #[test]
    fn pinned_result_permit_is_read_only_and_checks_the_bound_entry() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut v = manifest(d.path(), 1, 1);
        real(&mut v);
        collected(&mut v, outside.path());
        write(d.path(), &v);
        let b: Binding = serde_json::from_value(v["grants"][0]["binding"].clone()).unwrap();
        let read = |now: i64| GrantLoader::pinned_result_permit(d.path(), &b, now);
        // Nothing admitted yet: refused, and the trust store is not created.
        assert_eq!(read(0).err().unwrap().code, "GRANT_NOT_PINNED");
        assert!(!d.path().join(TRUST_STORE).exists());
        let grant = GrantLoader::open(d.path())
            .unwrap()
            .load_approved()
            .unwrap()
            .pop()
            .unwrap();
        let permit = read(0).unwrap();
        assert_eq!(permit.approved_path(), grant.result.approved_path());
        assert_eq!(permit.binding(), grant.result.binding());
        assert_eq!(read(10000000000).err().unwrap().code, "GRANT_EXPIRED");
        // The bound entry's own instruction and folder are still checked.
        let instruction = d.path().join("iris-execution-artifacts/instruction.txt");
        std::fs::write(&instruction, b"tamper").unwrap();
        assert_eq!(read(0).err().unwrap().code, "ARTIFACT_HASH");
        std::fs::write(&instruction, b"instruction").unwrap();
        std::fs::remove_dir(outside.path().join("workspace")).unwrap();
        assert_eq!(read(0).err().unwrap().code, "APPROVED_CWD");
    }
    #[test]
    fn real_pair_without_collector_is_refused() {
        // The real pair never reaches the provider-written result path.
        let d = tempfile::tempdir().unwrap();
        let mut v = manifest(d.path(), 1, 1);
        real(&mut v);
        write(d.path(), &v);
        let mut l = GrantLoader::open(d.path()).unwrap();
        assert_eq!(
            l.load_approved().err().map(|e| e.code).as_deref(),
            Some("MANIFEST_POLICY")
        );
    }
    #[test]
    fn payload_grants_are_real_pair_only_with_exact_collected_scope() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut v = manifest(d.path(), 1, 1);
        real(&mut v);
        collected(&mut v, outside.path());
        write(d.path(), &v);
        let mut l = GrantLoader::open(d.path()).unwrap();
        let grant = l.load_approved().unwrap().pop().unwrap();
        assert_eq!(grant.launch_mode, LaunchMode::Default);
        assert!(grant.payload.is_some());
        let mut cases = Vec::new();
        for (provider, profile) in [("synthetic", "synthetic"), ("codex", "codex")] {
            let mut other = v.clone();
            other["grants"][0]["binding"]["provider"] = json!(provider);
            other["grants"][0]["binding"]["profile_id"] = json!(profile);
            cases.push((other, "MANIFEST_POLICY"));
        }
        // Existing non-collector grants cannot use the collector vocabulary.
        let mut plain = manifest(d.path(), 1, 1);
        plain["grants"][0]["effect_scope"] =
            json!([super::super::iris_execution_folder::PAYLOAD_ONLY]);
        cases.push((plain, "INVALID_PERMIT"));
        for scope in [
            json!(["none"]),
            json!([super::super::iris_execution_folder::PAYLOAD_ONLY]),
            json!([
                super::super::iris_execution_folder::PAYLOAD_ONLY,
                super::super::iris_execution_folder::UNEXPECTED_CHANGE,
                "none"
            ]),
        ] {
            let mut other = v.clone();
            other["grants"][0]["effect_scope"] = scope;
            cases.push((other, "INVALID_PERMIT"));
        }
        let mut artifacts_cwd = v.clone();
        artifacts_cwd["grants"][0]["approved_cwd"] = json!("iris-execution-artifacts");
        cases.push((artifacts_cwd, "APPROVED_CWD"));
        // "<root>" is re-anchored at each case's fresh runtime root.
        let mut root_cwd = v.clone();
        root_cwd["grants"][0]["approved_cwd"] = json!("<root>");
        cases.push((root_cwd, "APPROVED_CWD"));
        // Anywhere at or below the runtime root is refused (C4-D), including
        // owner databases and plain subfolders such as the old "workspace".
        for inside in ["iris-execution/inner", "workspace", "inside/nested"] {
            let mut inside_cwd = v.clone();
            inside_cwd["grants"][0]["approved_cwd"] = json!(inside);
            cases.push((inside_cwd, "APPROVED_CWD"));
        }
        for (index, (value, code)) in cases.into_iter().enumerate() {
            let d = tempfile::tempdir().unwrap();
            let fresh = manifest(d.path(), 1, 1);
            let mut value = value;
            if value["grants"][0]["approved_cwd"] == "<root>" {
                value["grants"][0]["approved_cwd"] = json!(d.path().canonicalize().unwrap());
            } else if let Some(relative) = value["grants"][0]["approved_cwd"]
                .as_str()
                .filter(|s| !Path::new(s).is_absolute())
            {
                std::fs::create_dir_all(d.path().join(relative)).unwrap();
            }
            value["grants"][0]["binding"]["instruction_hash"] =
                fresh["grants"][0]["binding"]["instruction_hash"].clone();
            write(d.path(), &value);
            let mut l = GrantLoader::open(d.path()).unwrap();
            assert_eq!(
                l.load_approved().err().map(|e| e.code),
                Some(code.to_string()),
                "case {index}"
            );
        }
    }
}
