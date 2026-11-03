//! Independent blackbox acceptance for fixed-payload verification only.
//! Owner root IDs here are trusted Rust fixtures, not native cessation proof.
//! No process, provider, Hub, external network, or task-success authority.
use many_ai_cli::hub::{
    iris_execution_grants::GrantLoader,
    iris_execution_journal::Binding,
    iris_execution_owner::{ArtifactPermit, OwnerStore, OwnerTicket},
    iris_execution_verification_store::VerificationStore,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn binding() -> Binding {
    Binding {
        principal_id: "synthetic-principal".into(),
        workspace_id: "synthetic-workspace".into(),
        plan_id: "synthetic-plan".into(),
        plan_revision: 1,
        task_id: "synthetic-task".into(),
        instruction_revision: 1,
        instruction_hash: hash(b"synthetic verifier instruction"),
        approval_id: "synthetic-approval".into(),
        approval_revision: 1,
        scope_hash: hash(b"synthetic scope"),
        execution_host_id: "synthetic-host".into(),
        executor_instance_id: "synthetic-instance".into(),
        attempt_id: "synthetic-attempt".into(),
        fence: 1,
        provider: "synthetic".into(),
        profile_id: "synthetic".into(),
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    b: Binding,
    owners: OwnerStore,
    store: VerificationStore,
    permit: ArtifactPermit,
    db: PathBuf,
    result: PathBuf,
    at: u64,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("workspace")).unwrap();
        std::fs::create_dir(root.join("iris-execution-artifacts")).unwrap();
        std::fs::write(
            root.join("iris-execution-artifacts/instruction.txt"),
            b"synthetic verifier instruction",
        )
        .unwrap();
        let b = binding();
        let manifest = json!({"schema_version":1,"manifest_revision":1,"approval_state":"approved","grants":[{"binding":b,"expires_at":10000000000i64,"revoked":false,"approved_cwd":"workspace","instruction_path":"instruction.txt","result_path":"result.json","max_artifact_bytes":65536,"effect_scope":["none"],"allow_remote_jobs":false}]});
        std::fs::write(
            root.join("iris-execution-approved-grants.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let mut loader = GrantLoader::open(root).unwrap();
        let permit = loader.load_approved().unwrap().remove(0).result;
        let db = root.join("verification.sqlite3");
        let store = VerificationStore::open(&db).unwrap();
        let owners = OwnerStore::open(&root.join("owners.sqlite3")).unwrap();
        let result = root.join("iris-execution-artifacts/result.json");
        Self {
            _dir: dir,
            b,
            owners,
            store,
            permit,
            db,
            result,
            at: ms(),
        }
    }
    fn policy(&self) -> Value {
        json!({"schema_version":1,"policy_id":"synthetic-policy","policy_revision":1,"verifier":"strict-json-payload-equality/1","binding":self.b,"expires_at_ms":self.at+10000,"expected_payload":{"answer":42,"items":["alpha","beta"]}})
    }
    fn pin(&mut self) -> Value {
        self.store
            .pin_policy(
                &serde_json::to_vec(&self.policy()).unwrap(),
                &mut self.owners,
                self.at,
            )
            .unwrap()
    }
    fn intent(&mut self) -> OwnerTicket {
        self.owners
            .begin_owner(&self.b, "synthetic-owner", 1, self.at)
            .unwrap()
    }
    fn root(&mut self) -> OwnerTicket {
        let ticket = self.intent();
        self.owners.claim_spawn(&ticket, 5001, self.at).unwrap();
        self.owners.bind_root(&ticket, 6001, self.at).unwrap();
        ticket
    }
    fn write_result(&mut self, ticket: &OwnerTicket, payload: Value) -> String {
        let document = json!({"binding":self.b,"owner_id":ticket.owner_id,"owner_epoch":ticket.owner_epoch,"instruction_hash":self.b.instruction_hash,"outcome":"succeeded","effects":"none","synthetic":true,"payload":payload});
        let bytes = serde_json::to_vec(&document).unwrap();
        std::fs::write(&self.result, &bytes).unwrap();
        hash(&bytes)
    }
    fn outcome(&mut self, ticket: &OwnerTicket, payload: Value) -> String {
        let digest = self.write_result(ticket, payload);
        self.owners
            .append_outcome(ticket, 1, &self.permit, &digest)
            .unwrap();
        digest
    }
    fn verify(&mut self, ticket: &OwnerTicket, digest: &str, at: u64) -> Value {
        self.store
            .verify(&self.owners, &self.permit, ticket, digest, at)
            .unwrap()
    }
}
fn assert_no_authority(receipt: &Value) {
    assert_eq!(receipt["kind"], "independent_verification");
    assert_eq!(receipt["verifier"], "strict-json-payload-equality/1");
    assert_eq!(receipt["verification_scope"], "fixed-payload-only");
    for key in [
        "task_outcome_verified",
        "effects_verified",
        "remote_jobs_stopped",
    ] {
        assert_eq!(receipt[key], false, "unexpected authority {key}");
    }
}

#[test]
fn fixed_payload_pass_is_independent_of_report_and_never_task_or_effects_authority() {
    let mut f = Fixture::new();
    assert_eq!(f.store.query(&f.b).unwrap_err().code, "NOT_FOUND");
    f.pin();
    assert_eq!(
        f.store.query(&f.b).unwrap_err().code,
        "NOT_FOUND",
        "pin alone was promoted to a verification"
    );
    let ticket = f.root();
    let digest = f.outcome(&ticket, json!({"answer":42,"items":["alpha","beta"]}));
    let owner_before = f.owners.query(&f.b).unwrap();
    let at = f.at;
    let receipt = f.verify(&ticket, &digest, at);
    assert_eq!(receipt["verdict"], "pass");
    assert_eq!(receipt["binding"], json!(f.b));
    assert_eq!(receipt["owner_id"], ticket.owner_id);
    assert_eq!(receipt["owner_epoch"], ticket.owner_epoch);
    assert_eq!(receipt["input_sha256"], digest);
    assert_eq!(receipt["policy_id"], "synthetic-policy");
    assert_eq!(receipt["policy_revision"], 1);
    assert_eq!(receipt["observed_at_ms"], at);
    assert_eq!(receipt["spec_hash"].as_str().unwrap().len(), 64);
    assert_no_authority(&receipt);
    assert_eq!(
        f.owners.query(&f.b).unwrap(),
        owner_before,
        "verification mutated owner execution authority"
    );
    assert_ne!(owner_before["local_tree_ceased"], true);
    assert_eq!(f.store.query(&f.b).unwrap(), receipt);
    let reopened = VerificationStore::open(&f.db).unwrap();
    assert_eq!(reopened.query(&f.b).unwrap(), receipt);
}

#[test]
fn provider_succeeded_and_effects_none_cannot_override_payload_mismatch() {
    let mut f = Fixture::new();
    f.pin();
    let ticket = f.root();
    let digest = f.outcome(&ticket, json!({"answer":0,"items":["alpha","beta"]}));
    let at = f.at;
    let receipt = f.verify(&ticket, &digest, at);
    assert_eq!(receipt["verdict"], "fail");
    assert_no_authority(&receipt);
    assert_eq!(
        f.owners.query(&f.b).unwrap()["verified_task_outcome"],
        "unknown"
    );
}

#[test]
fn immutable_pin_and_historical_verify_replay_do_not_create_new_authority() {
    let mut f = Fixture::new();
    let raw = serde_json::to_vec(&f.policy()).unwrap();
    let pin = f.store.pin_policy(&raw, &mut f.owners, f.at).unwrap();
    let ticket = f.root();
    assert_eq!(f.store.pin_policy(&raw, &mut f.owners, f.at).unwrap(), pin);
    let mut changed = f.policy();
    changed["expected_payload"] = json!({"answer":99});
    assert!(
        f.store
            .pin_policy(&serde_json::to_vec(&changed).unwrap(), &mut f.owners, f.at)
            .is_err()
    );
    let digest = f.outcome(&ticket, json!({"answer":42,"items":["alpha","beta"]}));
    let at = f.at;
    let receipt = f.verify(&ticket, &digest, at);
    std::fs::remove_file(&f.result).unwrap();
    assert_eq!(
        f.verify(&ticket, &digest, at + 20000),
        receipt,
        "historical replay must retain original observation time and content"
    );
    let restored = VerificationStore::open_restored(&f.db).unwrap();
    assert_eq!(restored.query(&f.b).unwrap(), receipt);
    drop(restored);
    // The normal open must retain the restore hold rather than silently clear it.
    let mut reopened = VerificationStore::open(&f.db).unwrap();
    assert!(reopened.pin_policy(&raw, &mut f.owners, at).is_err());
    assert!(
        reopened
            .verify(&f.owners, &f.permit, &ticket, &digest, at)
            .is_err()
    );
    assert_eq!(reopened.query(&f.b).unwrap(), receipt);
}

#[test]
fn policy_must_precede_owner_intent_and_be_live_for_new_verification() {
    let mut late = Fixture::new();
    let _ticket = late.intent();
    let raw = serde_json::to_vec(&late.policy()).unwrap();
    assert!(
        late.store
            .pin_policy(&raw, &mut late.owners, late.at)
            .is_err()
    );
    let mut expired = Fixture::new();
    let mut policy = expired.policy();
    policy["expires_at_ms"] = json!(expired.at);
    assert!(
        expired
            .store
            .pin_policy(
                &serde_json::to_vec(&policy).unwrap(),
                &mut expired.owners,
                expired.at
            )
            .is_err()
    );
    let mut f = Fixture::new();
    f.pin();
    let ticket = f.root();
    let digest = f.outcome(&ticket, json!({"answer":42,"items":["alpha","beta"]}));
    assert!(
        f.store
            .verify(&f.owners, &f.permit, &ticket, &digest, f.at + 10000)
            .is_err()
    );
    assert_eq!(f.store.query(&f.b).unwrap_err().code, "NOT_FOUND");
}

#[test]
fn missing_integrity_receipt_wrong_ticket_binding_or_input_never_verifies() {
    let mut f = Fixture::new();
    f.pin();
    let ticket = f.root();
    let digest = f.write_result(&ticket, json!({"answer":42,"items":["alpha","beta"]}));
    assert!(
        f.store
            .verify(&f.owners, &f.permit, &ticket, &digest, f.at)
            .is_err(),
        "provider JSON without owner integrity receipt became evidence"
    );
    f.owners
        .append_outcome(&ticket, 1, &f.permit, &digest)
        .unwrap();
    let mut owner = ticket.clone();
    owner.owner_id = "different-owner".into();
    assert!(
        f.store
            .verify(&f.owners, &f.permit, &owner, &digest, f.at)
            .is_err()
    );
    let mut epoch = ticket.clone();
    epoch.owner_epoch += 1;
    assert!(
        f.store
            .verify(&f.owners, &f.permit, &epoch, &digest, f.at)
            .is_err()
    );
    for field in [
        "attempt_id",
        "fence",
        "instruction_hash",
        "approval_revision",
        "execution_host_id",
        "executor_instance_id",
    ] {
        let mut value = json!(ticket);
        value["binding"][field] = match field {
            "fence" | "approval_revision" => json!(2),
            "instruction_hash" => json!(hash(b"other instruction")),
            _ => json!("different-identity"),
        };
        let changed: OwnerTicket = serde_json::from_value(value).unwrap();
        assert!(
            f.store
                .verify(&f.owners, &f.permit, &changed, &digest, f.at)
                .is_err(),
            "wrong {field} accepted"
        );
    }
    assert!(
        f.store
            .verify(
                &f.owners,
                &f.permit,
                &ticket,
                &hash(b"wrong artifact"),
                f.at
            )
            .is_err()
    );
    std::fs::write(&f.result, b"tampered after owner hash").unwrap();
    assert!(
        f.store
            .verify(&f.owners, &f.permit, &ticket, &digest, f.at)
            .is_err()
    );
    assert_eq!(f.store.query(&f.b).unwrap_err().code, "NOT_FOUND");
}

#[test]
fn strict_policy_parser_rejects_duplicate_unknown_oversized_and_invalid_utf8() {
    let mut f = Fixture::new();
    let valid = serde_json::to_string(&f.policy()).unwrap();
    let mut unknown = f.policy();
    unknown["caller_success_override"] = json!(true);
    let duplicate = format!("{{\"schema_version\":1,{}", &valid[1..]);
    let mut oversized = f.policy();
    oversized["expected_payload"] = json!("x".repeat(1024 * 1024));
    for raw in [
        duplicate.into_bytes(),
        serde_json::to_vec(&unknown).unwrap(),
        serde_json::to_vec(&oversized).unwrap(),
        vec![0xff, 0xfe],
        b"{} trailing".to_vec(),
    ] {
        assert!(f.store.pin_policy(&raw, &mut f.owners, f.at).is_err());
    }
    f.pin();
    assert_eq!(f.store.query(&f.b).unwrap_err().code, "NOT_FOUND");
}

#[test]
fn verified_receipt_rejects_other_owner_input_policy_and_binding_reuse() {
    let mut f = Fixture::new();
    f.pin();
    let ticket = f.root();
    let digest = f.outcome(&ticket, json!({"answer":42,"items":["alpha","beta"]}));
    let at = f.at;
    let receipt = f.verify(&ticket, &digest, at);
    let mut wrong = ticket.clone();
    wrong.owner_epoch += 1;
    assert!(
        f.store
            .verify(&f.owners, &f.permit, &wrong, &digest, at)
            .is_err()
    );
    assert!(
        f.store
            .verify(&f.owners, &f.permit, &ticket, &hash(b"different input"), at)
            .is_err()
    );
    let mut foreign = f.b.clone();
    foreign.attempt_id = "different-attempt".into();
    assert_eq!(f.store.query(&foreign).unwrap_err().code, "NOT_FOUND");
    assert_eq!(f.store.query(&f.b).unwrap(), receipt);
}

#[test]
fn stored_policy_and_verification_tamper_are_not_returned_as_historical_truth() {
    for target in ["verifications", "policies"] {
        let mut f = Fixture::new();
        f.pin();
        let ticket = f.root();
        let digest = f.outcome(&ticket, json!({"answer":42,"items":["alpha","beta"]}));
        let at = f.at;
        let _receipt = f.verify(&ticket, &digest, at);
        let db = rusqlite::Connection::open(&f.db).unwrap();
        match target {
            "verifications" => db
                .execute("UPDATE verifications SET receipt='{}'", [])
                .unwrap(),
            "policies" => db
                .execute("UPDATE policies SET raw_document='{}'", [])
                .unwrap(),
            _ => unreachable!(),
        };
        drop(db);
        assert!(
            f.store.query(&f.b).is_err(),
            "query concealed {target} tamper"
        );
        assert!(
            f.store
                .verify(&f.owners, &f.permit, &ticket, &digest, at)
                .is_err(),
            "replay concealed {target} tamper"
        );
        assert!(
            VerificationStore::open(&f.db).is_err(),
            "reopen concealed {target} tamper"
        );
    }
}

#[test]
fn same_policy_pin_after_expiry_returns_original_receipt_without_reapproval() {
    let mut f = Fixture::new();
    let raw = serde_json::to_vec(&f.policy()).unwrap();
    let original = f.store.pin_policy(&raw, &mut f.owners, f.at).unwrap();
    let _ticket = f.root();
    for at in [f.at + 10000, f.at + 20000] {
        assert_eq!(
            f.store.pin_policy(&raw, &mut f.owners, at).unwrap(),
            original,
            "historical pin replay extended or recreated approval"
        );
    }
    assert_eq!(f.store.query(&f.b).unwrap_err().code, "NOT_FOUND");
}

#[test]
fn rehashed_receipt_at_or_after_policy_expiry_is_semantically_invalid() {
    for beyond_expiry in [0, 1] {
        let mut f = Fixture::new();
        f.pin();
        let ticket = f.root();
        let digest = f.outcome(&ticket, json!({"answer":42,"items":["alpha","beta"]}));
        let at = f.at;
        let mut receipt = f.verify(&ticket, &digest, at);
        receipt["observed_at_ms"] = json!(at + 10000 + beyond_expiry);
        let raw = serde_json::to_string(&receipt).unwrap();
        let db = rusqlite::Connection::open(&f.db).unwrap();
        assert_eq!(
            db.execute(
                "UPDATE verifications SET receipt=?1,receipt_digest=?2",
                rusqlite::params![raw, hash(raw.as_bytes())],
            )
            .unwrap(),
            1
        );
        drop(db);
        assert!(
            f.store.query(&f.b).is_err(),
            "query trusted a rehashed verification made outside policy lifetime"
        );
        assert!(
            f.store
                .verify(&f.owners, &f.permit, &ticket, &digest, at)
                .is_err(),
            "historical replay bypassed observed-time policy validation"
        );
        assert!(
            VerificationStore::open(&f.db).is_err(),
            "open trusted a rehashed verification made outside policy lifetime"
        );
    }
}
