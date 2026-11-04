//! Trusted local fixed-payload acceptance records, independent of execution.
//! No HTTP writer, launch permission, task outcome promotion or effects proof.
use super::{
    iris_execution_journal::Binding,
    iris_execution_mcp::parse_json,
    iris_execution_owner::{
        ArtifactPermit, OwnerStore, OwnerTicket, Result, check_binding, check_store_path, fail,
        hash, id, sha, sql,
    },
    iris_execution_verifier::{PinnedPolicy, evaluate},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::Value;
use std::{path::Path, time::Duration};

pub struct VerificationStore {
    db: Connection,
    restore_hold: bool,
}
type PolicyRow = (String, String, i64, String, String);
type ReceiptRow = (String, String, String, String);

fn key(binding: &Binding) -> Result<String> {
    check_binding(binding)?;
    Ok(sha(
        &serde_json::to_vec(binding).map_err(|_| fail("VERIFIER_STORAGE"))?
    ))
}
fn encoded(value: &Value) -> Result<String> {
    serde_json::to_string(value).map_err(|_| fail("VERIFIER_STORAGE"))
}
fn checked(raw: &str, digest: &str) -> Result<Value> {
    if sha(raw.as_bytes()) != digest {
        return Err(fail("VERIFIER_STORAGE"));
    }
    parse_json(raw.as_bytes()).map_err(|_| fail("VERIFIER_STORAGE"))
}
fn policy_row(db: &Connection, binding_key: &str) -> Result<Option<PolicyRow>> {
    sql(db.query_row(
        "SELECT spec_hash,raw_document,pinned_at_ms,receipt,receipt_digest FROM policies WHERE binding_key=?1",
        [binding_key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    ).optional())
}
fn load_policy(db: &Connection, binding_key: &str) -> Result<PinnedPolicy> {
    let (digest, raw, pinned_at, receipt, receipt_digest) =
        policy_row(db, binding_key)?.ok_or_else(|| fail("NOT_FOUND"))?;
    let pinned_at = u64::try_from(pinned_at).map_err(|_| fail("VERIFIER_STORAGE"))?;
    let policy =
        PinnedPolicy::parse(raw.as_bytes(), pinned_at).map_err(|_| fail("VERIFIER_STORAGE"))?;
    if policy.spec_hash() != digest
        || key(policy.binding())? != binding_key
        || checked(&receipt, &receipt_digest)? != policy.pin_receipt()
    {
        return Err(fail("VERIFIER_STORAGE"));
    }
    Ok(policy)
}
fn receipt_row(db: &Connection, binding_key: &str) -> Result<Option<ReceiptRow>> {
    sql(db.query_row(
        "SELECT input_hash,ticket_json,receipt,receipt_digest FROM verifications WHERE binding_key=?1",
        [binding_key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ).optional())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReceipt {
    kind: String,
    verifier: String,
    spec_hash: String,
    policy_id: String,
    policy_revision: u64,
    binding: Binding,
    owner_id: String,
    owner_epoch: u64,
    input_sha256: String,
    verdict: String,
    observed_at_ms: u64,
    verification_scope: String,
    task_outcome_verified: bool,
    effects_verified: bool,
    remote_jobs_stopped: bool,
}
fn load_receipt(db: &Connection, binding_key: &str) -> Result<Option<Value>> {
    let policy = load_policy(db, binding_key)?;
    let Some((input, ticket_raw, raw, digest)) = receipt_row(db, binding_key)? else {
        return Ok(None);
    };
    let value = checked(&raw, &digest)?;
    let r: StoredReceipt =
        serde_json::from_value(value.clone()).map_err(|_| fail("VERIFIER_STORAGE"))?;
    let ticket: OwnerTicket = serde_json::from_value(
        parse_json(ticket_raw.as_bytes()).map_err(|_| fail("VERIFIER_STORAGE"))?,
    )
    .map_err(|_| fail("VERIFIER_STORAGE"))?;
    let pin = policy.pin_receipt();
    if r.kind != "independent_verification"
        || r.verifier != "strict-json-payload-equality/1"
        || r.spec_hash != policy.spec_hash()
        || r.policy_id != pin["policy_id"]
        || r.policy_revision != pin["policy_revision"]
        || r.binding != *policy.binding()
        || ticket.binding != r.binding
        || ticket.owner_id != r.owner_id
        || ticket.owner_epoch != r.owner_epoch
        || !id(&r.owner_id)
        || r.owner_epoch == 0
        || r.owner_epoch > i64::MAX as u64
        || !hash(&input)
        || r.input_sha256 != input
        || !matches!(r.verdict.as_str(), "pass" | "fail")
        || r.observed_at_ms < policy.pinned_at_ms()
        || r.observed_at_ms > i64::MAX as u64
        || r.observed_at_ms >= policy.expires_at_ms()
        || r.verification_scope != "fixed-payload-only"
        || r.task_outcome_verified
        || r.effects_verified
        || r.remote_jobs_stopped
    {
        return Err(fail("VERIFIER_STORAGE"));
    }
    Ok(Some(value))
}
impl VerificationStore {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_mode(path, false)
    }
    pub fn open_restored(path: &Path) -> Result<Self> {
        Self::open_mode(path, true)
    }
    fn open_mode(path: &Path, restore_hold: bool) -> Result<Self> {
        let existed = path.exists();
        if existed {
            check_store_path(path)?;
        }
        let mut db = sql(Connection::open(path))?;
        sql(db.busy_timeout(Duration::from_millis(2500)))?;
        if sql(db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)))? != "ok" {
            return Err(fail("VERIFIER_STORAGE"));
        }
        sql(db.execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;",
        ))?;
        let tx = sql(db.transaction_with_behavior(TransactionBehavior::Immediate))?;
        let version = sql(tx.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)))?;
        if version == 0 && !existed {
            sql(tx.execute_batch(
                "CREATE TABLE meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1),schema_version INTEGER NOT NULL CHECK(schema_version=1),restore_hold INTEGER NOT NULL CHECK(restore_hold IN(0,1)));
                 INSERT INTO meta VALUES(1,1,0);
                 CREATE TABLE policies(binding_key TEXT PRIMARY KEY,spec_hash TEXT NOT NULL,raw_document TEXT NOT NULL,pinned_at_ms INTEGER NOT NULL CHECK(pinned_at_ms>=0),receipt TEXT NOT NULL,receipt_digest TEXT NOT NULL);
                 CREATE TABLE verifications(binding_key TEXT PRIMARY KEY REFERENCES policies(binding_key),input_hash TEXT NOT NULL,ticket_json TEXT NOT NULL,receipt TEXT NOT NULL,receipt_digest TEXT NOT NULL);
                 PRAGMA user_version=1;"
            ))?;
        } else if version != 1 {
            return Err(fail("VERIFIER_SCHEMA"));
        }
        if restore_hold {
            sql(tx.execute("UPDATE meta SET restore_hold=1 WHERE singleton=1", []))?;
        }
        let (schema, held): (i64, i64) = sql(tx.query_row(
            "SELECT schema_version,restore_hold FROM meta WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))?;
        if schema != 1 || !matches!(held, 0 | 1) {
            return Err(fail("VERIFIER_STORAGE"));
        }
        sql(tx.commit())?;
        let store = Self {
            db,
            restore_hold: held == 1,
        };
        // Validate all persisted data before accepting any new writes. This is
        // corruption detection, not authenticity against rewriting the whole DB.
        let keys = sql(store.db.prepare(
            "SELECT binding_key FROM policies UNION SELECT binding_key FROM verifications",
        ))?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| fail("VERIFIER_STORAGE"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| fail("VERIFIER_STORAGE"))?;
        for k in keys {
            load_receipt(&store.db, &k)?;
        }
        Ok(store)
    }
    fn writable(&self) -> Result<()> {
        let held: i64 =
            sql(self
                .db
                .query_row("SELECT restore_hold FROM meta WHERE singleton=1", [], |r| {
                    r.get(0)
                }))?;
        if self.restore_hold || held != 0 {
            return Err(fail("VERIFIER_RESTORE_HOLD"));
        }
        Ok(())
    }
    /// Rust/operator composition only. The owner writer lock spans the policy
    /// commit, proving ordering without relying on millisecond comparisons.
    pub fn pin_policy(
        &mut self,
        raw: &[u8],
        owners: &mut OwnerStore,
        now_ms: u64,
    ) -> Result<Value> {
        self.writable()?;
        // Parse identity before applying the current-time admission window.
        // An exact historical pin is inspection, including after expiry.
        let proposed = PinnedPolicy::parse(raw, 0)?;
        let k = key(proposed.binding())?;
        if policy_row(&self.db, &k)?.is_some() {
            let old = load_policy(&self.db, &k)?;
            if old.spec_hash() != proposed.spec_hash() {
                return Err(fail("VERIFIER_POLICY_CONFLICT"));
            }
            return Ok(old.pin_receipt());
        }
        let proposed = PinnedPolicy::parse(raw, now_ms)?;
        let receipt = proposed.pin_receipt();
        let document = std::str::from_utf8(raw).map_err(|_| fail("VERIFIER_POLICY"))?;
        owners.before_owner_intent(proposed.binding(), || {
            let tx = sql(self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate))?;
            // Check hold again inside the write transaction; another connection
            // may have opened this database in restored-inspection mode.
            let held: i64 = sql(tx.query_row(
                "SELECT restore_hold FROM meta WHERE singleton=1",
                [],
                |r| r.get(0),
            ))?;
            if held != 0 {
                return Err(fail("VERIFIER_RESTORE_HOLD"));
            }
            if policy_row(&tx, &k)?.is_some() {
                return Err(fail("VERIFIER_POLICY_CONFLICT"));
            }
            let text = encoded(&receipt)?;
            sql(tx.execute(
                "INSERT INTO policies VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    k,
                    proposed.spec_hash(),
                    document,
                    now_ms as i64,
                    text,
                    sha(text.as_bytes())
                ],
            ))?;
            sql(tx.commit())?;
            Ok(receipt.clone())
        })
    }
    pub fn verify(
        &mut self,
        owners: &OwnerStore,
        permit: &ArtifactPermit,
        ticket: &OwnerTicket,
        input_hash: &str,
        now_ms: u64,
    ) -> Result<Value> {
        self.writable()?;
        let k = key(&ticket.binding)?;
        let policy = load_policy(&self.db, &k)?;
        if let Some(old) = load_receipt(&self.db, &k)? {
            if old["input_sha256"] != input_hash
                || old["owner_id"] != ticket.owner_id
                || old["owner_epoch"] != ticket.owner_epoch
            {
                return Err(fail("VERIFIER_INPUT_CONFLICT"));
            }
            return Ok(old); // Original historical decision, never re-evaluation.
        }
        let receipt = evaluate(&policy, owners, permit, ticket, input_hash, now_ms)?;
        let tx = sql(self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        let held: i64 = sql(tx.query_row(
            "SELECT restore_hold FROM meta WHERE singleton=1",
            [],
            |r| r.get(0),
        ))?;
        if held != 0 {
            return Err(fail("VERIFIER_RESTORE_HOLD"));
        }
        if let Some(old) = load_receipt(&tx, &k)? {
            if old["input_sha256"] == input_hash
                && old["owner_id"] == ticket.owner_id
                && old["owner_epoch"] == ticket.owner_epoch
            {
                return Ok(old);
            }
            return Err(fail("VERIFIER_INPUT_CONFLICT"));
        }
        let text = encoded(&receipt)?;
        let ticket_text = serde_json::to_string(ticket).map_err(|_| fail("VERIFIER_STORAGE"))?;
        sql(tx.execute(
            "INSERT INTO verifications VALUES(?1,?2,?3,?4,?5)",
            params![k, input_hash, ticket_text, text, sha(text.as_bytes())],
        ))?;
        sql(tx.commit())?;
        Ok(receipt)
    }
    /// Original historical verification only; does not reread the artifact,
    /// launch anything, or certify current policy/input validity.
    pub fn query(&self, binding: &Binding) -> Result<Value> {
        load_receipt(&self.db, &key(binding)?)?.ok_or_else(|| fail("NOT_FOUND"))
    }

    /// Historical public pin metadata for trusted launch composition. This
    /// never creates a policy or authorizes a new launch after its expiry.
    pub fn policy_pin(&self, binding: &Binding) -> Result<Value> {
        Ok(load_policy(&self.db, &key(binding)?)?.pin_receipt())
    }

    /// Fresh admission must inspect the persistent restore hold independently
    /// of historical pin/verification lookup.
    pub fn check_admission(&self) -> Result<()> {
        self.writable()
    }
}
