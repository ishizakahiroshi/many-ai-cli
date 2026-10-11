//! Per-run billing. Recipes are private, non-serializable values and are never
//! executed here. Only Claude may consume a recipe through apiKeyHelper.
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BillingProfileSummary {
    pub id: String,
    pub display_name: String,
    pub provider: String,
}
/// Safe selection only. This must never acquire a command/key/settings field.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedBilling {
    pub mode: String,
    pub profile_id: String,
    pub display_name: String,
    pub provider: String,
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
/// Intentionally neither Debug nor Serialize: callers can only merge this into
/// a session-owned private Claude settings document.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BillingError {
    InvalidSelection,
    InvalidProfile,
    UnknownProfile,
    PrivateConfig,
    RevisionChanged,
    AuthConflict,
    AuthUnverified,
    UnsupportedVersion,
}
impl fmt::Display for BillingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidSelection => "invalid headless billing selection",
            Self::InvalidProfile => "invalid headless billing profile configuration",
            Self::UnknownProfile => "headless billing profile is unavailable",
            Self::PrivateConfig => "headless billing private configuration is unavailable",
            Self::RevisionChanged => {
                "headless billing profile changed; confirm the selection again"
            }
            Self::AuthConflict => {
                "headless billing authentication configuration conflicts with the selected mode"
            }
            Self::AuthUnverified => "headless plan-login authentication could not be verified",
            Self::UnsupportedVersion => "headless billing Claude version is not verified",
        })
    }
}
impl std::error::Error for BillingError {}
impl From<BillingError> for io::Error {
    fn from(error: BillingError) -> Self {
        io::Error::new(io::ErrorKind::InvalidInput, error)
    }
}
pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
pub fn validate_selection(
    provider: &str,
    execution_mode: &str,
    mode: &str,
    profile: &str,
    budget: Option<f64>,
    route: &str,
) -> Result<ResolvedBilling, BillingError> {
    let mode = if mode.is_empty() { PLAN_LOGIN } else { mode };
    match mode {
        PLAN_LOGIN if profile.is_empty() && budget.is_none() => Ok(ResolvedBilling {
            provider: provider.into(),
            ..Default::default()
        }),
        API_KEY
            if provider == "claude"
                && execution_mode == "headless"
                && valid_id(profile)
                && budget.is_some_and(|v| v.is_finite() && v > 0.0)
                && matches!(route, "" | "default") =>
        {
            Ok(ResolvedBilling {
                mode: mode.into(),
                profile_id: profile.into(),
                provider: provider.into(),
                max_budget_usd: budget,
                ..Default::default()
            })
        }
        _ => Err(BillingError::InvalidSelection),
    }
}
fn document(paths: &RuntimePaths) -> Result<Option<(Document, String)>, BillingError> {
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Read-only check: never silently repair caller-owned permissions.
        if metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err(BillingError::PrivateConfig);
        }
    }
    #[cfg(windows)]
    super::private_io::PrivateSecurity::for_current_user(false)
        .and_then(|security| security.verify_file(&file))
        .map_err(|_| BillingError::PrivateConfig)?;
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BillingError::PrivateConfig)?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(BillingError::PrivateConfig);
    }
    let doc: Document = serde_json::from_slice(&bytes).map_err(|_| BillingError::InvalidProfile)?;
    if doc.version != 1 || doc.profiles.len() > 64 {
        return Err(BillingError::InvalidProfile);
    }
    let mut ids = BTreeSet::new();
    for profile in &doc.profiles {
        if [&profile.id, &profile.display_name, &profile.command]
            .into_iter()
            .any(|value| crate::storage::mask_secrets(value) != *value)
            || !valid_id(&profile.id)
            || !ids.insert(&profile.id)
            || profile.provider != "claude"
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
pub fn profiles(paths: &RuntimePaths) -> Result<Vec<BillingProfileSummary>, BillingError> {
    Ok(document(paths)?
        .map(|(doc, _)| {
            doc.profiles
                .into_iter()
                .map(|profile| BillingProfileSummary {
                    id: profile.id,
                    display_name: profile.display_name,
                    provider: profile.provider,
                })
                .collect()
        })
        .unwrap_or_default())
}
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
pub fn recipe(
    paths: &RuntimePaths,
    selected: &ResolvedBilling,
) -> Result<PrivateRecipe, BillingError> {
    let (doc, revision) = document(paths)?.ok_or(BillingError::UnknownProfile)?;
    if !selected.is_api() || selected.revision.is_empty() || revision != selected.revision {
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
/// Check names only; never format values into errors. A configured competing
/// source is refused, including empty/false values whose CLI semantics vary.
pub fn auth_environment_conflicts(environment: &[String]) -> bool {
    environment
        .iter()
        .filter_map(|e| e.split_once('=').map(|(k, _)| k.to_ascii_uppercase()))
        .any(|key| {
            key.starts_with("ANTHROPIC_")
                && (key.contains("KEY")
                    || key.contains("TOKEN")
                    || key.contains("BASE_URL")
                    || key.contains("AUTH")
                    || key.contains("CUSTOM_HEADERS")
                    || key.contains("FEDERATION")
                    || key.contains("ORGANIZATION")
                    || key.contains("UNIX_SOCKET")
                    || key.contains("PROFILE"))
                || key.starts_with("CLAUDE_CODE_USE_")
                || key.starts_with("CLAUDE_CODE_OAUTH")
                || key.starts_with("CLAUDE_CODE_HOST_")
                || key.starts_with("CLAUDE_CODE_PROVIDER_")
                || key == "CLAUDE_CODE_API_BASE_URL"
                || key == "CCR_SESSION_PROFILE"
                || key == "CLAUDE_CODE_REMOTE"
                || key.starts_with("CLAUDE_CODE_PROJECT_DIR")
                || key.starts_with("CLAUDE_CODE_") && key.contains("SETTINGS")
                || key.starts_with("CLAUDE_CODE_API_KEY")
                || key.contains("GATEWAY") && key.starts_with("CLAUDE_CODE_")
                || key.starts_with("CLAUDE_CODE_") && key.contains("TOKEN")
                || matches!(
                    key.as_str(),
                    "CLAUDE_CONFIG_DIR_OVERRIDE"
                        | "CLAUDE_CODE_SETTINGS_PATH"
                        | "CLAUDE_CODE_MANAGED_SETTINGS"
                        | "CLAUDE_CODE_SIMPLE"
                        | "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"
                        | "CCR_OAUTH_TOKEN_FILE"
                        | "CCR_API_KEY_FILE"
                )
        })
}
pub fn auth_settings_conflict(value: &serde_json::Value) -> bool {
    let Some(object) = value.as_object() else {
        return true;
    };
    object.contains_key("apiKeyHelper")
        || object.contains_key("forceLoginMethod")
        || object.contains_key("forceLoginOrgUUID")
        || object.contains_key("awsAuthRefresh")
        || object.contains_key("awsCredentialExport")
        || object.contains_key("gcpAuthRefresh")
        || object.get("env").is_some_and(|env| {
            let Some(env) = env.as_object() else {
                return true;
            };
            auth_environment_conflicts(&env.keys().map(|key| format!("{key}=")).collect::<Vec<_>>())
        })
}
pub fn verified_plan_auth(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    value.get("forcedLoginMethod").is_none()
        && value.get("allowedProviders").is_none()
        && value.get("loggedIn").and_then(|v| v.as_bool()) == Some(true)
        && value.get("authMethod").and_then(|v| v.as_str()) == Some("claude.ai")
        && value
            .get("subscriptionType")
            .and_then(|v| v.as_str())
            .is_some_and(|v| matches!(v, "pro" | "max"))
        && value
            .get("apiProvider")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v == "firstParty")
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
        super::super::private_io::write_atomic(&paths.resource(Resource::HeadlessBilling), serde_json::to_string(&serde_json::json!({"version":1,"profiles":[{"id":"synthetic","display_name":"Synthetic resolver","provider":"claude","command":command}]})).unwrap().as_bytes()).unwrap();
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
            ("claude", "interactive", API_KEY, "synthetic", ""),
            ("claude", "headless", "other", "synthetic", ""),
            ("claude", "headless", PLAN_LOGIN, "synthetic", ""),
            ("claude", "headless", API_KEY, "synthetic", "gateway"),
        ] {
            assert!(
                validate_selection(provider, execution, mode, profile, Some(1.0), route).is_err()
            );
        }
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
        assert!(!format!("{selected:?}").contains("never-executed"));
        assert!(
            !serde_json::to_string(&profiles(&paths).unwrap())
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
    #[test]
    fn auth_checks_are_narrow_and_do_not_echo_values() {
        assert!(auth_environment_conflicts(&[
            "ANTHROPIC_API_KEY=synthetic-secret".into()
        ]));
        assert!(auth_settings_conflict(
            &serde_json::json!({"apiKeyHelper":"synthetic-private"})
        ));
        assert!(verified_plan_auth(br#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max","apiProvider":"firstParty"}"#));
        assert!(!verified_plan_auth(
            br#"{"loggedIn":true,"authMethod":"api_key","subscriptionType":"max"}"#
        ));
        assert!(!verified_plan_auth(
            br#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"team"}"#
        ));
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
