//! How one headless run is paid for (Issue #14). Every new run uses the plan
//! login unless a person explicitly picks API key billing for that run.
//!
//! API key billing never hands a key to the Hub. The private file
//! `headless-billing.json` names, per profile, the command a person already
//! uses to fetch a key (a "recipe"). The wrapper merges that command into the
//! session-owned Claude settings as `apiKeyHelper`; Claude Code runs it and
//! keeps its output. Nothing here runs the command, reads its output, or puts
//! the command into Debug output, errors, the wire, the database or a log.
//!
//! Only a built-in provider whose shipped headless output is Claude Code's
//! stream-json supports it (the helper, `--bare` and `--max-budget-usd` are
//! that CLI's). Registered providers and overrides never qualify.
use super::{Resource, RuntimePaths};
use crate::files::safe_fs::Dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fmt,
    io::{self, Read},
};

pub const PLAN_LOGIN: &str = "plan-login";
pub const API_KEY: &str = "api-key";
const MAX_CONFIG_BYTES: usize = 128 * 1024;
const MAX_PROFILES: usize = 64;

/// What the confirmation dialog may show about a profile. No command field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BillingProfileSummary {
    pub id: String,
    pub display_name: String,
    pub provider: String,
}

/// The selection a person confirmed. Safe to show and to carry through the
/// spawn: it must never acquire a command, key or settings field.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedBilling {
    pub mode: String,
    pub profile_id: String,
    pub display_name: String,
    pub provider: String,
    /// SHA-256 of the private file when the selection was resolved. The launch
    /// refuses a file that changed after the person confirmed it.
    pub revision: String,
    pub max_budget_usd: Option<f64>,
}
impl Default for ResolvedBilling {
    fn default() -> Self {
        Self {
            mode: PLAN_LOGIN.into(),
            profile_id: String::new(),
            display_name: String::new(),
            provider: String::new(),
            revision: String::new(),
            max_budget_usd: None,
        }
    }
}
impl ResolvedBilling {
    pub fn is_api(&self) -> bool {
        self.mode == API_KEY
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    profiles: Vec<Recipe>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    id: String,
    display_name: String,
    provider: String,
    command: String,
}

/// Intentionally neither Debug, Clone nor Serialize: the only use is merging
/// the command into a session-owned private Claude settings document.
pub struct PrivateRecipe {
    command: String,
}
impl PrivateRecipe {
    pub(crate) fn insert_settings(
        &self,
        settings: &mut serde_json::Map<String, serde_json::Value>,
    ) {
        settings.insert("apiKeyHelper".into(), self.command.clone().into());
    }
}

/// Fixed classes only. No path, command, value or provider output is ever
/// formatted into these messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BillingError {
    InvalidSelection,
    HumanConfirmationRequired,
    InvalidProfile,
    UnknownProfile,
    PrivateConfig,
    RevisionChanged,
    AuthConflict,
    UnsupportedVersion,
    PlanLoginWouldUseKey,
}
impl fmt::Display for BillingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidSelection => {
                "invalid headless billing selection: API key billing needs a headless run of a provider that supports it, a profile and a positive budget"
            }
            Self::HumanConfirmationRequired => {
                "API key billing needs a person to approve the spawn confirmation"
            }
            Self::InvalidProfile => "invalid headless billing profile configuration",
            Self::UnknownProfile => "headless billing profile is unavailable",
            Self::PrivateConfig => {
                "headless billing private configuration is unavailable or not private to this user"
            }
            Self::RevisionChanged => {
                "headless billing profile changed; confirm the selection again"
            }
            Self::AuthConflict => {
                "headless billing authentication configuration conflicts with API key billing"
            }
            Self::UnsupportedVersion => "headless billing Claude Code version is not verified",
            Self::PlanLoginWouldUseKey => {
                "plan login was selected, but the environment or the Claude settings provide an API key, so the run would be billed to that key; remove it or choose API key billing"
            }
        })
    }
}
impl std::error::Error for BillingError {}
impl From<BillingError> for io::Error {
    fn from(error: BillingError) -> Self {
        io::Error::new(io::ErrorKind::InvalidInput, error)
    }
}

/// Whether `provider` can be billed through an API key helper.
pub fn api_key_supported(provider: &str) -> bool {
    crate::profile::declared::builtin(provider)
        .and_then(|definition| definition.launch)
        .and_then(|launch| launch.headless)
        .is_some_and(|headless| headless.format == super::HEADLESS_FORMAT_CLAUDE_STREAM_JSON)
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

pub fn valid_budget(budget: Option<f64>) -> bool {
    budget.is_some_and(|value| value.is_finite() && value > 0.0)
}

/// Checks a requested selection without reading any file. An empty mode is
/// the plan login; the plan login carries no profile and no budget.
pub fn validate_selection(
    provider: &str,
    execution_mode: &str,
    mode: &str,
    profile: &str,
    budget: Option<f64>,
    route: &str,
) -> Result<ResolvedBilling, BillingError> {
    let mode = mode.trim();
    let mode = if mode.is_empty() { PLAN_LOGIN } else { mode };
    match mode {
        PLAN_LOGIN if profile.is_empty() && budget.is_none() => Ok(ResolvedBilling {
            provider: provider.into(),
            ..Default::default()
        }),
        API_KEY
            if api_key_supported(provider)
                && super::is_headless_execution_mode(execution_mode)
                && valid_id(profile)
                && valid_budget(budget)
                && matches!(route, "" | "default") =>
        {
            Ok(ResolvedBilling {
                mode: API_KEY.into(),
                profile_id: profile.into(),
                provider: provider.into(),
                max_budget_usd: budget,
                ..Default::default()
            })
        }
        _ => Err(BillingError::InvalidSelection),
    }
}

fn read_private(paths: &RuntimePaths) -> Result<Option<Vec<u8>>, BillingError> {
    let path = paths.resource(Resource::HeadlessBilling);
    let parent = path.parent().ok_or(BillingError::PrivateConfig)?;
    let directory = match Dir::open(parent) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(BillingError::PrivateConfig),
    };
    let file = match directory.open_file("headless-billing.json", false) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(BillingError::PrivateConfig),
    };
    let metadata = file.metadata().map_err(|_| BillingError::PrivateConfig)?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES as u64 {
        return Err(BillingError::PrivateConfig);
    }
    // Read-only checks: never silently repair the person's permissions.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions and cannot fail.
        let euid = unsafe { libc::geteuid() };
        if metadata.mode() & 0o077 != 0 || metadata.uid() != euid || metadata.nlink() != 1 {
            return Err(BillingError::PrivateConfig);
        }
    }
    #[cfg(windows)]
    {
        use super::private_io::PrivateSecurity;
        let trusted =
            PrivateSecurity::trusted_owner(&file).map_err(|_| BillingError::PrivateConfig)?;
        let wide =
            PrivateSecurity::grants_others(&file).map_err(|_| BillingError::PrivateConfig)?;
        if !trusted || wide {
            return Err(BillingError::PrivateConfig);
        }
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BillingError::PrivateConfig)?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(BillingError::PrivateConfig);
    }
    Ok(Some(bytes))
}

fn document(paths: &RuntimePaths) -> Result<Option<(Document, String)>, BillingError> {
    let Some(bytes) = read_private(paths)? else {
        return Ok(None);
    };
    let doc: Document = serde_json::from_slice(&bytes).map_err(|_| BillingError::InvalidProfile)?;
    if doc.version != 1 || doc.profiles.len() > MAX_PROFILES {
        return Err(BillingError::InvalidProfile);
    }
    let mut ids = BTreeSet::new();
    for profile in &doc.profiles {
        // A recognizable key literal means the file holds a key, not a way to
        // fetch one. Refuse the whole file without echoing anything.
        if [&profile.id, &profile.display_name, &profile.command]
            .into_iter()
            .any(|value| crate::storage::mask_secrets(value) != *value)
            || !valid_id(&profile.id)
            || !ids.insert(profile.id.as_str())
            || !api_key_supported(&profile.provider)
            || profile.display_name.trim().is_empty()
            || profile.display_name.len() > 120
            || profile.display_name.chars().any(char::is_control)
            || profile.command.trim().is_empty()
            || profile.command.len() > 16 * 1024
            || profile.command.contains(['\0', '\r', '\n'])
        {
            return Err(BillingError::InvalidProfile);
        }
    }
    let revision = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(Some((doc, revision)))
}

/// The profiles a person may pick, and the revision they are shown at. No
/// file means no profiles (API key billing stays unavailable).
pub fn disclosure(
    paths: &RuntimePaths,
) -> Result<(Vec<BillingProfileSummary>, String), BillingError> {
    Ok(document(paths)?
        .map(|(doc, revision)| {
            (
                doc.profiles
                    .into_iter()
                    .map(|profile| BillingProfileSummary {
                        id: profile.id,
                        display_name: profile.display_name,
                        provider: profile.provider,
                    })
                    .collect(),
                revision,
            )
        })
        .unwrap_or_default())
}

pub fn profiles(paths: &RuntimePaths) -> Result<Vec<BillingProfileSummary>, BillingError> {
    disclosure(paths).map(|(profiles, _)| profiles)
}

/// What a spawn confirmation dialog shows about billing. A broken or
/// non-private file leaves the plan login usable and marks API key billing
/// unavailable instead of failing every confirmation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BillingDisclosure {
    pub profiles: Vec<BillingProfileSummary>,
    pub revision: String,
    pub unavailable: Option<BillingError>,
}
pub fn presentation(paths: &RuntimePaths) -> BillingDisclosure {
    match disclosure(paths) {
        Ok((profiles, revision)) => BillingDisclosure {
            profiles,
            revision,
            unavailable: None,
        },
        Err(error) => BillingDisclosure {
            unavailable: Some(error),
            ..Default::default()
        },
    }
}

/// Fills the display name and pins the current revision. A plan-login
/// selection is returned unchanged without reading the file.
pub fn resolve(
    paths: &RuntimePaths,
    mut selected: ResolvedBilling,
) -> Result<ResolvedBilling, BillingError> {
    if !selected.is_api() {
        return Ok(selected);
    }
    let (doc, revision) = document(paths)?.ok_or(BillingError::UnknownProfile)?;
    let profile = doc
        .profiles
        .into_iter()
        .find(|p| p.id == selected.profile_id && p.provider == selected.provider)
        .ok_or(BillingError::UnknownProfile)?;
    selected.display_name = profile.display_name;
    selected.revision = revision;
    Ok(selected)
}

/// The recipe of a confirmed selection, only while the file still has the
/// revision the person confirmed.
pub fn recipe(
    paths: &RuntimePaths,
    selected: &ResolvedBilling,
) -> Result<PrivateRecipe, BillingError> {
    if !selected.is_api() || selected.revision.is_empty() {
        return Err(BillingError::RevisionChanged);
    }
    let (doc, revision) = document(paths)?.ok_or(BillingError::UnknownProfile)?;
    if revision != selected.revision {
        return Err(BillingError::RevisionChanged);
    }
    let profile = doc
        .profiles
        .into_iter()
        .find(|p| p.id == selected.profile_id && p.provider == selected.provider)
        .ok_or(BillingError::UnknownProfile)?;
    Ok(PrivateRecipe {
        command: profile.command,
    })
}

pub fn verify_revision(
    paths: &RuntimePaths,
    selected: &ResolvedBilling,
) -> Result<(), BillingError> {
    if selected.is_api() {
        recipe(paths, selected)?;
    }
    Ok(())
}

/// Names only; values are never read into a message. A configured competing
/// credential or route source is refused, including empty values whose CLI
/// meaning varies by version.
pub fn auth_environment_conflicts(environment: &[String]) -> bool {
    environment
        .iter()
        .filter_map(|e| e.split_once('=').map(|(k, _)| k.to_ascii_uppercase()))
        .any(|key| conflicting_auth_name(&key))
}
fn conflicting_auth_name(key: &str) -> bool {
    key.starts_with("ANTHROPIC_")
        && [
            "KEY",
            "TOKEN",
            "BASE_URL",
            "AUTH",
            "CUSTOM_HEADERS",
            "FEDERATION",
            "ORGANIZATION",
            "UNIX_SOCKET",
            "PROFILE",
        ]
        .iter()
        .any(|part| key.contains(part))
        || key.starts_with("CLAUDE_CODE_USE_")
        || key.starts_with("CLAUDE_CODE_OAUTH")
        || key.starts_with("CLAUDE_CODE_HOST_")
        || key.starts_with("CLAUDE_CODE_PROVIDER_")
        || key.starts_with("CLAUDE_CODE_PROJECT_DIR")
        || key.starts_with("CLAUDE_CODE_API_KEY")
        || key.starts_with("CLAUDE_CODE_")
            && (key.contains("SETTINGS") || key.contains("TOKEN") || key.contains("GATEWAY"))
        || matches!(
            key,
            "CLAUDE_CODE_API_BASE_URL"
                | "CCR_SESSION_PROFILE"
                | "CLAUDE_CODE_REMOTE"
                | "CLAUDE_CONFIG_DIR_OVERRIDE"
                | "CLAUDE_CODE_SIMPLE"
                | "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"
                | "CCR_OAUTH_TOKEN_FILE"
                | "CCR_API_KEY_FILE"
        )
}

/// A settings document that could pick another credential source.
pub fn auth_settings_conflict(value: &serde_json::Value) -> bool {
    let Some(object) = value.as_object() else {
        return true;
    };
    [
        "apiKeyHelper",
        "forceLoginMethod",
        "forceLoginOrgUUID",
        "awsAuthRefresh",
        "awsCredentialExport",
        "gcpAuthRefresh",
    ]
    .iter()
    .any(|key| object.contains_key(*key))
        || object.get("env").is_some_and(|env| {
            let Some(env) = env.as_object() else {
                return true;
            };
            env.keys()
                .any(|key| conflicting_auth_name(&key.to_ascii_uppercase()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::production(root.path()).unwrap();
        (root, paths)
    }
    fn save(paths: &RuntimePaths, command: &str) {
        super::super::private_io::write_atomic(
            &paths.resource(Resource::HeadlessBilling),
            serde_json::to_string(&serde_json::json!({"version":1,"profiles":[{"id":"synthetic","display_name":"Synthetic resolver","provider":"claude","command":command}]}))
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
    }
    #[test]
    fn only_the_shipped_claude_definition_supports_api_key_billing() {
        assert!(api_key_supported("claude"));
        for provider in [
            "codex",
            "copilot",
            "cursor-agent",
            "opencode",
            "grok",
            "command-code",
            "fixture-registered",
            "",
        ] {
            assert!(!api_key_supported(provider), "{provider}");
        }
    }
    #[test]
    fn explicit_api_selection_requires_claude_headless_profile_and_positive_finite_budget() {
        assert_eq!(
            validate_selection("claude", "headless", "", "", None, "")
                .unwrap()
                .mode,
            PLAN_LOGIN
        );
        for budget in [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert!(
                validate_selection("claude", "headless", API_KEY, "synthetic", budget, "").is_err()
            );
        }
        for (provider, execution, mode, profile, route) in [
            ("codex", "headless", API_KEY, "synthetic", ""),
            ("copilot", "headless", API_KEY, "synthetic", ""),
            ("grok", "headless", API_KEY, "synthetic", ""),
            ("command-code", "headless", API_KEY, "synthetic", ""),
            ("claude", "interactive", API_KEY, "synthetic", ""),
            ("claude", "", API_KEY, "synthetic", ""),
            ("claude", "headless", "other", "synthetic", ""),
            ("claude", "headless", PLAN_LOGIN, "synthetic", ""),
            ("claude", "headless", API_KEY, "synthetic", "gateway"),
            ("claude", "headless", API_KEY, "bad id!", ""),
        ] {
            assert!(
                validate_selection(provider, execution, mode, profile, Some(1.0), route).is_err(),
                "{provider} {execution} {mode} {profile} {route}"
            );
        }
        // Plan login carries no budget either.
        assert!(validate_selection("claude", "headless", PLAN_LOGIN, "", Some(1.0), "").is_err());
    }
    #[test]
    fn recipes_are_private_revision_pinned_and_never_run() {
        let (_root, paths) = setup();
        save(&paths, "synthetic-resolver-never-executed");
        let selected = resolve(
            &paths,
            validate_selection("claude", "headless", API_KEY, "synthetic", Some(2.0), "").unwrap(),
        )
        .unwrap();
        assert_eq!(selected.display_name, "Synthetic resolver");
        assert_eq!(selected.revision.len(), 64);
        assert!(!format!("{selected:?}").contains("never-executed"));
        assert!(
            !serde_json::to_string(&selected)
                .unwrap()
                .contains("never-executed")
        );
        let (listed, revision) = disclosure(&paths).unwrap();
        assert_eq!(revision, selected.revision);
        assert!(
            !serde_json::to_string(&listed)
                .unwrap()
                .contains("never-executed")
        );
        let mut settings = serde_json::Map::new();
        recipe(&paths, &selected)
            .unwrap()
            .insert_settings(&mut settings);
        assert_eq!(
            settings["apiKeyHelper"],
            "synthetic-resolver-never-executed"
        );
        save(&paths, "different-synthetic-resolver");
        assert_eq!(
            verify_revision(&paths, &selected).unwrap_err(),
            BillingError::RevisionChanged
        );
        let mut unpinned = selected.clone();
        unpinned.revision.clear();
        assert_eq!(
            recipe(&paths, &unpinned).err(),
            Some(BillingError::RevisionChanged)
        );
    }
    #[test]
    fn a_missing_file_offers_no_profile_and_cannot_resolve_api_billing() {
        let (_root, paths) = setup();
        assert_eq!(disclosure(&paths).unwrap(), (vec![], String::new()));
        let selected =
            validate_selection("claude", "headless", API_KEY, "synthetic", Some(1.0), "").unwrap();
        assert_eq!(
            resolve(&paths, selected).unwrap_err(),
            BillingError::UnknownProfile
        );
    }
    #[test]
    fn unsupported_providers_and_unknown_fields_in_the_file_are_refused() {
        let (_root, paths) = setup();
        for document in [
            serde_json::json!({"version":1,"profiles":[{"id":"synthetic","display_name":"S","provider":"codex","command":"synthetic"}]}),
            serde_json::json!({"version":1,"profiles":[{"id":"synthetic","display_name":"S","provider":"claude","command":"synthetic","key":"synthetic"}]}),
            serde_json::json!({"version":2,"profiles":[]}),
            serde_json::json!({"version":1,"profiles":[{"id":"a","display_name":"S","provider":"claude","command":"x"},{"id":"a","display_name":"T","provider":"claude","command":"y"}]}),
            serde_json::json!({"version":1,"profiles":[{"id":"a","display_name":"S","provider":"claude","command":"line\nbreak"}]}),
        ] {
            super::super::private_io::write_atomic(
                &paths.resource(Resource::HeadlessBilling),
                &serde_json::to_vec(&document).unwrap(),
            )
            .unwrap();
            assert_eq!(profiles(&paths).unwrap_err(), BillingError::InvalidProfile);
        }
    }
    #[cfg(unix)]
    #[test]
    fn private_file_permissions_and_symlinks_fail_closed() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (_root, paths) = setup();
        save(&paths, "synthetic-only");
        let path = paths.resource(Resource::HeadlessBilling);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(profiles(&paths).unwrap_err(), BillingError::PrivateConfig);
        let target = path.with_extension("target");
        std::fs::rename(&path, &target).unwrap();
        symlink(target, &path).unwrap();
        assert_eq!(profiles(&paths).unwrap_err(), BillingError::PrivateConfig);
    }
    #[cfg(windows)]
    #[test]
    fn a_file_with_an_inherited_acl_is_not_private() {
        let (_root, paths) = setup();
        let path = paths.resource(Resource::HeadlessBilling);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A plain write keeps the inherited (unprotected) ACL of the folder.
        std::fs::write(
            &path,
            br#"{"version":1,"profiles":[{"id":"synthetic","display_name":"S","provider":"claude","command":"synthetic"}]}"#,
        )
        .unwrap();
        assert_eq!(profiles(&paths).unwrap_err(), BillingError::PrivateConfig);
        save(&paths, "synthetic-only");
        assert_eq!(profiles(&paths).unwrap().len(), 1);
    }
    #[test]
    fn auth_checks_are_narrow_and_do_not_echo_values() {
        assert!(auth_environment_conflicts(&[
            "ANTHROPIC_API_KEY=synthetic-secret".into()
        ]));
        assert!(auth_environment_conflicts(&["anthropic_base_url=".into()]));
        assert!(auth_environment_conflicts(&[
            "CLAUDE_CODE_USE_BEDROCK=1".into()
        ]));
        assert!(!auth_environment_conflicts(&[
            "CLAUDE_CONFIG_DIR=/synthetic".into(),
            "PATH=/synthetic".into()
        ]));
        assert!(auth_settings_conflict(
            &serde_json::json!({"apiKeyHelper":"synthetic-private"})
        ));
        assert!(auth_settings_conflict(
            &serde_json::json!({"env":{"ANTHROPIC_AUTH_TOKEN":"synthetic"}})
        ));
        assert!(!auth_settings_conflict(
            &serde_json::json!({"model":"sonnet"})
        ));
        assert_eq!(
            BillingError::AuthConflict.to_string(),
            "headless billing authentication configuration conflicts with API key billing"
        );
    }
    #[test]
    fn recognizable_embedded_key_literals_are_rejected_without_echoing() {
        let (_root, paths) = setup();
        let sentinel = format!("sk-ant-{}", "syntheticnotarealsecret".repeat(3));
        for field in ["id", "display_name", "command"] {
            let mut profile = serde_json::json!({"id":"synthetic","display_name":"Synthetic resolver","provider":"claude","command":"synthetic-resolver"});
            profile[field] = sentinel.clone().into();
            super::super::private_io::write_atomic(
                &paths.resource(Resource::HeadlessBilling),
                serde_json::to_vec(&serde_json::json!({"version":1,"profiles":[profile]}))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
            let error = profiles(&paths).unwrap_err();
            assert_eq!(error, BillingError::InvalidProfile);
            assert!(!error.to_string().contains(&sentinel));
        }
    }
}
