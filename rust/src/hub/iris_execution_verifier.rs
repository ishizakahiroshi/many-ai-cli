//! Trusted fixed-payload comparison only. This is not a task/effects verifier
//! and has no HTTP entry, command evaluation, policy lookup or database writer.
use super::{
    iris_execution_journal::Binding,
    iris_execution_mcp::parse_json,
    iris_execution_owner::{
        ArtifactPermit, OwnerStore, OwnerTicket, Result, check_binding, fail, hash, id, sha,
        static_binding,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};

const VERIFIER: &str = "strict-json-payload-equality/1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyDocument {
    schema_version: u32,
    policy_id: String,
    policy_revision: u64,
    verifier: String,
    binding: Binding,
    expires_at_ms: u64,
    expected_payload: Value,
}

/// Construct only from operator-approved bytes before beginning the owner
/// attempt. Private fields prevent result JSON from substituting expectations.
pub(crate) struct PinnedPolicy {
    document: PolicyDocument,
    spec_hash: String,
    pinned_at_ms: u64,
}
impl std::fmt::Debug for PinnedPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinnedPolicy")
            .field("spec_hash", &self.spec_hash)
            .finish_non_exhaustive()
    }
}
impl PinnedPolicy {
    pub(crate) fn parse(raw: &[u8], pinned_at_ms: u64) -> Result<Self> {
        // Shared parser bounds bytes/depth and rejects duplicate keys, including
        // duplicates inside the operator's arbitrary expected JSON payload.
        let value = parse_json(raw).map_err(|_| fail("VERIFIER_POLICY"))?;
        let document: PolicyDocument =
            serde_json::from_value(value).map_err(|_| fail("VERIFIER_POLICY"))?;
        if document.schema_version != 1
            || document.verifier != VERIFIER
            || !id(&document.policy_id)
            || document.policy_revision == 0
            || document.policy_revision > i64::MAX as u64
            || pinned_at_ms > i64::MAX as u64
            || document.expires_at_ms <= pinned_at_ms
            || document.expires_at_ms > i64::MAX as u64
        {
            return Err(fail("VERIFIER_POLICY"));
        }
        check_binding(&document.binding).map_err(|_| fail("VERIFIER_POLICY_BINDING"))?;
        Ok(Self {
            document,
            spec_hash: sha(raw),
            pinned_at_ms,
        })
    }
    pub(crate) fn binding(&self) -> &Binding {
        &self.document.binding
    }
    pub(crate) fn spec_hash(&self) -> &str {
        &self.spec_hash
    }
    pub(crate) fn pinned_at_ms(&self) -> u64 {
        self.pinned_at_ms
    }
    pub(crate) fn expires_at_ms(&self) -> u64 {
        self.document.expires_at_ms
    }
    /// Public pin metadata only; never disclose operator expectations.
    pub(crate) fn pin_receipt(&self) -> Value {
        json!({
            "kind":"verification_policy_pin", "binding":self.binding(),
            "spec_hash":self.spec_hash(), "policy_id":self.document.policy_id,
            "policy_revision":self.document.policy_revision,
            "pinned_at_ms":self.pinned_at_ms()
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultEnvelope {
    binding: Binding,
    owner_id: String,
    owner_epoch: u64,
    instruction_hash: String,
    outcome: String,
    effects: String,
    synthetic: bool,
    payload: Value,
}

/// `pass` establishes only equality to the pre-pinned operator JSON. Provider
/// outcome/effects reports, native exit and cessation cannot determine verdict.
/// Invalid inputs return a fixed code (unknown); only well-formed inequality is
/// `fail`. Neither result carries the actual or expected payload.
pub(crate) fn evaluate(
    policy: &PinnedPolicy,
    owners: &OwnerStore,
    permit: &ArtifactPermit,
    ticket: &OwnerTicket,
    artifact_sha256: &str,
    now_ms: u64,
) -> Result<Value> {
    if now_ms > i64::MAX as u64
        || now_ms < policy.pinned_at_ms()
        || now_ms >= policy.document.expires_at_ms
    {
        return Err(fail("VERIFIER_EXPIRED"));
    }
    if policy.binding() != &ticket.binding
        || static_binding(permit.binding()) != static_binding(&ticket.binding)
        || !id(&ticket.owner_id)
        || ticket.owner_epoch == 0
        || ticket.owner_epoch > i64::MAX as u64
        || !hash(artifact_sha256)
    {
        return Err(fail("VERIFIER_BINDING"));
    }
    let owner = owners
        .query(&ticket.binding)
        .map_err(|_| fail("VERIFIER_OWNER"))?;
    if owner["ticket"] != json!(ticket)
        || owner["intent_at_ms"]
            .as_u64()
            .is_none_or(|n| n < policy.pinned_at_ms() || n > now_ms)
        || owner["root_pid"]
            .as_u64()
            .is_none_or(|n| n == 0 || n > u32::MAX as u64)
        || owner["root_started_at_ms"]
            .as_u64()
            .is_none_or(|n| n > now_ms)
    {
        return Err(fail("VERIFIER_OWNER"));
    }
    let integrity = &owner["outcome_receipt"];
    if integrity["kind"] != "artifact_outcome"
        || integrity["ticket"] != json!(ticket)
        || integrity["artifact_sha256"] != artifact_sha256
        || integrity["instruction_hash"] != ticket.binding.instruction_hash
        || integrity["verification_scope"] != "artifact-integrity-only"
    {
        return Err(fail("VERIFIER_INTEGRITY"));
    }
    let artifact = permit
        .verify(Some(artifact_sha256))
        .map_err(|_| fail("VERIFIER_ARTIFACT"))?;
    // Deserialize the very bytes hashed by the permit; do not reopen the path.
    let value = parse_json(artifact.bytes()).map_err(|_| fail("VERIFIER_ENVELOPE"))?;
    let envelope: ResultEnvelope =
        serde_json::from_value(value).map_err(|_| fail("VERIFIER_ENVELOPE"))?;
    if envelope.binding != ticket.binding
        || envelope.owner_id != ticket.owner_id
        || envelope.owner_epoch != ticket.owner_epoch
        || envelope.instruction_hash != ticket.binding.instruction_hash
        || !matches!(
            envelope.outcome.as_str(),
            "succeeded" | "failed" | "rejected" | "unknown"
        )
        || !matches!(
            envelope.effects.as_str(),
            "none" | "local-files" | "unknown"
        )
        || (envelope.synthetic
            && (ticket.binding.provider != "synthetic" || ticket.binding.profile_id != "synthetic"))
    {
        return Err(fail("VERIFIER_ENVELOPE"));
    }
    let verdict = if envelope.payload == policy.document.expected_payload {
        "pass"
    } else {
        "fail"
    };
    Ok(json!({
        "kind":"independent_verification", "verifier":VERIFIER,
        "spec_hash":policy.spec_hash(), "policy_id":policy.document.policy_id,
        "policy_revision":policy.document.policy_revision, "binding":ticket.binding,
        "owner_id":ticket.owner_id, "owner_epoch":ticket.owner_epoch,
        "input_sha256":artifact.sha256(), "verdict":verdict, "observed_at_ms":now_ms,
        "verification_scope":"fixed-payload-only", "task_outcome_verified":false,
        "effects_verified":false, "remote_jobs_stopped":false
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> Binding {
        serde_json::from_value(json!({"principal_id":"principal","workspace_id":"workspace","plan_id":"plan","plan_revision":1,"task_id":"task","instruction_revision":1,"instruction_hash":sha(b"instruction"),"approval_id":"approval","approval_revision":1,"scope_hash":sha(b"scope"),"execution_host_id":"host","executor_instance_id":"instance","attempt_id":"attempt","fence":1,"provider":"synthetic","profile_id":"synthetic"})).unwrap()
    }
    fn policy_document() -> Value {
        json!({"schema_version":1,"policy_id":"policy","policy_revision":1,"verifier":VERIFIER,"binding":binding(),"expires_at_ms":1000,"expected_payload":{"answer":"operator-only-marker"}})
    }
    fn pinned() -> PinnedPolicy {
        PinnedPolicy::parse(policy_document().to_string().as_bytes(), 100).unwrap()
    }
    fn fixture(
        payload: Value,
        outcome: &str,
    ) -> (
        tempfile::TempDir,
        OwnerStore,
        ArtifactPermit,
        OwnerTicket,
        String,
    ) {
        let root = tempfile::tempdir().unwrap();
        let b = binding();
        let mut owners = OwnerStore::open(&root.path().join("owner.sqlite3")).unwrap();
        let ticket = owners.begin_owner(&b, "owner", 1, 100).unwrap();
        owners.claim_spawn(&ticket, 1, 100).unwrap();
        owners.bind_root(&ticket, 123, 100).unwrap();
        let bytes=json!({"binding":b,"owner_id":"owner","owner_epoch":1,"instruction_hash":b.instruction_hash,"outcome":outcome,"effects":"none","synthetic":true,"payload":payload}).to_string().into_bytes();
        std::fs::write(root.path().join("result.json"), &bytes).unwrap();
        let permit =
            ArtifactPermit::approved(root.path(), "result.json", &b, 65536, vec!["none".into()])
                .unwrap();
        let digest = sha(&bytes);
        owners.append_outcome(&ticket, 1, &permit, &digest).unwrap();
        (root, owners, permit, ticket, digest)
    }
    #[test]
    fn pinned_policy_strict_bounds_and_raw_digest() {
        let raw = policy_document().to_string();
        let policy = PinnedPolicy::parse(raw.as_bytes(), 100).unwrap();
        assert_eq!(policy.spec_hash(), sha(raw.as_bytes()));
        assert_eq!(policy.binding(), &binding());
        assert_eq!(policy.pinned_at_ms(), 100);
        assert_eq!(policy.pin_receipt()["kind"], "verification_policy_pin");
        assert!(
            !policy
                .pin_receipt()
                .to_string()
                .contains("operator-only-marker")
        );
        for key in [
            "schema_version",
            "policy_id",
            "policy_revision",
            "verifier",
            "binding",
            "expires_at_ms",
            "expected_payload",
        ] {
            let mut v = policy_document();
            v.as_object_mut().unwrap().remove(key);
            assert!(
                PinnedPolicy::parse(v.to_string().as_bytes(), 100).is_err(),
                "missing {key}"
            );
        }
        let mut v = policy_document();
        v["extra"] = json!(true);
        assert!(PinnedPolicy::parse(v.to_string().as_bytes(), 100).is_err());
        let mut v = policy_document();
        v["binding"]["fence"] = json!(0);
        assert!(PinnedPolicy::parse(v.to_string().as_bytes(), 100).is_err());
        assert!(PinnedPolicy::parse(raw.as_bytes(), 1000).is_err());
        assert!(PinnedPolicy::parse(&vec![b' '; 65537], 100).is_err());
        assert!(PinnedPolicy::parse(br#"{"schema_version":1,"schema_version":1}"#, 100).is_err());
        let deep = format!("{}0{}", "[".repeat(70), "]".repeat(70));
        assert!(PinnedPolicy::parse(deep.as_bytes(), 100).is_err());
    }
    #[test]
    fn verdict_is_payload_only_and_never_promotes_task_or_effects() {
        for (payload, outcome, verdict) in [
            (json!({"answer":"operator-only-marker"}), "failed", "pass"),
            (
                json!({"answer":"provider-other-marker"}),
                "succeeded",
                "fail",
            ),
        ] {
            let (_root, owners, permit, ticket, digest) = fixture(payload, outcome);
            let receipt = evaluate(&pinned(), &owners, &permit, &ticket, &digest, 200).unwrap();
            assert_eq!(receipt["verdict"], verdict);
            assert_eq!(receipt["task_outcome_verified"], false);
            assert_eq!(receipt["effects_verified"], false);
            assert_eq!(receipt["remote_jobs_stopped"], false);
            assert_eq!(receipt["verification_scope"], "fixed-payload-only");
            assert!(!receipt.to_string().contains("marker"));
            assert_eq!(
                owners.query(&ticket.binding).unwrap()["verified_task_outcome"],
                "unknown"
            );
        }
    }
    #[test]
    fn expired_late_policy_wrong_ticket_and_changed_file_are_unknown_errors() {
        let (root, owners, permit, ticket, digest) =
            fixture(json!({"answer":"operator-only-marker"}), "succeeded");
        assert_eq!(
            evaluate(&pinned(), &owners, &permit, &ticket, &digest, 1000)
                .unwrap_err()
                .code,
            "VERIFIER_EXPIRED"
        );
        let late = PinnedPolicy::parse(policy_document().to_string().as_bytes(), 101).unwrap();
        assert_eq!(
            evaluate(&late, &owners, &permit, &ticket, &digest, 200)
                .unwrap_err()
                .code,
            "VERIFIER_OWNER"
        );
        let mut wrong = ticket.clone();
        wrong.owner_epoch = 2;
        assert!(evaluate(&pinned(), &owners, &permit, &wrong, &digest, 200).is_err());
        assert!(evaluate(&pinned(), &owners, &permit, &ticket, &sha(b"wrong"), 200).is_err());
        std::fs::write(root.path().join("result.json"), b"changed-provider-data").unwrap();
        let error = evaluate(&pinned(), &owners, &permit, &ticket, &digest, 200).unwrap_err();
        assert_eq!(error.code, "VERIFIER_ARTIFACT");
        assert!(!error.to_string().contains("provider-data"));
    }
}
