use crate::{
    process::{Cancellation, ProcessPlan},
    proto::{
        core::{ProviderCommandPlan, ProviderCommandPurpose},
        provider::{Definition, LaunchDefinition, UpdateDefinition},
    },
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    NotInstalled,
    UpdateDisabled,
    NotConfigured,
    LoginMayBeRequired,
    UpdateExecutableMissing(String),
}
impl Unavailable {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInstalled | Self::UpdateExecutableMissing(_) => "not_installed",
            Self::UpdateDisabled => "update_disabled",
            Self::NotConfigured => "update_not_configured",
            Self::LoginMayBeRequired => "login_may_be_required",
        }
    }
}
#[derive(Clone)]
pub struct UpdatePlan {
    pub command: ProviderCommandPlan,
    /// Command identifier selected from the manifest. The resolved path used in
    /// execution is intentionally shown by preview/log rather than launch A.
    pub requested_executable: String,
    pub selected_launch_executable: String,
    pub launch_path: PathBuf,
    pub version_args: Vec<String>,
}
impl UpdatePlan {
    pub fn argv(&self) -> Vec<String> {
        std::iter::once(
            self.command
                .process
                .executable
                .to_string_lossy()
                .into_owned(),
        )
        .chain(
            self.command
                .process
                .args
                .iter()
                .map(|v| v.to_string_lossy().into_owned()),
        )
        .collect()
    }
    pub fn log_header(&self) -> String {
        format!(
            "provider: {}\nexecutable: {}\nargv: {}\n",
            self.command.provider,
            self.command.process.executable.display(),
            serde_json::to_string(&self.argv()).expect("string argv")
        )
    }
}
pub fn update_enabled(update: Option<&UpdateDefinition>) -> bool {
    update.is_some_and(|u| u.enabled.unwrap_or(!u.args.is_empty()))
}
pub fn version_args(update: Option<&UpdateDefinition>) -> Vec<String> {
    update
        .filter(|u| !u.version_args.is_empty())
        .map(|u| u.version_args.clone())
        .unwrap_or_else(|| vec!["--version".into()])
}
pub fn timeout_seconds(update: Option<&UpdateDefinition>) -> u64 {
    update
        .map(|u| u.timeout_seconds)
        .filter(|v| *v > 0)
        .unwrap_or(300) as u64
}
pub fn primary_executable(launch: &LaunchDefinition) -> &str {
    if launch.executable.is_empty() {
        launch
            .executable_candidates
            .first()
            .map(String::as_str)
            .unwrap_or("")
    } else {
        &launch.executable
    }
}
pub fn resolve_argv(definition: &Definition, selected: &str) -> Result<Vec<String>, Unavailable> {
    let update = definition
        .update
        .as_ref()
        .filter(|u| !u.args.is_empty())
        .ok_or(Unavailable::NotConfigured)?;
    let executable = if update.executable.is_empty() {
        let launch = definition
            .launch
            .as_ref()
            .ok_or(Unavailable::NotConfigured)?;
        if primary_executable(launch).is_empty() || primary_executable(launch) != selected {
            return Err(Unavailable::NotConfigured);
        }
        selected
    } else {
        &update.executable
    };
    Ok(std::iter::once(executable.to_string())
        .chain(update.args.clone())
        .collect())
}

/// `resolve` is the platform executable lookup owned by the caller. It is called
/// for B even when A was found. An absent B never falls back to A's resolved path.
pub fn plan_update(
    definition: &Definition,
    cwd: &Path,
    mut resolve: impl FnMut(&str) -> Option<PathBuf>,
) -> Result<UpdatePlan, Unavailable> {
    let launch = definition
        .launch
        .as_ref()
        .ok_or(Unavailable::NotInstalled)?;
    let candidates: Vec<&str> = std::iter::once(launch.executable.as_str())
        .chain(launch.executable_candidates.iter().map(String::as_str))
        .filter(|v| !v.is_empty())
        .collect();
    let (selected, path) = candidates
        .into_iter()
        .find_map(|name| resolve(name).map(|p| (name.to_string(), p)))
        .ok_or(Unavailable::NotInstalled)?;
    let update = definition.update.as_ref();
    if !update_enabled(update) {
        return Err(if update.is_some_and(|u| u.login_may_be_required) {
            Unavailable::LoginMayBeRequired
        } else {
            Unavailable::UpdateDisabled
        });
    }
    let argv = resolve_argv(definition, &selected)?;
    let resolved = if argv[0] == selected {
        path.clone()
    } else {
        resolve(&argv[0]).ok_or_else(|| Unavailable::UpdateExecutableMissing(argv[0].clone()))?
    };
    if !resolved.is_absolute() {
        return Err(Unavailable::UpdateExecutableMissing(argv[0].clone()));
    }
    Ok(UpdatePlan {
        command: ProviderCommandPlan {
            provider: definition.id.clone(),
            purpose: ProviderCommandPurpose::Update,
            cancellation: Cancellation::default(),
            process: ProcessPlan {
                executable: resolved,
                args: argv[1..].iter().map(Into::into).collect(),
                cwd: cwd.into(),
                env: BTreeMap::new(),
                stdin: vec![],
                timeout: Duration::from_secs(timeout_seconds(update)),
                output_cap: 1024 * 1024,
                pipe_drain_timeout: Duration::from_millis(500),
            },
        },
        requested_executable: argv[0].clone(),
        selected_launch_executable: selected,
        launch_path: path,
        version_args: version_args(update),
    })
}
