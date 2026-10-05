//! On-demand CLI version checks from the frozen Go caller. The existing Hub
//! request owner must retain the initiating future after an HTTP disconnect.
//! No process, timer, provider registry, or updater is owned by this module.
pub mod native;
#[cfg(test)]
mod tests;

use super::http::{Request, Response, decode_json, require_method};
use crate::{
    application::spawn_policy::PathEnvironment,
    process::execpath::{ExecutableFs, Platform, Resolver},
    profile::registry::Registry,
    proto::{
        core::CoreFuture,
        provider::Definition,
        time::{Timestamp, format_go_rfc3339_layout},
        wire::{Field, GoWire, Schema, go_utf8_lossy},
    },
};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;

pub const PATH: &str = "/api/cli-versions";
pub const OUTPUT_CAP: usize = 8 * 1024;
pub const MAX_CONCURRENCY: usize = 8;
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliVersionResult {
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub executable: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version_line: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version_text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub executable_modified_at: String,
    pub exit_code: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliVersionResponse {
    pub checked_at: String,
    pub results: Vec<CliVersionResult>,
}

/// No Debug/serialization: arguments and environment may contain private data.
/// The executor must use an owned managed process, no stdin, a combined ordered
/// stdout/stderr capture, and kill/reap its tree on timeout or future drop.
/// This is deliberately not a ProcessOutput adapter: concatenating independent
/// stdout/stderr buffers changes both Go's first-line and shared-cap behavior.
pub struct CliVersionCommand {
    pub executable: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub environment: Vec<String>,
    pub platform: Platform,
    pub timeout: Duration,
    pub output_cap: usize,
}
#[derive(Clone, Default)]
pub struct CliVersionProcessOutput {
    /// Raw combined bytes, in observed write order. Extra bytes are discarded.
    pub output: Vec<u8>,
    pub exit_code: i32,
    pub timed_out: bool,
    /// Go cmd.Start failure is not arbitrary diagnostic text on the wire.
    pub start_failed: bool,
}
pub trait CliVersionExecutor: Send + Sync {
    fn execute(
        &self,
        command: CliVersionCommand,
    ) -> CoreFuture<'_, Result<CliVersionProcessOutput, CliVersionFailure>>;
}
pub type VersionMtime = dyn Fn(&str) -> Option<Timestamp> + Send + Sync;
pub struct CliVersionDependencies {
    pub executor: Option<Arc<dyn CliVersionExecutor>>,
    pub cwd: PathBuf,
    pub environment: Vec<String>,
    pub platform: Platform,
    /// Reuses the same explicit spawn-time PATH expansion source as launch.
    pub path_environment: Arc<dyn PathEnvironment>,
    pub executable_fs: Arc<dyn ExecutableFs>,
    /// Stat the resolved executable (including Windows shim unwrapping), with
    /// the supplied cwd for relative paths. Missing metadata is not an error.
    pub modified_at: Arc<VersionMtime>,
    /// Called after all providers finish, never on a cache GET or joined run.
    pub clock: Arc<dyn Fn() -> Timestamp + Send + Sync>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Body {
    providers: Option<Vec<String>>,
}
impl GoWire for Body {
    const GO_TYPE: &'static str = "CliVersionRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: Self::GO_TYPE,
        fields: &[Field {
            name: "providers",
            kind: "[]string",
        }],
    }];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliVersionFailure {
    RunnerUnavailable,
    EnvironmentUnavailable,
    ClockUnavailable,
    OwnerDropped,
    TrialDisabled,
    TrialBoundary,
    InvalidPlatform,
    InvalidEnvironment,
    InvalidContext,
    ProcessUnavailable,
}
impl CliVersionFailure {
    fn response(self) -> Response {
        let (status, code, detail) = match self {
            Self::RunnerUnavailable => (
                503,
                "cli_version_runner_unavailable",
                "CLI version runner is not configured",
            ),
            Self::EnvironmentUnavailable => {
                (500, "internal", "CLI version environment unavailable")
            }
            Self::ClockUnavailable => (500, "internal", "CLI version clock unavailable"),
            Self::OwnerDropped => (503, "cli_version_cancelled", "CLI version owner stopped"),
            Self::TrialDisabled => (
                503,
                "cli_version_disabled",
                "CLI version checks are disabled in trial mode",
            ),
            Self::TrialBoundary => (
                403,
                "cli_version_trial_boundary",
                "CLI version command is outside the selected trial scope",
            ),
            Self::InvalidPlatform => (
                503,
                "cli_version_platform_unavailable",
                "CLI version platform is unavailable",
            ),
            Self::InvalidEnvironment => (500, "internal", "CLI version environment is invalid"),
            Self::InvalidContext => (500, "internal", "CLI version execution context is invalid"),
            Self::ProcessUnavailable => (
                502,
                "cli_version_process_unavailable",
                "CLI version process or capture could not complete",
            ),
        };
        Response::error(status, code, detail)
    }
}
type RunResult = Result<CliVersionResponse, CliVersionFailure>;
type Flight = watch::Sender<Option<RunResult>>;
#[derive(Default)]
struct State {
    last: Option<CliVersionResponse>,
    inflight: Option<Arc<Flight>>,
}

/// One memory-only cache per Hub, never per provider/body/client. All concurrent
/// callers join the first operation, even if their requested IDs differ.
/// Parent owns the shared instance and its HubTaskOwner integration.
pub struct CliVersionHttp {
    state: Mutex<State>,
    dependencies: CliVersionDependencies,
}
impl CliVersionHttp {
    pub fn new(dependencies: CliVersionDependencies) -> Self {
        Self {
            state: Mutex::new(State::default()),
            dependencies,
        }
    }
    pub fn last_result(&self) -> Option<CliVersionResponse> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last
            .clone()
    }
    pub fn refresh_update_result(&self, result: CliVersionResult) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let last = state.last.get_or_insert_with(|| CliVersionResponse {
            checked_at: crate::proto::time::format_go_rfc3339_layout(
                (self.dependencies.clock)(),
                0,
                false,
            )
            .unwrap_or_default(),
            results: Vec::new(),
        });
        if let Some(entry) = last
            .results
            .iter_mut()
            .find(|entry| entry.provider == result.provider)
        {
            *entry = result;
        } else {
            last.results.push(result);
        }
    }
    #[cfg(test)]
    pub(crate) fn inflight_waiter_count_for_test(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .inflight
            .as_ref()
            .map(|flight| flight.receiver_count())
            .unwrap_or(0)
    }
    /// Call only after the full token/Host/Origin/PIN guard. The existing request
    /// owner must keep this future alive after the response waiter disconnects:
    /// the Go POST intentionally uses Background, not the request context.
    /// This function spawns no unmanaged tasks and adds no update admission.
    pub async fn handle_authenticated(
        &self,
        request: &Request,
        registry: Option<&Registry>,
    ) -> Response {
        if request.method != "POST" {
            if let Err(response) = require_method(request, &["GET"]) {
                return response;
            }
            return Response::json(200, &self.last_result().unwrap_or_default()).no_store();
        }
        // A genuinely empty body is optional in this specific Go endpoint.
        // Shared Request has no ContentLength; transport supplies actual bytes.
        let body: Body = if !should_decode_body(request) {
            Body::default()
        } else {
            match decode_json(request) {
                Ok(body) => body,
                Err(response) => return response,
            }
        };
        let Some(registry) = registry else {
            return Response::error(
                503,
                "provider_registry_unavailable",
                "provider registry is unavailable",
            );
        };
        match self
            .run(
                registry,
                normalize_provider_ids(body.providers.as_deref().unwrap_or_default()),
            )
            .await
        {
            Ok(result) => Response::json(200, &result).no_store(),
            Err(error) => error.response(),
        }
    }
    pub async fn run(&self, registry: &Registry, ids: Vec<String>) -> RunResult {
        let (flight, leader) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(flight) = &state.inflight {
                (flight.clone(), false)
            } else {
                let (sender, _) = watch::channel(None);
                let flight = Arc::new(sender);
                state.inflight = Some(flight.clone());
                (flight, true)
            }
        };
        if !leader {
            let mut receiver = flight.subscribe();
            loop {
                if let Some(result) = receiver.borrow_and_update().clone() {
                    return result;
                }
                if receiver.changed().await.is_err() {
                    return Err(CliVersionFailure::OwnerDropped);
                }
            }
        }
        let mut completion = Completion {
            owner: self,
            flight,
            published: false,
        };
        let result = self.check_versions(registry, ids).await;
        completion.publish(result.clone());
        result
    }
    async fn check_versions(&self, registry: &Registry, ids: Vec<String>) -> RunResult {
        let ids = if ids.is_empty() {
            all_provider_ids(registry)
        } else {
            ids
        };
        let mut results = stream::iter(ids.into_iter().enumerate().map(|(index, id)| async move {
            let result = match registry.lookup(&id) {
                Some(definition) => self.check_one(&id, &definition.definition).await,
                None => Ok(not_found(&id)),
            };
            (index, result)
        }))
        .buffer_unordered(MAX_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        results.sort_by_key(|(index, _)| *index);
        let results = results
            .into_iter()
            .map(|(_, result)| result)
            .collect::<Result<_, _>>()?;
        let checked_at = format_go_rfc3339_layout((self.dependencies.clock)(), 0, false)
            .map_err(|_| CliVersionFailure::ClockUnavailable)?;
        Ok(CliVersionResponse {
            checked_at,
            results,
        })
    }
    /// Reusable by before/after update callers; still resolves launch A and
    /// version_args, never update executable B or an updater admission lease.
    pub async fn check_one(
        &self,
        id: &str,
        definition: &Definition,
    ) -> Result<CliVersionResult, CliVersionFailure> {
        let Some(launch) = &definition.launch else {
            return Ok(not_found(id));
        };
        let environment = self
            .dependencies
            .path_environment
            .sanitize(&self.dependencies.environment)
            .map_err(|_| CliVersionFailure::EnvironmentUnavailable)?;
        let resolver = Resolver::new(
            self.dependencies.platform,
            &environment,
            &self.dependencies.cwd,
            self.dependencies.executable_fs.as_ref(),
        );
        let path = std::iter::once(launch.executable.as_str())
            .chain(launch.executable_candidates.iter().map(String::as_str))
            .filter(|candidate| !candidate.is_empty())
            .find_map(|candidate| resolver.look_path_like_spawn(candidate).ok());
        let Some(path) = path else {
            return Ok(not_found(id));
        };
        let args = crate::update::plan::version_args(definition.update.as_ref());
        let command = resolver.resolve(&path, &args);
        let modified = (self.dependencies.modified_at)(&command.executable)
            .map(|timestamp| format_go_rfc3339_layout(timestamp, 0, false))
            .transpose()
            .map_err(|_| CliVersionFailure::ClockUnavailable)?
            .unwrap_or_default();
        let executor = self
            .dependencies
            .executor
            .as_ref()
            .ok_or(CliVersionFailure::RunnerUnavailable)?;
        let executable = command.executable.clone();
        let output = executor
            .execute(CliVersionCommand {
                executable: command.executable,
                args: command.args,
                cwd: self.dependencies.cwd.clone(),
                environment,
                platform: self.dependencies.platform,
                timeout: CHECK_TIMEOUT,
                output_cap: OUTPUT_CAP,
            })
            .await?;
        Ok(classify(id, executable, modified, output))
    }
}
struct Completion<'a> {
    owner: &'a CliVersionHttp,
    flight: Arc<Flight>,
    published: bool,
}
impl Completion<'_> {
    fn publish(&mut self, result: RunResult) {
        let mut state = self.owner.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(result) = &result {
            state.last = Some(result.clone());
        }
        if state
            .inflight
            .as_ref()
            .is_some_and(|flight| Arc::ptr_eq(flight, &self.flight))
        {
            state.inflight = None;
        }
        self.flight.send_replace(Some(result));
        self.published = true;
    }
}
impl Drop for Completion<'_> {
    fn drop(&mut self) {
        if !self.published {
            self.publish(Err(CliVersionFailure::OwnerDropped));
        }
    }
}
fn should_decode_body(request: &Request) -> bool {
    if request
        .header("Transfer-Encoding")
        .split(',')
        .any(|c| c.trim().eq_ignore_ascii_case("chunked"))
    {
        return true;
    }
    if let Ok(length) = request.header("Content-Length").parse::<u64>() {
        return length != 0;
    }
    !request.body.is_empty()
}
pub fn all_provider_ids(registry: &Registry) -> Vec<String> {
    registry
        .list()
        .into_iter()
        .filter(|summary| summary.enabled && summary.id != "shell")
        .map(|summary| summary.id)
        .collect()
}
pub fn normalize_provider_ids(ids: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .filter(|id| seen.insert((*id).to_owned()))
        .map(str::to_owned)
        .collect()
}
pub fn first_non_empty_line(text: &str) -> String {
    text.split('\n')
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_owned()
}
fn not_found(id: &str) -> CliVersionResult {
    CliVersionResult {
        provider: id.into(),
        error: "見つからない".into(),
        ..Default::default()
    }
}
pub fn classify(
    id: &str,
    executable: String,
    modified_at: String,
    output: CliVersionProcessOutput,
) -> CliVersionResult {
    let text = go_utf8_lossy(&output.output[..output.output.len().min(OUTPUT_CAP)]);
    let error = if output.timed_out {
        "打ち切り".into()
    } else if output.start_failed {
        "見つからない".into()
    } else if output.exit_code != 0 {
        format!("終了コード {}", output.exit_code)
    } else if text.trim().is_empty() {
        "出力が空".into()
    } else {
        String::new()
    };
    CliVersionResult {
        provider: id.into(),
        executable,
        version_line: first_non_empty_line(&text),
        version_text: text,
        executable_modified_at: modified_at,
        exit_code: output.exit_code,
        error,
    }
}
