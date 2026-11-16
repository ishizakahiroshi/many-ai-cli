//! Fixed operator manifest. Local registration only; never an MCP grant endpoint.
//! Credentials live separately and are deliberately not opened by this module.
use super::{
    iris_execution_journal::{Binding, Grant},
    iris_execution_mcp::parse_json,
    iris_execution_owner::{
        ArtifactPermit, Result, check_binding, check_store_path, fail, relative_path, secure_open,
        sha, sql, static_binding,
    },
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
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

pub struct ApprovedGrant {
    pub grant: Grant,
    pub approved_cwd: PathBuf,
    pub instruction: ArtifactPermit,
    pub result: ArtifactPermit,
    pub effect_scope: Vec<String>,
    pub allow_remote_jobs: bool,
}
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
        let path = root.join("iris-execution-grant-trust.sqlite3");
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
        let mut file = secure_open(&self.root, Path::new("iris-execution-approved-grants.json"))?;
        if file.metadata().map_err(|_| fail("MANIFEST_READ"))?.len() > 65536 {
            return Err(fail("MANIFEST_LIMIT"));
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| fail("MANIFEST_READ"))?;
        let value = parse_json(&bytes).map_err(|_| fail("MANIFEST_JSON"))?;
        let manifest: Manifest =
            serde_json::from_value(value).map_err(|_| fail("MANIFEST_JSON"))?;
        if manifest.schema_version != 1
            || manifest.manifest_revision == 0
            || manifest.manifest_revision > i64::MAX as u64
            || manifest.approval_state != "approved"
            || manifest.grants.len() > 128
        {
            return Err(fail("MANIFEST_POLICY"));
        }
        let artifact_path = self.root.join("iris-execution-artifacts");
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
        if !artifacts.starts_with(&self.root) {
            return Err(fail("ARTIFACT_ROOT"));
        }
        let mut approved = Vec::new();
        let mut seen = BTreeSet::new();
        for e in &manifest.grants {
            check_binding(&e.binding)?;
            if e.expires_at < 0 || !seen.insert(scope_key(&e.binding)) {
                return Err(fail("MANIFEST_POLICY"));
            }
            let instruction = ArtifactPermit::approved(
                &artifacts,
                &e.instruction_path,
                &e.binding,
                e.max_artifact_bytes,
                e.effect_scope.clone(),
            )?;
            let result = ArtifactPermit::approved(
                &artifacts,
                &e.result_path,
                &e.binding,
                e.max_artifact_bytes,
                e.effect_scope.clone(),
            )?;
            if !e.revoked {
                instruction.verify(Some(&e.binding.instruction_hash))?;
            }
            let cwd = Path::new(&e.approved_cwd);
            let cwd = if cwd.is_absolute() {
                cwd.to_path_buf()
            } else {
                self.root.join(relative_path(&e.approved_cwd)?)
            };
            let cwd = cwd.canonicalize().map_err(|_| fail("APPROVED_CWD"))?;
            if !cwd.is_dir() {
                return Err(fail("APPROVED_CWD"));
            }
            approved.push(ApprovedGrant {
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
            });
        }
        let canonical = serde_json::to_vec(&manifest).map_err(|_| fail("MANIFEST_JSON"))?;
        let digest = sha(&canonical);
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
}
