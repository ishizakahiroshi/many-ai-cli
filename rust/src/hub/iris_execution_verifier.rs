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
/// Real project profile only: the policy pins the work folder's launch
/// baseline and launch limits. It never produces a verdict.
pub const FOLDER_RECORD_VERIFIER: &str = "folder-change-record/1";
pub const DEFAULT_MAX_RUNTIME_SEC: u64 = 1800;
pub const MAX_RUNTIME_SEC_LIMIT: u64 = 3600;
const MAX_ALLOWED_PATHS: usize = 64;
const MAX_ALLOWED_TOOLS: usize = 16;

/// Present-only field: absent is `None`, any JSON value including `null` is
/// `Some`. Keeps the existing strict policy shape exactly: a strict policy
/// that names a folder field, even as null, is still refused.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyDocument {
    schema_version: u32,
    policy_id: String,
    policy_revision: u64,
    verifier: String,
    binding: Binding,
    expires_at_ms: u64,
    #[serde(default, deserialize_with = "present")]
    expected_payload: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    baseline_sha256: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    baseline_entry_count: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    allowed_paths: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    allowed_tools: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    max_runtime_sec: Option<Value>,
}

/// The `folder-change-record/1` launch limits, from operator bytes only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderPolicy {
    pub baseline_sha256: String,
    pub baseline_entry_count: u64,
    /// Relative `/`-separated folder paths, or `.` for the whole folder.
    pub allowed_paths: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub max_runtime_sec: u64,
}
impl FolderPolicy {
    pub fn document(&self) -> Value {
        json!({"baseline_sha256":self.baseline_sha256,
            "baseline_entry_count":self.baseline_entry_count,
            "allowed_paths":self.allowed_paths,"allowed_tools":self.allowed_tools,
            "max_runtime_sec":self.max_runtime_sec})
    }
}

/// `.` or a relative path of plain names joined by `/`, optionally ending in
/// one `/` (a folder, e.g. `.pytest-tmp/`). No drive, root, backslash, `..`,
/// empty or reserved component, and never a protected name.
pub fn valid_allowed_path(raw: &str) -> bool {
    if raw == "." {
        return true;
    }
    let raw = raw.strip_suffix('/').unwrap_or(raw);
    !raw.is_empty()
        && raw.len() <= 1024
        && !raw.contains('\\')
        && raw.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !super::iris_execution_folder::protected_name(part)
                && super::iris_execution_owner::relative_path(part).is_ok()
        })
}

fn folder_policy(d: &PolicyDocument) -> Option<FolderPolicy> {
    let baseline_sha256 = d.baseline_sha256.as_ref()?.as_str()?.to_owned();
    let baseline_entry_count = d.baseline_entry_count.as_ref()?.as_u64()?;
    let strings = |value: &Value, limit: usize| -> Option<Vec<String>> {
        let items = value.as_array()?;
        if items.len() > limit {
            return None;
        }
        let mut out: Vec<String> = Vec::new();
        for item in items {
            let text = item.as_str()?.to_owned();
            if out.contains(&text) {
                return None;
            }
            out.push(text);
        }
        Some(out)
    };
    let allowed_paths = strings(d.allowed_paths.as_ref()?, MAX_ALLOWED_PATHS)?;
    let allowed_tools = strings(d.allowed_tools.as_ref()?, MAX_ALLOWED_TOOLS)?;
    let max_runtime_sec = match &d.max_runtime_sec {
        None => DEFAULT_MAX_RUNTIME_SEC,
        Some(value) => value.as_u64()?,
    };
    if !hash(&baseline_sha256)
        || baseline_entry_count == 0
        || baseline_entry_count > super::iris_execution_folder::MAX_ENTRIES as u64
        || allowed_paths.is_empty()
        || !allowed_paths.iter().all(|p| valid_allowed_path(p))
        || !allowed_tools
            .iter()
            .all(|t| crate::config::valid_allowed_tool_value(t) && t.trim() == t)
        || max_runtime_sec == 0
        || max_runtime_sec > MAX_RUNTIME_SEC_LIMIT
    {
        return None;
    }
    Some(FolderPolicy {
        baseline_sha256,
        baseline_entry_count,
        allowed_paths,
        allowed_tools,
        max_runtime_sec,
    })
}

/// Construct only from operator-approved bytes before beginning the owner
/// attempt. Private fields prevent result JSON from substituting expectations.
pub(crate) struct PinnedPolicy {
    document: PolicyDocument,
    spec_hash: String,
    pinned_at_ms: u64,
    folder: Option<FolderPolicy>,
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
        // Exactly one shape per verifier. The strict shape is unchanged: it
        // requires expected_payload and admits none of the folder fields.
        let folder_fields = [
            &document.baseline_sha256,
            &document.baseline_entry_count,
            &document.allowed_paths,
            &document.allowed_tools,
            &document.max_runtime_sec,
        ];
        let folder = match document.verifier.as_str() {
            VERIFIER
                if document.expected_payload.is_some()
                    && folder_fields.iter().all(|f| f.is_none()) =>
            {
                None
            }
            FOLDER_RECORD_VERIFIER if document.expected_payload.is_none() => {
                Some(folder_policy(&document).ok_or_else(|| fail("VERIFIER_POLICY"))?)
            }
            _ => return Err(fail("VERIFIER_POLICY")),
        };
        if document.schema_version != 1
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
            folder,
        })
    }
    pub(crate) fn binding(&self) -> &Binding {
        &self.document.binding
    }
    /// The real project launch limits; `None` for a fixed-payload policy.
    pub(crate) fn folder(&self) -> Option<&FolderPolicy> {
        self.folder.as_ref()
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
        let mut receipt = json!({
            "kind":"verification_policy_pin", "binding":self.binding(),
            "spec_hash":self.spec_hash(), "policy_id":self.document.policy_id,
            "policy_revision":self.document.policy_revision,
            "pinned_at_ms":self.pinned_at_ms()
        });
        // Fixed-payload pins keep their exact shape. Folder limits are public
        // operator values (no expectation to hide), recorded with the pin.
        if let Some(folder) = &self.folder {
            receipt["verifier"] = json!(FOLDER_RECORD_VERIFIER);
            receipt["folder_policy"] = folder.document();
        }
        receipt
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
    // A folder record policy has no expectation and never yields a verdict.
    let Some(expected) = policy
        .document
        .expected_payload
        .as_ref()
        .filter(|_| policy.folder.is_none())
    else {
        return Err(fail("VERIFIER_POLICY"));
    };
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
        || !(matches!(
            envelope.effects.as_str(),
            "none" | "local-files" | "unknown"
        ) || (matches!(
            envelope.effects.as_str(),
            super::iris_execution_folder::PAYLOAD_ONLY
                | super::iris_execution_folder::UNEXPECTED_CHANGE
        ) && !envelope.synthetic
            && super::iris_execution_owned::real_bounded_binding(&ticket.binding)))
        || (envelope.synthetic
            && (ticket.binding.provider != "synthetic" || ticket.binding.profile_id != "synthetic"))
    {
        return Err(fail("VERIFIER_ENVELOPE"));
    }
    let verdict = if envelope.payload == *expected {
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
    fn folder_document() -> Value {
        json!({"schema_version":1,"policy_id":"policy","policy_revision":1,
            "verifier":FOLDER_RECORD_VERIFIER,"binding":binding(),"expires_at_ms":1000,
            "baseline_sha256":"c".repeat(64),"baseline_entry_count":3,
            "allowed_paths":["src",".pytest-tmp/"],
            "allowed_tools":["Bash(python -B -m pytest -p no:cacheprovider --basetemp=.pytest-tmp:*)"]})
    }
    #[test]
    fn folder_record_policy_has_its_own_strict_shape_and_never_a_verdict() {
        let policy = PinnedPolicy::parse(folder_document().to_string().as_bytes(), 100).unwrap();
        let folder = policy.folder().unwrap();
        assert_eq!(folder.max_runtime_sec, DEFAULT_MAX_RUNTIME_SEC);
        assert_eq!(folder.allowed_paths, ["src", ".pytest-tmp/"]);
        assert_eq!(policy.pin_receipt()["verifier"], FOLDER_RECORD_VERIFIER);
        assert_eq!(policy.pin_receipt()["folder_policy"], folder.document());
        // The fixed-payload pin keeps its exact key set.
        let strict = pinned().pin_receipt();
        assert!(strict.get("verifier").is_none() && strict.get("folder_policy").is_none());
        let ok = |field: &str, value: Option<Value>| {
            let mut v = folder_document();
            match value {
                Some(value) => v[field] = value,
                None => {
                    v.as_object_mut().unwrap().remove(field);
                }
            }
            PinnedPolicy::parse(v.to_string().as_bytes(), 100).is_ok()
        };
        assert!(ok("max_runtime_sec", Some(json!(3600))));
        assert!(ok("allowed_tools", Some(json!([]))));
        assert!(ok("allowed_paths", Some(json!(["."]))));
        for (field, value) in [
            ("expected_payload", Some(json!(1))),
            ("expected_payload", Some(Value::Null)),
            ("max_runtime_sec", Some(json!(0))),
            ("max_runtime_sec", Some(json!(3601))),
            ("allowed_paths", Some(json!([]))),
            ("allowed_paths", Some(json!(["C:/x"]))),
            ("allowed_paths", Some(json!(["/x"]))),
            ("allowed_paths", Some(json!(["../x"]))),
            ("allowed_paths", Some(json!(["a\\b"]))),
            ("allowed_paths", Some(json!(["a//b"]))),
            ("allowed_paths", Some(json!([".claude"]))),
            ("allowed_paths", Some(json!(["src", "src"]))),
            ("allowed_tools", Some(json!(["Bash(rm -rf /;x)"]))),
            ("allowed_tools", Some(json!(["--dangerously"]))),
            ("allowed_tools", Some(json!([" Read"]))),
            ("baseline_sha256", Some(json!("x"))),
            ("baseline_entry_count", Some(json!(0))),
            ("baseline_entry_count", Some(json!(1001))),
            ("allowed_tools", None),
            ("baseline_sha256", None),
            ("extra", Some(json!(1))),
        ] {
            assert!(!ok(field, value.clone()), "{field} {value:?}");
        }
        // A strict policy naming any folder field, even as null, is refused.
        for field in [
            "baseline_sha256",
            "allowed_paths",
            "allowed_tools",
            "max_runtime_sec",
        ] {
            let mut v = policy_document();
            v[field] = Value::Null;
            assert!(
                PinnedPolicy::parse(v.to_string().as_bytes(), 100).is_err(),
                "{field}"
            );
        }
        let mut null_payload = policy_document();
        null_payload["expected_payload"] = Value::Null;
        assert!(PinnedPolicy::parse(null_payload.to_string().as_bytes(), 100).is_ok());
        // No verdict can be produced from a folder record policy.
        let (_root, owners, permit, ticket, digest) =
            fixture(json!({"answer":"operator-only-marker"}), "unknown");
        assert_eq!(
            evaluate(&policy, &owners, &permit, &ticket, &digest, 200)
                .unwrap_err()
                .code,
            "VERIFIER_POLICY"
        );
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
    #[test]
    fn folder_effects_are_comparable_only_for_the_real_pair_and_never_synthetic() {
        use super::super::{
            iris_execution_folder::{PAYLOAD_ONLY, UNEXPECTED_CHANGE},
            iris_execution_owned::{REAL_BOUNDED_PROFILE_ID, REAL_BOUNDED_PROVIDER},
        };
        let collected = || vec![PAYLOAD_ONLY.to_string(), UNEXPECTED_CHANGE.to_string()];
        let run = |provider: &str, profile: &str, effects: &str, synthetic: bool| {
            let root = tempfile::tempdir().unwrap();
            let mut b = binding();
            b.provider = provider.into();
            b.profile_id = profile.into();
            let mut document = policy_document();
            document["binding"] = json!(b);
            let policy = PinnedPolicy::parse(document.to_string().as_bytes(), 100).unwrap();
            let mut owners = OwnerStore::open(&root.path().join("owner.sqlite3")).unwrap();
            let ticket = owners.begin_owner(&b, "owner", 1, 100).unwrap();
            owners.claim_spawn(&ticket, 1, 100).unwrap();
            owners.bind_root(&ticket, 123, 100).unwrap();
            let bytes=json!({"binding":b,"owner_id":"owner","owner_epoch":1,"instruction_hash":b.instruction_hash,"outcome":"unknown","effects":effects,"synthetic":synthetic,"payload":{"answer":"operator-only-marker"}}).to_string().into_bytes();
            std::fs::write(root.path().join("result.json"), &bytes).unwrap();
            // Bypass the owner permit's own scope check: this isolates the
            // verifier's independent envelope gate.
            let permit =
                ArtifactPermit::collected(root.path(), "result.json", &b, 65536, collected())
                    .unwrap();
            let digest = sha(&bytes);
            let _ = owners.append_outcome(&ticket, 1, &permit, &digest);
            let integrity = json!({"kind":"artifact_outcome","ticket":ticket,"sequence":1,"artifact_sha256":digest,"artifact_bytes":bytes.len(),"instruction_hash":b.instruction_hash,"reported_outcome":"unknown","reported_effects":effects,"synthetic":synthetic,"verification_scope":"artifact-integrity-only","task_outcome_verified":false,"effects_verified":false,"remote_jobs_stopped":false,"verified_task_outcome":"unknown"});
            if owners.query(&b).unwrap()["outcome_receipt"].is_null() {
                return Err("owner refused".to_string());
            }
            assert_eq!(owners.query(&b).unwrap()["outcome_receipt"], integrity);
            evaluate(&policy, &owners, &permit, &ticket, &digest, 200)
                .map(|r| r["verdict"].as_str().unwrap().to_string())
                .map_err(|e| e.code)
        };
        assert_eq!(
            run(
                REAL_BOUNDED_PROVIDER,
                REAL_BOUNDED_PROFILE_ID,
                PAYLOAD_ONLY,
                false
            )
            .unwrap(),
            "pass"
        );
        assert_eq!(
            run(
                REAL_BOUNDED_PROVIDER,
                REAL_BOUNDED_PROFILE_ID,
                UNEXPECTED_CHANGE,
                false
            )
            .unwrap(),
            "pass"
        );
        for (provider, profile, synthetic) in [
            ("synthetic", "synthetic", true),
            ("synthetic", "synthetic", false),
            ("codex", "codex", false),
            (REAL_BOUNDED_PROVIDER, "claude-other", false),
        ] {
            assert_eq!(
                run(provider, profile, PAYLOAD_ONLY, synthetic).unwrap_err(),
                "VERIFIER_ENVELOPE",
                "{provider}/{profile}/{synthetic}"
            );
        }
        // synthetic:true is refused for the real pair by the owner receipt.
        assert!(
            run(
                REAL_BOUNDED_PROVIDER,
                REAL_BOUNDED_PROFILE_ID,
                PAYLOAD_ONLY,
                true
            )
            .is_err()
        );
    }
}
