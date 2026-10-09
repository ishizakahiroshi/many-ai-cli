//! Application launch decisions from the fixed Go spawn/subscription sources.
//! No process-global environment access, provider requests, usage probes, or
//! session map. Callers supply the application's actual registry/cache snapshots.
mod environment;
mod routes;
#[cfg(test)]
mod tests;
mod trust;

use super::wrapped_spawn::{ResolvedSpawnPolicy, SpawnLaunchPolicy};
use crate::{
    config::{self, ConfigError, ConfigStore, RuntimePaths},
    profile::{
        registry::Registry,
        subscriptions::{SeedNotice, SubscriptionLauncher},
    },
    proto::core::WrappedSpawnSpec,
};
pub use environment::{
    NativePathEnvironment, PathEnvironment, WindowsEnvironmentSources, WindowsPathEnvironment,
};
use std::{collections::BTreeSet, io, path::PathBuf, sync::Arc};
pub use trust::{FolderTrustResult, NativeFolderTrust};

/// Stale model IDs are intentional: route inference never waits for a refresh.
#[derive(Clone, Default)]
pub struct LocalModelSnapshot {
    pub ollama: BTreeSet<String>,
    pub lm_studio: BTreeSet<String>,
}
pub type RegistrySnapshot = Arc<dyn Fn() -> io::Result<Registry> + Send + Sync>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PolicyFailureStage {
    Subscription,
    Route,
}
#[derive(Debug)]
struct PolicyFailure {
    stage: PolicyFailureStage,
    source: io::Error,
}
impl std::fmt::Display for PolicyFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}
impl std::error::Error for PolicyFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}
fn policy_failure(stage: PolicyFailureStage, source: io::Error) -> io::Error {
    io::Error::new(source.kind(), PolicyFailure { stage, source })
}
pub(crate) fn failure_stage(error: &io::Error) -> Option<PolicyFailureStage> {
    error
        .get_ref()?
        .downcast_ref::<PolicyFailure>()
        .map(|error| error.stage)
}
pub type ModelSnapshot = Arc<dyn Fn() -> io::Result<LocalModelSnapshot> + Send + Sync>;
pub type SeedObserver = Arc<dyn Fn(&SeedNotice) + Send + Sync>;
pub type TrustObserver =
    Arc<dyn Fn(&WrappedSpawnSpec, &io::Result<FolderTrustResult>) + Send + Sync>;

/// Dependencies are explicit so trials cannot discover the operator's home or
/// silently substitute an empty provider registry/cache. Observers receive no
/// configuration values or credential material.
#[derive(Clone)]
pub struct SpawnPolicyDependencies {
    pub registry: RegistrySnapshot,
    pub local_models: ModelSnapshot,
    pub path_environment: Arc<dyn PathEnvironment>,
    pub vendor_home: PathBuf,
    /// Hub actor cwd for Go-compatible relative vendor-settings I/O. It is not
    /// the child cwd and never replaces the inherited child environment text.
    pub hub_cwd: PathBuf,
    pub seeded: SeedObserver,
    pub trusted: TrustObserver,
}
pub struct ConfigSpawnLaunchPolicy {
    paths: RuntimePaths,
    config: Arc<ConfigStore>,
    dependencies: SpawnPolicyDependencies,
    subscriptions: SubscriptionLauncher,
    trust: NativeFolderTrust,
}
impl ConfigSpawnLaunchPolicy {
    /// Source resolveRoute: reads current configured IDs and retained cache only.
    pub fn detected_model_route(&self, provider: &str, model: &str) -> io::Result<String> {
        let cfg = self.config.snapshot().map_err(config_error)?.config;
        let mut models = (self.dependencies.local_models)()?;
        models.ollama.extend(
            cfg.local_models
                .iter()
                .map(|m| m.id.trim())
                .filter(|m| !m.is_empty())
                .map(str::to_owned),
        );
        Ok(routes::infer(provider, model, &models).into())
    }
    pub fn new(
        paths: RuntimePaths,
        config: Arc<ConfigStore>,
        dependencies: SpawnPolicyDependencies,
    ) -> io::Result<Self> {
        if !dependencies.vendor_home.is_absolute() {
            return Err(invalid("vendor home must be explicitly absolute"));
        }
        crate::profile::subscriptions::check_path(&paths, &dependencies.vendor_home)?;
        if !dependencies.hub_cwd.is_absolute() {
            return Err(invalid("Hub cwd must be explicitly absolute"));
        }
        crate::profile::subscriptions::check_path(&paths, &dependencies.hub_cwd)?;
        Ok(Self {
            subscriptions: SubscriptionLauncher::new(
                paths.clone(),
                dependencies.vendor_home.clone(),
                dependencies.hub_cwd.clone(),
            ),
            trust: NativeFolderTrust::new(
                paths.clone(),
                dependencies.vendor_home.clone(),
                dependencies.hub_cwd.clone(),
            ),
            paths,
            config,
            dependencies,
        })
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn valid_model_label(value: &str) -> bool {
    !value.starts_with('-')
        && !value.chars().any(|c| {
            c < ' '
                || c == '\u{7f}'
                || matches!(
                    c,
                    ' ' | '"'
                        | '\''
                        | '|'
                        | '&'
                        | '>'
                        | '<'
                        | '^'
                        | '%'
                        | '('
                        | ')'
                        | ';'
                        | '`'
                        | '$'
                )
        })
}
fn wrapper_effort(provider: &str, effort: &str) -> Vec<String> {
    if config::effort_support_for(provider).is_some() {
        vec!["--effort".into(), effort.into()]
    } else {
        vec![]
    }
}
fn config_error(_: ConfigError) -> io::Error {
    io::Error::other("spawn configuration operation failed")
}
impl SpawnLaunchPolicy for ConfigSpawnLaunchPolicy {
    fn resolve(
        &self,
        spec: &WrappedSpawnSpec,
        base_environment: &[String],
    ) -> io::Result<ResolvedSpawnPolicy> {
        let cfg = self.config.snapshot().map_err(config_error)?.config;
        let registry = (self.dependencies.registry)()?;
        let definition = registry.lookup(&spec.provider);
        if spec.provider != "shell"
            && !definition.as_ref().is_some_and(|d| {
                d.definition.enabled.unwrap_or(true) && d.definition.launch.is_some()
            })
        {
            return Err(invalid(
                "provider is absent, disabled, or has no launch definition",
            ));
        }
        routes::validate(&spec.provider, &spec.route)
            .map_err(|error| policy_failure(PolicyFailureStage::Route, error))?;
        let custom = cfg.is_custom_provider_id(&spec.provider)
            || (!crate::proto::provider::BUILTIN_PROVIDER_IDS.contains(&spec.provider.as_str())
                && spec.provider != "shell");
        if (custom || spec.provider == "shell") && !spec.subscription_profile_id.trim().is_empty() {
            return Err(invalid("provider does not support subscription profiles"));
        }
        // Source order: profile choice/seed occurs once, before login's early
        // return and before route environment construction. Never repeat this in
        // folder_trust or substitute another account after an error.
        let (subscription_environment, notice) = self
            .subscriptions
            .launch(
                &cfg,
                &spec.provider,
                &spec.subscription_profile_id,
                base_environment,
            )
            .map_err(|error| policy_failure(PolicyFailureStage::Subscription, error))?;
        if let Some(notice) = notice {
            (self.dependencies.seeded)(&notice);
        }
        if !valid_model_label(&spec.model) || !valid_model_label(&spec.label) {
            return Err(invalid("invalid model or label value"));
        }
        // The source rereads live preferences/local-model configuration after
        // profile seeding, which can perform filesystem I/O. Retain the chosen
        // profile but do not freeze unrelated route/model settings across it.
        let cfg = self.config.snapshot().map_err(config_error)?.config;
        let current_model = cfg
            .user_prefs
            .spawn
            .last_model
            .get(&spec.provider)
            .map(|m| m.trim().to_owned())
            .unwrap_or_default();
        let resolved_model = if spec.subscription_login
            || spec.provider == "shell"
            || (custom
                && !definition.as_ref().is_some_and(|d| {
                    d.definition
                        .launch
                        .as_ref()
                        .is_some_and(|l| !l.model_args.is_empty())
                })) {
            String::new()
        } else {
            spec.model.trim().to_owned()
        };
        // Match provider_capability.go exactly: a declared registry mapping owns
        // the result, including the empty result when its level is invalid.
        // These are raw wrapper argv, not a second translation into --effort.
        let effort_args = if spec.subscription_login || spec.effort.is_empty() {
            vec![]
        } else if let Some(definition) = &definition {
            match crate::profile::registry::effort_args(definition, &spec.effort) {
                Ok(args) if !args.is_empty() => args,
                _ if definition
                    .definition
                    .launch
                    .as_ref()
                    .is_some_and(|l| !l.effort_args.is_empty()) =>
                {
                    vec![]
                }
                _ => wrapper_effort(&spec.provider, &spec.effort),
            }
        } else {
            wrapper_effort(&spec.provider, &spec.effort)
        };
        let effective_route = if spec.subscription_login || spec.provider == "shell" {
            String::new()
        } else if !spec.route.is_empty() {
            spec.route.clone()
        } else if custom {
            String::new()
        } else {
            let mut models = (self.dependencies.local_models)()?;
            models.ollama.extend(
                cfg.local_models
                    .iter()
                    .map(|m| m.id.trim())
                    .filter(|m| !m.is_empty())
                    .map(str::to_owned),
            );
            routes::infer(&spec.provider, &resolved_model, &models).into()
        };
        routes::validate(&spec.provider, &effective_route)
            .map_err(|error| policy_failure(PolicyFailureStage::Route, error))?;
        let base_environment = self
            .dependencies
            .path_environment
            .sanitize(base_environment)?;
        let route_cfg = self.config.snapshot().map_err(config_error)?.config;
        let route_environment = routes::environment(
            &self.paths,
            &route_cfg,
            &spec.provider,
            &effective_route,
            &spec.model,
            &base_environment,
        )
        .map_err(|error| policy_failure(PolicyFailureStage::Route, error))?;
        // Only ordinary HTTP Start appends custom registry model templates.
        // The historical registration-wait path retains its --model argument.
        let ordinary_model_args = if custom && !resolved_model.is_empty() {
            definition
                .as_ref()
                .and_then(|d| crate::profile::registry::model_args(d, &resolved_model).ok())
                .unwrap_or_default()
        } else {
            vec![]
        };
        Ok(ResolvedSpawnPolicy {
            base_environment,
            route_environment,
            subscription_environment,
            effective_route,
            current_model,
            resolved_model,
            effort_args,
            ordinary_model_args,
        })
    }
    fn folder_trust(
        &self,
        spec: &WrappedSpawnSpec,
        exact_environment: &[String],
    ) -> io::Result<()> {
        // Unsupported providers have no checkbox in Go. Defense in depth for
        // a core grant mistakenly passed for another provider: no guessed file.
        let result = self
            .trust
            .grant(&spec.provider, exact_environment, &spec.cwd);
        (self.dependencies.trusted)(spec, &result);
        result.map(|_| ())
    }
    fn registered_model(&self, provider: &str, model: &str) -> io::Result<()> {
        loop {
            let snapshot = self.config.snapshot().map_err(config_error)?;
            let mut next = snapshot.config;
            next.user_prefs
                .spawn
                .last_model
                .insert(provider.into(), model.into());
            match self
                .config
                .publish_then_persist_legacy(snapshot.revision, next)
            {
                Ok(_) => return Ok(()),
                Err(ConfigError::Conflict { .. }) => continue,
                Err(error) => return Err(config_error(error)),
            }
        }
    }
}
