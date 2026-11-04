//! Trusted wrapper owner observer. Bootstrap is server-owned metadata, stripped
//! before provider exec. The fixed operator manifest and durable Hub admission
//! are both required; neither an environment value nor a spawn proof grants work.
use crate::{
    application::spawn_policy::NativeFolderTrust,
    hub::{
        iris_execution_folder as folder,
        iris_execution_grants::{ApprovedGrant, GrantLoader, LaunchMode},
        iris_execution_journal::Journal,
        iris_execution_mcp::parse_json,
        iris_execution_owned::{
            real_bounded_binding, real_bounded_launch_admitted, real_project_launch_policy,
        },
        iris_execution_owner::{OwnerStore, OwnerTicket, sha},
        iris_execution_verifier::FolderPolicy,
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
/// Provider arguments the Hub adds to a real project launch: no user settings
/// (`~/.claude/settings.json` permissions, hooks, default mode, additional
/// directories), project and local settings only (the work folder has no
/// `.claude`, which its baseline refuses), no MCP server from any file, and
/// no Claude in Chrome (`--no-chrome`, listed by the `--help` of claude
/// 2.1.316 and 2.1.318).
pub const REAL_PROJECT_ISOLATION_ARGS: [&str; 4] = [
    "--setting-sources",
    "project,local",
    "--strict-mcp-config",
    "--no-chrome",
];
/// Flags a real project argv must carry exactly once.
const REAL_PROJECT_REQUIRED_FLAGS: [&str; 4] = [
    "--setting-sources",
    "--strict-mcp-config",
    "--no-chrome",
    "--permission-mode",
];
const MAX_ARGV_FILE_BYTES: u64 = 1024 * 1024;

fn failure() -> io::Error {
    io::Error::other("trusted execution owner conditions were not satisfied")
}

/// Files named by a checked real project argv (the appended instruction),
/// each still open through the handle its
/// bytes were checked from. On Windows that handle shares reading only, so
/// nobody can write, replace, rename or delete the file while it is held;
/// the provider can still read it. Dropping the value releases the files.
#[derive(Debug, Default)]
pub struct HeldArgvFiles(Vec<std::fs::File>);
impl HeldArgvFiles {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Open a plain file (never through a link) with a handle that refuses
/// concurrent writing and deletion, and read at most the argv file limit
/// through that same handle.
fn open_held(path: &str) -> Result<(std::fs::File, Vec<u8>), &'static str> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        options
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|_| "ARGV_FILE")?;
    let meta = file.metadata().map_err(|_| "ARGV_FILE")?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || reparse
        || meta.len() > MAX_ARGV_FILE_BYTES
    {
        return Err("ARGV_FILE");
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_ARGV_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "ARGV_FILE")?;
    if bytes.len() as u64 > MAX_ARGV_FILE_BYTES {
        return Err("ARGV_FILE");
    }
    Ok((file, bytes))
}

fn uuid_shape(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12])
        && parts
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn model_shape(value: &str) -> bool {
    value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._[]:/@".contains(&b))
}

/// The single value after a flag: present, not empty and not another option.
fn flag_value<'a>(argv: &'a [String], index: &mut usize) -> Result<&'a str, &'static str> {
    let value = argv
        .get(*index)
        .filter(|v| !v.is_empty() && !v.starts_with('-'))
        .ok_or("ARGV_VALUE")?;
    *index += 1;
    Ok(value)
}

/// The real project rules for the final provider argv, checked immediately
/// before the spawn claim. An allow list: the first element may be the
/// prompt (never starting with `-`, always with whitespace so it can never
/// be a claude sub-command), then only these flags, each at most once and
/// never in the `--flag=value` form:
/// `--model`, `--effort`, `--permission-mode acceptEdits`,
/// `--allowedTools`/`--allowed-tools` (exactly the policy's tools),
/// `--setting-sources project,local`, `--strict-mcp-config`, `--no-chrome`,
/// `--session-id` and `--append-system-prompt-file` (exactly the approved
/// instruction bytes). `--settings` refuses: the wrapper's status line would
/// carry the Hub token.
/// Every other element, `--` included, refuses. Returns the files it read,
/// still held, or only a fixed code.
pub fn check_real_project_argv(
    argv: &[String],
    instruction_hash: &str,
    tools: &[String],
) -> Result<HeldArgvFiles, &'static str> {
    let mut held = HeldArgvFiles::default();
    let mut seen = std::collections::BTreeSet::new();
    let mut index = 0;
    if let Some(first) = argv.first()
        && !first.starts_with('-')
    {
        if !first.chars().any(char::is_whitespace) {
            return Err("ARGV_PROMPT");
        }
        index = 1;
    }
    while index < argv.len() {
        let flag = argv[index].as_str();
        index += 1;
        let canonical = if flag == "--allowed-tools" {
            "--allowedTools"
        } else {
            flag
        };
        if !seen.insert(canonical) {
            return Err("ARGV_DUPLICATE");
        }
        match canonical {
            "--strict-mcp-config" | "--no-chrome" => {}
            "--setting-sources" => {
                if flag_value(argv, &mut index)? != REAL_PROJECT_ISOLATION_ARGS[1] {
                    return Err("ARGV_SETTING_SOURCES");
                }
            }
            "--permission-mode" => {
                if flag_value(argv, &mut index)? != "acceptEdits" {
                    return Err("ARGV_PERMISSION_MODE");
                }
            }
            "--model" => {
                if !model_shape(flag_value(argv, &mut index)?) {
                    return Err("ARGV_MODEL");
                }
            }
            "--effort" => {
                if crate::config::validate_effort("claude", flag_value(argv, &mut index)?).is_err()
                {
                    return Err("ARGV_EFFORT");
                }
            }
            "--session-id" => {
                if !uuid_shape(flag_value(argv, &mut index)?) {
                    return Err("ARGV_SESSION_ID");
                }
            }
            "--allowedTools" => {
                // Claude reads every following non-option element as a tool.
                let start = index;
                while argv.get(index).is_some_and(|v| !v.starts_with('-')) {
                    index += 1;
                }
                if tools.is_empty() || argv[start..index] != *tools {
                    return Err("ARGV_ALLOWED_TOOLS");
                }
            }
            "--append-system-prompt-file" => {
                let (file, bytes) = open_held(flag_value(argv, &mut index)?)?;
                // Only the approved instruction itself, byte for byte.
                if sha(&bytes) != instruction_hash {
                    return Err("ARGV_SYSTEM_PROMPT");
                }
                held.0.push(file);
            }
            _ => return Err("ARGV_REFUSED_FLAG"),
        }
    }
    if REAL_PROJECT_REQUIRED_FLAGS
        .iter()
        .any(|flag| !seen.contains(flag))
    {
        return Err("ARGV_REQUIRED_FLAG");
    }
    if !tools.is_empty() && !seen.contains("--allowedTools") {
        return Err("ARGV_ALLOWED_TOOLS");
    }
    Ok(held)
}

/// The environment rule of a real project provider plan, checked at the exec
/// boundary: no planned value and no name the wrapper process would pass on
/// (`inherited`) under the removed names, except the kept ones; the Claude
/// folder is planned explicitly; and no launch shell payload.
pub fn check_real_project_environment(
    plan: &ProcessPlan,
    inherited: &[String],
) -> Result<(), &'static str> {
    use crate::wrapper::launch::{REAL_PROJECT_ENV_KEPT, real_project_env_removed};
    let planned = |name: &str| {
        plan.env
            .iter()
            .filter(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case(name))
            .map(|(_, value)| value.is_some())
            .collect::<Vec<_>>()
    };
    for (key, value) in &plan.env {
        let name = key.to_string_lossy();
        if value.is_some() && real_project_env_removed(&name) {
            return Err("ENV_REMOVED_NAME");
        }
    }
    for entry in inherited {
        let name = entry
            .split_once('=')
            .map_or(entry.as_str(), |(name, _)| name);
        // Every spelling planned for an inherited removed name is a removal.
        let spellings = planned(name);
        if real_project_env_removed(name) && (spellings.is_empty() || spellings.contains(&true)) {
            return Err("ENV_INHERITED_NAME");
        }
    }
    if planned(REAL_PROJECT_ENV_KEPT[0]) != [true] {
        return Err("ENV_CLAUDE_CONFIG_DIR");
    }
    for name in [
        crate::wrapper::launch_payload::RUNNER_ENV,
        crate::wrapper::launch_payload::PAYLOAD_ENV,
    ] {
        if planned(name).contains(&true) {
            return Err("ENV_LAUNCH_SHELL");
        }
    }
    Ok(())
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
    trust: Option<NativeFolderTrust>,
    /// The allowed tools this wrapper actually passes to the provider.
    launch_tools: Option<Vec<String>>,
    /// The real project policy checked at the exec boundary, kept for the
    /// collector of the same launch.
    project: Mutex<Option<FolderPolicy>>,
    /// The exact provider argv this wrapper is about to execute (after every
    /// hook and the instruction), recorded by the wrapper right before the
    /// exec boundary. A real project launch refuses without it.
    provider_argv: Mutex<Option<Vec<String>>>,
    /// The instruction file the real project argv check read,
    /// held (no shared write or delete on Windows) from that check until the
    /// provider run has finished.
    held_argv_files: Mutex<Option<HeldArgvFiles>>,
}
impl DurableExecutionOwner {
    /// Release the files held since the real project argv check. Called when
    /// the provider run has finished (and safe to call more than once), so
    /// the wrapper can delete its private files afterwards.
    pub fn release_argv_files(&self) {
        if let Ok(mut held) = self.held_argv_files.lock() {
            held.take();
        }
    }
    /// The number of files currently held (diagnostics and tests).
    pub fn held_argv_file_count(&self) -> usize {
        self.held_argv_files
            .lock()
            .ok()
            .and_then(|held| held.as_ref().map(HeldArgvFiles::len))
            .unwrap_or(0)
    }
    pub fn instruction_hash(&self) -> &str {
        &self.ticket.binding.instruction_hash
    }
    /// A real project (`folder-diff` collector) launch.
    pub fn real_project(&self) -> bool {
        self.grant.folder_diff()
    }
    /// Record the final provider argv (the resolved command's arguments,
    /// whether or not a launch shell runs it). The latest call wins; the real
    /// project check at the exec boundary reads it.
    pub fn observe_provider_argv(&self, argv: &[String]) {
        if let Ok(mut slot) = self.provider_argv.lock() {
            *slot = Some(argv.to_vec());
        }
    }
    /// The parsed `--allowed-tools` of this wrapper launch. A real project
    /// launch refuses unless they equal its pinned policy's `allowed_tools`.
    pub fn with_launch_tools(mut self, tools: Vec<String>) -> Self {
        self.launch_tools = Some(tools);
        self
    }
    /// The same native Claude trust writer ordinary child launches use, built
    /// by the caller from its explicit runtime paths and vendor home. It is
    /// used only for the real bounded launch; without it that launch refuses.
    pub fn with_folder_trust(mut self, trust: NativeFolderTrust) -> Self {
        self.trust = Some(trust);
        self
    }
    /// The first three conditions, from the freshly reloaded grant and this
    /// ticket: the exact claude/claude-hiroshi pair, a collector payload and
    /// bounded-edit. The fourth, an enabled current policy of the real bounded
    /// profile, is checked in `trust_folder`; when it fails nothing is written
    /// and the launch is refused. Codex, synthetic, the synthetic owned
    /// profile and default launches never reach the writer.
    /// The real project profile (folder-diff collector) has the same three
    /// conditions with a folder-diff grant instead of a payload; its fourth
    /// is the pinned real project policy checked in `before_spawn`.
    fn wants_folder_trust(&self, live: &ApprovedGrant) -> bool {
        real_bounded_binding(&self.ticket.binding)
            && ((self.grant.payload.is_some() && live.payload.is_some())
                || (self.grant.folder_diff() && live.folder_diff()))
            && self.grant.launch_mode == LaunchMode::BoundedEdit
            && live.launch_mode == LaunchMode::BoundedEdit
    }
    /// Trust exactly the approved folder in the provider's own Claude config
    /// and refuse on any failure. Two records, both before the claim: an
    /// intent (target, planned key, time) before the vendor write, so a write
    /// can never exist without a record, then the result after it.
    /// `precondition` is the profile and baseline decision made by the
    /// caller: real bounded requires its admitted policy and an empty
    /// folder; real project its pinned policy and the pinned baseline.
    fn trust_folder(
        &self,
        store: &mut OwnerStore,
        ticket: &OwnerTicket,
        process: &ProcessPlan,
        at: u64,
        precondition: Result<(), &'static str>,
    ) -> io::Result<()> {
        // The provider's exact environment: the wrapper builds a complete map.
        let environment = process
            .env
            .iter()
            .filter_map(|(key, value)| {
                Some(format!("{}={}", key.to_str()?, value.as_ref()?.to_str()?))
            })
            .collect::<Vec<_>>();
        // One trust decision per attempt, before its claim. Any earlier row,
        // in particular an intent without a result, refuses re-entry before
        // the writer is reached again.
        let earlier = store
            .query_folder_trust(&ticket.binding)
            .map_err(|_| failure())?;
        if !store.query(&ticket.binding).map_err(|_| failure())?["claimed_session"].is_null()
            || !earlier["intent"].is_null()
            || !earlier["result"].is_null()
        {
            return Err(failure());
        }
        let target = self.grant.approved_cwd.to_string_lossy().into_owned();
        let planned_key = NativeFolderTrust::exact_claude_key(&self.grant.approved_cwd).ok();
        let flags = json!({"kind":"folder_trust","ticket":ticket,"target":target,
            "scope":"approved-cwd-only","tool_grants":[],"risk_confirmed":false,
            "effects_verified":false,"task_outcome_verified":false});
        let outcome = if let Err(code) = precondition {
            Err(code)
        } else {
            match (&self.trust, &planned_key) {
                (None, _) => Err("FOLDER_TRUST_UNAVAILABLE"),
                (_, None) => Err("FOLDER_TRUST_WRITE"),
                (Some(trust), Some(key)) => {
                    let mut intent = flags.clone();
                    intent["phase"] = json!("intent");
                    intent["result"] = json!("pending");
                    intent["key"] = json!(key);
                    intent["observed_at_ms"] = json!(at);
                    store
                        .record_folder_trust(ticket, &intent)
                        .map_err(|_| failure())?;
                    trust
                        .grant_exact_claude(&environment, &self.grant.approved_cwd)
                        .map_err(|_| "FOLDER_TRUST_WRITE")
                }
            }
        };
        let (result, key, written, existing, reason) = match &outcome {
            Ok(r) => (
                if r.written {
                    "written"
                } else {
                    "already-trusted"
                },
                json!(r.key),
                r.written,
                json!(r.existing),
                Value::Null,
            ),
            Err(code) => ("failed", Value::Null, false, Value::Null, json!(code)),
        };
        let mut document = flags;
        for (field, value) in [
            ("phase", json!("result")),
            ("planned_key", json!(planned_key)),
            ("key", key),
            ("written", json!(written)),
            ("existing", existing),
            ("result", json!(result)),
            ("reason", reason),
            ("observed_at_ms", json!(now_ms().unwrap_or(at).max(at))),
        ] {
            document[field] = value;
        }
        store
            .record_folder_trust(ticket, &document)
            .map_err(|_| failure())?;
        outcome.map(|_| ()).map_err(|_| failure())
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
    /// Trusted local collector for the real project profile (`folder-diff`).
    /// After recorded cessation it records the after snapshot, writes every
    /// difference once to the change list artifact, builds the envelope
    /// itself (outcome always unknown, synthetic false, payload = summary
    /// only) and records the collection. Effects: `folder-no-change`,
    /// `folder-scoped-change` or `folder-unexpected-change`; anything not
    /// fully recorded is unexpected. Only storage failures are errors, and
    /// no flag here verifies the task or the effects.
    fn collect_folder_diff(&self, store: &mut OwnerStore) -> io::Result<()> {
        let Some(changes_permit) = &self.grant.changes else {
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
        let allowed = self
            .project
            .lock()
            .map_err(|_| failure())?
            .as_ref()
            .map(|policy| policy.allowed_paths.clone());
        let records = match (&baseline, &after) {
            (Some(old), Ok(new)) => Some(folder::change_records(old, new)),
            _ => None,
        };
        let (mut effects, mut reason, mut reason_detail) = match (&baseline, &after, &allowed) {
            (Some(old), Ok(new), Some(allowed)) => {
                let (effects, code) = folder::classify(old, new, allowed);
                (effects, json!(code), Value::Null)
            }
            (_, Err(error), _) => (
                folder::UNEXPECTED_CHANGE,
                json!("AFTER_SNAPSHOT_FAILED"),
                json!(error.code),
            ),
            (None, _, _) => (
                folder::UNEXPECTED_CHANGE,
                json!("BEFORE_UNAVAILABLE"),
                Value::Null,
            ),
            (_, _, None) => (
                folder::UNEXPECTED_CHANGE,
                json!("POLICY_UNAVAILABLE"),
                Value::Null,
            ),
        };
        let mut change_count = None;
        let mut changes_sha256 = Value::Null;
        let mut changes_bytes = Value::Null;
        if let Some(records) = &records {
            let document = json!({"kind":"folder_changes","ticket":t,
                "change_count":records.len(),"changes":records,
                "verification_scope":folder::SCOPE,"effects_verified":false,
                "task_outcome_verified":false,"remote_jobs_stopped":false});
            let bytes = serde_json::to_vec(&document).map_err(|_| failure())?;
            if bytes.len() as u64 > folder::MAX_CHANGES_BYTES {
                effects = folder::UNEXPECTED_CHANGE;
                note_reason(
                    &mut reason,
                    &mut reason_detail,
                    "CHANGES_LIMIT",
                    json!(null),
                );
            } else {
                match changes_permit.create_exclusive(&bytes) {
                    // The count is reported only together with the SHA256 of
                    // the list that holds it; an unwritten list has neither,
                    // and the folder reason says why.
                    Ok(()) => {
                        change_count = Some(records.len());
                        changes_sha256 = json!(sha(&bytes));
                        changes_bytes = json!(bytes.len());
                    }
                    Err(error) => {
                        effects = folder::UNEXPECTED_CHANGE;
                        let code = if error.code == "ARTIFACT_EXISTS" {
                            "CHANGES_PREEXISTING"
                        } else {
                            "CHANGES_WRITE"
                        };
                        note_reason(&mut reason, &mut reason_detail, code, json!(error.code));
                    }
                }
            }
        }
        let mut result_sha256 = Value::Null;
        let mut outcome_sequence = Value::Null;
        if std::fs::symlink_metadata(self.grant.result.approved_path()).is_ok() {
            // Never overwrite or adopt an existing result, whoever wrote it.
            effects = folder::UNEXPECTED_CHANGE;
            note_reason(
                &mut reason,
                &mut reason_detail,
                "RESULT_PREEXISTING",
                json!(null),
            );
        } else {
            let envelope = json!({"binding":t.binding,"owner_id":t.owner_id,
                "owner_epoch":t.owner_epoch,"instruction_hash":t.binding.instruction_hash,
                "outcome":"unknown","effects":effects,"synthetic":false,
                "payload":{"kind":"folder_change_summary","change_count":change_count,
                    "changes_sha256":changes_sha256,"changes_bytes":changes_bytes,
                    "allowed_paths":allowed,"folder_reason":reason}});
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
                    let next = store.query(&t.binding).map_err(|_| failure())?["sequence"]
                        .as_u64()
                        .ok_or_else(failure)?
                        .checked_add(1)
                        .ok_or_else(failure)?;
                    match store.append_outcome(t, next, &self.grant.result, &digest) {
                        Ok(receipt) => outcome_sequence = receipt["sequence"].clone(),
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
        let collection = json!({"kind":"folder_collection","phase":"collection","ticket":t,
            "collector":"folder-diff","effects":effects,"reason":reason,
            "reason_detail":reason_detail,"change_count":change_count,
            "changes_sha256":changes_sha256,"changes_bytes":changes_bytes,
            "allowed_paths":allowed,"result_sha256":result_sha256,
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
            trust: None,
            launch_tools: None,
            project: Mutex::new(None),
            provider_argv: Mutex::new(None),
            held_argv_files: Mutex::new(None),
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
        if live.payload.is_some() != self.grant.payload.is_some()
            || live.folder_diff() != self.grant.folder_diff()
        {
            return Err(failure());
        }
        // Real project: the enabled, current policy of this exact binding,
        // equal to the bytes the Hub pinned at admission, and exactly its
        // allowed tools on this launch (none can be added by a caller).
        let mut held_files = None;
        let project = if self.grant.folder_diff() {
            let policy = real_project_launch_policy(&self.root, &self.ticket.binding, at)
                .ok_or_else(failure)?;
            if self.launch_tools.as_deref() != Some(policy.allowed_tools.as_slice()) {
                return Err(failure());
            }
            // The final argv, an allow list: no user settings, no MCP server,
            // no Chrome, no settings file, no other system prompt. The files it read stay held.
            let argv = self
                .provider_argv
                .lock()
                .map_err(|_| failure())?
                .clone()
                .ok_or_else(failure)?;
            held_files = Some(
                check_real_project_argv(
                    &argv,
                    &self.ticket.binding.instruction_hash,
                    &policy.allowed_tools,
                )
                .map_err(|_| failure())?,
            );
            // The environment: no Claude, Anthropic or many-ai-cli name
            // except the kept ones, the subscription's Claude folder, and
            // no launch shell.
            let inherited = std::env::vars_os()
                .filter_map(|(name, _)| name.into_string().ok())
                .collect::<Vec<_>>();
            check_real_project_environment(process, &inherited).map_err(|_| failure())?;
            Some(policy)
        } else {
            None
        };
        let mut store = self.store.lock().map_err(|_| failure())?;
        let ticket = store
            .begin_owner(
                &self.ticket.binding,
                &self.ticket.owner_id,
                self.ticket.owner_epoch,
                at,
            )
            .map_err(|_| failure())?;
        let mut baseline_empty = false;
        if let Some(policy) = &project {
            // The whole work folder must equal the operator-pinned baseline
            // (digest and entry count): plain single-link files and folders,
            // no stream, link or protected name. Recorded before the claim.
            // No instruction file above it either (CLAUDE.md, CLAUDE.local.md,
            // .claude, AGENTS.md up to the drive root).
            let entries = folder::check_no_ancestor_instructions(&self.grant.approved_cwd)
                .and_then(|()| folder::snapshot(&self.grant.approved_cwd))
                .and_then(|entries| {
                    folder::check_project_baseline(
                        &entries,
                        &policy.baseline_sha256,
                        policy.baseline_entry_count,
                    )
                    .map(|_| entries)
                })
                .map_err(|_| failure())?;
            let mut document = folder::snapshot_document(&ticket, "before", &Ok(entries), at);
            document["baseline_sha256"] = json!(policy.baseline_sha256);
            document["allowed_paths"] = json!(policy.allowed_paths);
            store
                .record_folder(&ticket, "before", &document)
                .map_err(|_| failure())?;
            *self.project.lock().map_err(|_| failure())? = Some(policy.clone());
        } else if self.grant.payload.is_some() {
            // Collector grants persist the whole approved folder baseline
            // before the claim. An oversized, unreadable or aliased folder
            // refuses the launch; nothing is spawned without a baseline.
            let entries = folder::snapshot(&self.grant.approved_cwd)
                .and_then(|entries| folder::require_plain(&entries).map(|_| entries))
                .map_err(|_| failure())?;
            baseline_empty = folder::only_the_folder(&entries);
            let document = folder::snapshot_document(&ticket, "before", &Ok(entries), at);
            store
                .record_folder(&ticket, "before", &document)
                .map_err(|_| failure())?;
        }
        if self.wants_folder_trust(&live) {
            if let Some(policy) = &project {
                // Trust is written only over the recorded pinned baseline,
                // and the folder must still equal it right before the claim.
                self.trust_folder(&mut store, &ticket, process, at, Ok(()))?;
                if folder::check_no_ancestor_instructions(&self.grant.approved_cwd).is_err()
                    || !folder::snapshot(&self.grant.approved_cwd).is_ok_and(|entries| {
                        folder::check_project_baseline(
                            &entries,
                            &policy.baseline_sha256,
                            policy.baseline_entry_count,
                        )
                        .is_ok()
                    })
                {
                    return Err(failure());
                }
            } else {
                // Immediately before the claim and spawn, after the baseline:
                // a Claude config that happened to live inside the approved
                // folder would show up as a change, never be hidden by the
                // baseline. Trust is written only over the recorded empty
                // baseline.
                let precondition = if !real_bounded_launch_admitted(&self.root, &ticket.binding, at)
                {
                    Err("FOLDER_TRUST_PROFILE")
                } else if !baseline_empty {
                    Err("FOLDER_TRUST_NOT_EMPTY")
                } else {
                    Ok(())
                };
                self.trust_folder(&mut store, &ticket, process, at, precondition)?;
                // Still empty after the write, right before the claim;
                // otherwise the trusted folder would expose content nobody
                // checked.
                if !folder::snapshot(&self.grant.approved_cwd)
                    .is_ok_and(|entries| folder::only_the_folder(&entries))
                {
                    return Err(failure());
                }
            }
        } else if project.is_some() {
            // A real project launch always runs with bounded-edit and trust.
            return Err(failure());
        }
        // The persisted one-use claim closes the crash/replay boundary before
        // factory.spawn. Re-entering with the same ticket is never a retry.
        store
            .claim_spawn(&ticket, session_id, at)
            .map_err(|_| failure())?;
        // Keep the checked instruction file held through the
        // provider run; a refusal above has already released them.
        if let Some(files) = held_files {
            *self.held_argv_files.lock().map_err(|_| failure())? = Some(files);
        }
        Ok(())
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
        // The provider run is over: the wrapper may remove its private files.
        self.release_argv_files();
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
        if self.grant.folder_diff() {
            // Same rule as the payload collector: no provider-written result
            // is read, and collection runs only after persisted cessation.
            return if ceased {
                self.collect_folder_diff(&mut store)
            } else {
                Ok(())
            };
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

#[cfg(test)]
mod real_project_argv_tests {
    use super::*;

    const SESSION: &str = "00000000-0000-4000-8000-000000000000";

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }
    /// The shape the Hub and wrapper build for a real project launch.
    fn base() -> Vec<String> {
        strings(&[
            "synthetic instruction text",
            "--session-id",
            SESSION,
            "--model",
            "synthetic-model",
            "--permission-mode",
            "acceptEdits",
            "--effort",
            "high",
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--no-chrome",
        ])
    }
    fn with(extra: &[&str]) -> Vec<String> {
        let mut argv = base();
        argv.extend(strings(extra));
        argv
    }

    /// C4 re-review M1 (reverse of the argv reproduction): the checker is an
    /// allow list. Every form the old deny list accepted, the end of options
    /// marker and `=` forms are refused.
    #[test]
    fn the_real_project_argv_check_is_an_allow_list() {
        assert!(check_real_project_argv(&base(), "x", &[]).is_ok());
        let cases: Vec<(&str, Vec<&str>)> = vec![
            ("managed-settings", vec!["--managed-settings", "{}"]),
            (
                "project-config-root",
                vec!["--project-config-root", "C:\\synthetic"],
            ),
            (
                "plugin-dir-no-mcp",
                vec!["--plugin-dir-no-mcp", "C:\\synthetic"],
            ),
            (
                "plugin-url",
                vec!["--plugin-url", "https://example.invalid/p.zip"],
            ),
            (
                "append-subagent-system-prompt-file",
                vec!["--append-subagent-system-prompt-file", "C:\\synthetic.md"],
            ),
            (
                "plan-mode-instructions",
                vec!["--plan-mode-instructions", "synthetic"],
            ),
            ("prefill", vec!["--prefill", "synthetic"]),
            ("resume short", vec!["-r", "synthetic"]),
            ("continue short", vec!["-c"]),
            ("remote-control alias", vec!["--rc"]),
            ("channels", vec!["--channels", "synthetic"]),
            ("agent", vec!["--agent", "synthetic"]),
            (
                "inherit-permission-mode",
                vec!["--inherit-permission-mode", "bypassPermissions"],
            ),
            ("enable-auto-mode", vec!["--enable-auto-mode"]),
            ("tools", vec!["--tools", "Bash"]),
            ("disallowed-tools kebab", vec!["--disallowed-tools", "Read"]),
            ("mcp-config", vec!["--mcp-config", "{}"]),
            ("add-dir", vec!["--add-dir", "C:\\synthetic"]),
            ("chrome", vec!["--chrome"]),
            ("print", vec!["-p"]),
            ("equals form", vec!["--model=synthetic"]),
            ("equals settings", vec!["--settings={}"]),
            ("end of options", vec!["--"]),
            ("lone dash", vec!["-"]),
            ("stray positional", vec!["synthetic word"]),
            ("duplicate flag", vec!["--strict-mcp-config"]),
            (
                "second permission mode",
                vec!["--permission-mode", "acceptEdits"],
            ),
            ("allowedTools not in policy", vec!["--allowedTools", "Bash"]),
            (
                "allowed-tools not in policy",
                vec!["--allowed-tools", "Read"],
            ),
        ];
        let mut accepted = Vec::new();
        for (label, extra) in &cases {
            if check_real_project_argv(&with(extra), "x", &[]).is_ok() {
                accepted.push(*label);
            }
        }
        // `--` ends claude's options: the isolation flags after it would be
        // prompt words.
        let mut dashed = strings(&["synthetic instruction text", "--"]);
        dashed.extend(base().into_iter().skip(1));
        if check_real_project_argv(&dashed, "x", &[]).is_ok() {
            accepted.push("end-of-options marker before isolation flags");
        }
        assert!(accepted.is_empty(), "accepted: {accepted:?}");
    }

    /// Required flags, flag values and the prompt position.
    #[test]
    fn the_real_project_argv_requires_the_isolation_and_exact_values() {
        let without = |flag: &str, takes_value: bool| {
            let mut argv = base();
            let at = argv.iter().position(|a| a == flag).unwrap();
            argv.drain(at..at + 1 + usize::from(takes_value));
            argv
        };
        for (flag, takes_value) in [
            ("--setting-sources", true),
            ("--strict-mcp-config", false),
            ("--no-chrome", false),
            ("--permission-mode", true),
        ] {
            assert!(
                check_real_project_argv(&without(flag, takes_value), "x", &[]).is_err(),
                "{flag} is required"
            );
        }
        // Optional ones may be absent.
        for (flag, takes_value) in [
            ("--model", true),
            ("--effort", true),
            ("--session-id", true),
        ] {
            assert!(
                check_real_project_argv(&without(flag, takes_value), "x", &[]).is_ok(),
                "{flag} is optional"
            );
        }
        let replaced = |flag: &str, value: &str| {
            let mut argv = base();
            let at = argv.iter().position(|a| a == flag).unwrap();
            argv[at + 1] = value.into();
            argv
        };
        for (flag, value) in [
            ("--setting-sources", "user,project,local"),
            ("--permission-mode", "bypassPermissions"),
            ("--effort", "unbounded"),
            ("--session-id", "not-a-uuid"),
            ("--model", "--settings"),
            ("--model", "two words"),
        ] {
            assert!(
                check_real_project_argv(&replaced(flag, value), "x", &[]).is_err(),
                "{flag} {value}"
            );
        }
        // The prompt is only the first element and always has whitespace,
        // so it can never be a claude sub-command name.
        let mut argv = base();
        argv[0] = "mcp".into();
        assert!(check_real_project_argv(&argv, "x", &[]).is_err());
        assert!(check_real_project_argv(&base()[1..], "x", &[]).is_ok());
        // The policy's tools: exactly once, exactly the policy values, under
        // either spelling; none without tools.
        let tools = strings(&["Bash(python -B -m pytest:*)"]);
        assert!(check_real_project_argv(&base(), "x", &tools).is_err());
        for flag in ["--allowedTools", "--allowed-tools"] {
            assert!(
                check_real_project_argv(&with(&[flag, "Bash(python -B -m pytest:*)"]), "x", &tools)
                    .is_ok(),
                "{flag}"
            );
            assert!(
                check_real_project_argv(
                    &with(&[flag, "Bash(python -B -m pytest:*)", "Read"]),
                    "x",
                    &tools
                )
                .is_err()
            );
        }
        assert!(
            check_real_project_argv(
                &with(&[
                    "--allowedTools",
                    "Bash(python -B -m pytest:*)",
                    "--allowed-tools",
                    "Bash(python -B -m pytest:*)"
                ]),
                "x",
                &tools
            )
            .is_err()
        );
    }

    /// C4 re-check: a real project argv never names a settings file (the
    /// wrapper's status line would carry the Hub token), whatever it holds.
    #[test]
    fn a_real_project_argv_never_names_a_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"{}").unwrap();
        let argv = with(&["--settings", path.to_str().unwrap()]);
        assert_eq!(
            check_real_project_argv(&argv, "x", &[]).err(),
            Some("ARGV_REFUSED_FLAG")
        );
    }

    /// C4 re-review M2 (exec boundary): an inherited removed name must be
    /// planned as a removal under every spelling; kept names may stay.
    #[test]
    fn the_exec_boundary_checks_planned_and_inherited_names() {
        let plan = |env: &[(&str, Option<&str>)]| ProcessPlan {
            executable: "synthetic".into(),
            args: vec![],
            cwd: std::path::PathBuf::from("synthetic"),
            env: env
                .iter()
                .map(|(k, v)| ((*k).into(), v.map(Into::into)))
                .collect(),
            stdin: vec![],
            timeout: std::time::Duration::ZERO,
            output_cap: 1,
            pipe_drain_timeout: std::time::Duration::from_millis(1),
        };
        let good = plan(&[
            ("CLAUDE_CONFIG_DIR", Some("C:\\synthetic")),
            ("CLAUDE_PID", None),
            ("MANY_AI_CLI_SESSION_ID", Some("1")),
            ("MANY_AI_CLI_HUB_TOKEN", None),
        ]);
        let inherited = strings(&["CLAUDE_PID=1", "PATH", "MANY_AI_CLI_SESSION_ID"]);
        assert_eq!(check_real_project_environment(&good, &inherited), Ok(()));
        // An inherited removed name the plan does not remove.
        let more = strings(&["CLAUDE_PID", "ANTHROPIC_BASE_URL"]);
        assert_eq!(
            check_real_project_environment(&good, &more),
            Err("ENV_INHERITED_NAME")
        );
        // The Hub token is never planned for a real project provider.
        let token = plan(&[
            ("CLAUDE_CONFIG_DIR", Some(r"C:\synthetic")),
            ("MANY_AI_CLI_HUB_TOKEN", Some("synthetic")),
        ]);
        assert_eq!(
            check_real_project_environment(&token, &[]),
            Err("ENV_REMOVED_NAME")
        );
        let missing = plan(&[("CLAUDE_PID", None)]);
        assert_eq!(
            check_real_project_environment(&missing, &[]),
            Err("ENV_CLAUDE_CONFIG_DIR")
        );
    }

    /// C4 re-review L1: the instruction file is read
    /// through a handle that shares neither writing nor deletion, and that
    /// handle stays open until it is released.
    #[cfg(windows)]
    #[test]
    fn the_checked_files_cannot_be_replaced_while_held() {
        let dir = tempfile::tempdir().unwrap();
        let instruction = dir.path().join("instruction.md");
        std::fs::write(&instruction, b"synthetic approved instruction").unwrap();
        let argv = with(&["--append-system-prompt-file", instruction.to_str().unwrap()]);
        let held =
            check_real_project_argv(&argv, &sha(b"synthetic approved instruction"), &[]).unwrap();
        for path in [&instruction] {
            assert!(std::fs::read(path).is_ok(), "still readable");
            assert!(std::fs::write(path, b"replaced").is_err(), "no write");
            assert!(std::fs::remove_file(path).is_err(), "no delete");
            assert!(
                std::fs::rename(path, dir.path().join("moved")).is_err(),
                "no rename"
            );
        }
        drop(held);
        std::fs::remove_file(&instruction).unwrap();
        // A wrong instruction digest is refused.
        std::fs::write(&instruction, b"other bytes").unwrap();
        let argv = with(&["--append-system-prompt-file", instruction.to_str().unwrap()]);
        assert!(
            check_real_project_argv(&argv, &sha(b"synthetic approved instruction"), &[]).is_err()
        );
    }
}
