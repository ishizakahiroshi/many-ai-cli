//! Explicit CLI update jobs retain the canonical provider admission lease.
use crate::{
    application::event_observer::EventWarning,
    config::RuntimePaths,
    files::safe_fs::Dir,
    hub::{
        cli_version::{CliVersionHttp, all_provider_ids, normalize_provider_ids},
        http::Response,
        task_owner::HubTaskHandle,
    },
    process::{Cancellation, ExitOutcome, ProcessOutput, SpawnOptions, run_capped_with_options},
    profile::{registry::Registry, store::ProviderRegistryStore},
    proto::{
        core::*,
        time::{Timestamp, format_go_rfc3339_layout},
    },
    terminal::session::SessionEngine,
    update::{
        job::classify,
        plan::{UpdatePlan, plan_update},
    },
};
use futures_util::{StreamExt, stream};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read},
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[cfg(test)]
mod tests;
pub type Resolve = Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;
pub trait UpdateExecutor: Send + Sync {
    fn execute<'a>(
        &'a self,
        plan: &'a UpdatePlan,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, std::io::Result<ProcessOutput>>;
}
pub struct NativeUpdateExecutor;
impl UpdateExecutor for NativeUpdateExecutor {
    fn execute<'a>(
        &'a self,
        plan: &'a UpdatePlan,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, std::io::Result<ProcessOutput>> {
        Box::pin(async move {
            let process = normalize_process(
                &plan.command.process,
                crate::process::execpath::Platform::native(),
                &crate::process::execpath::NativeFs,
            );
            run_capped_with_options(
                &process,
                cancel,
                SpawnOptions {
                    combined_output: true,
                    no_window: true,
                    env_clear: true,
                    stdin_null: true,
                    ..Default::default()
                },
            )
            .await
        })
    }
}
fn normalize_process(
    plan: &crate::process::ProcessPlan,
    platform: crate::process::execpath::Platform,
    fs: &dyn crate::process::execpath::ExecutableFs,
) -> crate::process::ProcessPlan {
    let environment = plan
        .env
        .iter()
        .filter_map(|(key, value)| {
            value
                .as_ref()
                .map(|value| format!("{}={}", key.to_string_lossy(), value.to_string_lossy()))
        })
        .collect::<Vec<_>>();
    let args = plan
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let command = crate::process::execpath::Resolver::new(platform, &environment, &plan.cwd, fs)
        .resolve(&plan.executable.to_string_lossy(), &args);
    let mut resolved = plan.clone();
    resolved.executable = command.executable.into();
    resolved.args = command.args.into_iter().map(Into::into).collect();
    resolved
}
pub struct Dependencies {
    pub core: Arc<SessionEngine>,
    pub store: Arc<ProviderRegistryStore>,
    pub versions: Arc<CliVersionHttp>,
    pub paths: RuntimePaths,
    pub cwd: PathBuf,
    pub environment: Vec<String>,
    pub tasks: HubTaskHandle,
    pub resolve: Resolve,
    pub executor: Arc<dyn UpdateExecutor>,
    pub warning: EventWarning,
    /// Actual current configured cli-updates directory, already scope-validated.
    pub log_directory: Arc<dyn Fn() -> io::Result<PathBuf> + Send + Sync>,
}
#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub provider: String,
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub executable: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version_before: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version_after: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub exit_code: i32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub started_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub finished_at: String,
    #[serde(skip_serializing_if = "is_false")]
    pub log_available: bool,
    #[serde(skip)]
    pub log_name: String,
    #[serde(skip)]
    pub log_directory: Option<Arc<Dir>>,
}
fn is_zero(v: &i32) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !*v
}
#[derive(Clone, Serialize)]
pub struct Excluded {
    pub provider: String,
    pub reason: String,
    #[serde(skip_serializing_if = "usize_zero")]
    pub running_sessions: usize,
}
fn usize_zero(v: &usize) -> bool {
    *v == 0
}
#[derive(Clone, Serialize)]
pub struct Eligibility {
    pub provider: String,
    pub eligible: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(skip_serializing_if = "usize_zero")]
    pub running_sessions: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub executable: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
}
pub struct Job {
    id: String,
    started: Timestamp,
    serial: bool,
    accepted: Vec<String>,
    excluded: Vec<Excluded>,
    statuses: Mutex<BTreeMap<String, Status>>,
}
impl Job {
    pub fn snapshot(&self) -> serde_json::Value {
        let statuses = self.statuses.lock().unwrap_or_else(|p| p.into_inner());
        let providers = self
            .accepted
            .iter()
            .filter_map(|id| statuses.get(id))
            .collect::<Vec<_>>();
        serde_json::json!({"job_id":self.id,"started_at":rfc(self.started),"serial":self.serial,"providers":providers,"excluded":if self.excluded.is_empty(){None}else{Some(&self.excluded)}})
    }
}
fn rfc(at: Timestamp) -> String {
    format_go_rfc3339_layout(at, 0, false).unwrap_or_default()
}
struct Lease {
    core: Arc<SessionEngine>,
    lease: Option<ProviderUpdateLease>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            self.core.end_provider_update(lease);
        }
    }
}
struct Work {
    plan: UpdatePlan,
    definition: crate::proto::provider::Definition,
    _lease: Lease,
    job: Option<Arc<Job>>,
}
impl Drop for Work {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            let mut statuses = job.statuses.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(status) = statuses.get_mut(&self.plan.command.provider)
                && matches!(status.state.as_str(), "queued" | "running")
            {
                status.state = "failed".into();
                status.exit_code = -1;
                status.finished_at = rfc(Timestamp::now());
            }
        }
    }
}
pub struct CliUpdates {
    dependencies: Dependencies,
    jobs: Mutex<VecDeque<Arc<Job>>>,
}
impl CliUpdates {
    pub fn new(dependencies: Dependencies) -> Arc<Self> {
        Arc::new(Self {
            dependencies,
            jobs: Mutex::new(VecDeque::new()),
        })
    }
    fn registry(&self) -> Result<Arc<Registry>, Response> {
        self.dependencies
            .store
            .snapshot()
            .map(|s| s.registry)
            .map_err(|_| {
                Response::error(
                    503,
                    "provider_registry_unavailable",
                    "provider registry is unavailable",
                )
            })
    }
    fn plan(
        &self,
        id: &str,
        registry: &Registry,
    ) -> Result<(UpdatePlan, crate::proto::provider::Definition), String> {
        let definition = registry.lookup(id).ok_or("not_installed")?.definition;
        let mut plan = plan_update(&definition, &self.dependencies.cwd, |name| {
            (self.dependencies.resolve)(name)
        })
        .map_err(|e| e.code().to_owned())?;
        if self.dependencies.paths.is_trial() {
            let root = std::fs::canonicalize(self.dependencies.paths.root())
                .map_err(|_| "not_installed".to_owned())?;
            for path in [&plan.command.process.executable, &plan.launch_path] {
                if !std::fs::canonicalize(path).is_ok_and(|path| path.starts_with(&root)) {
                    return Err("not_installed".into());
                }
            }
        }
        plan.command.process.env = self
            .dependencies
            .environment
            .iter()
            .filter_map(|v| v.split_once('=').map(|(k, v)| (k.into(), Some(v.into()))))
            .collect();
        Ok((plan, definition))
    }
    pub fn eligibility(&self) -> Result<Vec<Eligibility>, Response> {
        let registry = self.registry()?;
        Ok(all_provider_ids(&registry)
            .into_iter()
            .map(|id| match self.plan(&id, &registry) {
                Err(reason) => Eligibility {
                    provider: id,
                    eligible: false,
                    reason,
                    running_sessions: 0,
                    executable: String::new(),
                    argv: vec![],
                },
                Ok((plan, _)) => {
                    let count = self.dependencies.core.provider_session_count(&id);
                    let reason = if self.dependencies.core.provider_updating(&id) {
                        "already_updating"
                    } else if count > 0 {
                        "running_sessions"
                    } else {
                        ""
                    };
                    Eligibility {
                        provider: id,
                        eligible: reason.is_empty(),
                        reason: reason.into(),
                        running_sessions: if reason == "running_sessions" {
                            count
                        } else {
                            0
                        },
                        executable: plan.selected_launch_executable.clone(),
                        argv: display_argv(&plan),
                    }
                }
            })
            .collect())
    }
    pub fn create(
        self: &Arc<Self>,
        providers: Vec<String>,
        serial: bool,
        at: Timestamp,
    ) -> Result<serde_json::Value, Response> {
        let ids = normalize_provider_ids(&providers);
        if ids.is_empty() {
            return Err(Response::error(400, "bad_request", "providers is required"));
        }
        let registry = self.registry()?;
        let permit = self.dependencies.tasks.effect_permit().map_err(|_| {
            Response::error(503, "update_unavailable", "CLI updater is shutting down")
        })?;
        let cancellation = permit.cancellation();
        let mut work = Vec::new();
        let mut excluded = Vec::new();
        let mut accepted = Vec::new();
        for id in ids {
            match self.plan(&id, &registry) {
                Err(reason) => excluded.push(Excluded {
                    provider: id,
                    reason,
                    running_sessions: 0,
                }),
                Ok((plan, definition)) => match self.dependencies.core.begin_provider_update(&id) {
                    Ok(lease) => {
                        accepted.push(id);
                        work.push(Work {
                            plan,
                            definition,
                            _lease: Lease {
                                core: self.dependencies.core.clone(),
                                lease: Some(lease),
                            },
                            job: None,
                        });
                    }
                    Err(error) => {
                        let (reason, count) = match error {
                            ProviderAdmissionError::AlreadyUpdating => ("already_updating", 0),
                            ProviderAdmissionError::RunningSessions { count }
                            | ProviderAdmissionError::PendingSpawns { count } => {
                                ("running_sessions", count)
                            }
                            _ => ("already_updating", 0),
                        };
                        excluded.push(Excluded {
                            provider: id,
                            reason: reason.into(),
                            running_sessions: count,
                        });
                    }
                },
            }
        }
        let id = crate::process::random_token()
            .map(|token| format!("cliu-{}", &token[..16]))
            .unwrap_or_else(|_| {
                format!(
                    "cliu-{:x}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                )
            });
        let job = Arc::new(Job {
            id: id.clone(),
            started: at,
            serial,
            statuses: Mutex::new(
                accepted
                    .iter()
                    .map(|id| {
                        (
                            id.clone(),
                            Status {
                                provider: id.clone(),
                                state: "queued".into(),
                                ..Default::default()
                            },
                        )
                    })
                    .collect(),
            ),
            accepted: accepted.clone(),
            excluded: excluded.clone(),
        });
        for work in &mut work {
            work.job = Some(job.clone());
        }
        {
            let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
            jobs.push_back(job.clone());
            while jobs.len() > 10 {
                jobs.pop_front();
            }
        }
        let owner = self.clone();
        drop(permit.start(async move {
            if serial {
                for work in work {
                    owner.run_one(job.clone(), work, cancellation.token()).await;
                }
            } else {
                // The retained job directly owns these futures. Aborting the
                // job drops every process/lease synchronously; no nested task
                // can outlive its Hub effect receipt or journal shutdown.
                stream::iter(work)
                    .map(|work| {
                        let owner = owner.clone();
                        let job = job.clone();
                        let cancel = cancellation.token().clone();
                        async move { owner.run_one(job, work, &cancel).await }
                    })
                    .buffer_unordered(8)
                    .collect::<Vec<_>>()
                    .await;
            }
        }));
        Ok(
            serde_json::json!({"job_id":id,"accepted":if accepted.is_empty(){None}else{Some(accepted)},"excluded":excluded}),
        )
    }
    pub fn job(&self, id: &str) -> Option<Arc<Job>> {
        self.jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|j| j.id == id)
            .cloned()
    }
    pub fn log(&self, id: &str, provider: &str) -> Result<serde_json::Value, Response> {
        let job = self
            .job(id)
            .ok_or_else(|| Response::error(404, "not_found", "unknown job"))?;
        let (directory, name) = job
            .statuses
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(provider)
            .and_then(|s| s.log_directory.clone().map(|dir| (dir, s.log_name.clone())))
            .filter(|(_, name)| !name.is_empty())
            .ok_or_else(|| Response::error(404, "not_found", "log not available"))?;
        let mut file = directory
            .open_file(&name, false)
            .map_err(|_| Response::error(404, "not_found", "log not available"))?;
        let bytes = read_log(&mut file)?;
        Ok(
            serde_json::json!({"content":String::from_utf8_lossy(&bytes[..bytes.len().min(256*1024)]),"truncated":bytes.len()>256*1024}),
        )
    }
    async fn run_one(&self, job: Arc<Job>, work: Work, cancel: &Cancellation) {
        let started = Timestamp::now();
        let id = work.plan.command.provider.clone();
        let argv = display_argv(&work.plan);
        let mut status = Status {
            provider: id.clone(),
            state: "running".into(),
            executable: work.plan.selected_launch_executable.clone(),
            argv: argv.clone(),
            started_at: rfc(started),
            ..Default::default()
        };
        job.statuses
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), status.clone());
        let before = tokio::select! {_=cancel.cancelled()=>None,result=self.dependencies.versions.check_one(&id,&work.definition)=>result.ok()};
        let output = if cancel.is_cancelled() {
            Ok(ProcessOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                pipes_forced_closed: false,
                outcome: ExitOutcome::Cancelled,
            })
        } else {
            self.dependencies.executor.execute(&work.plan, cancel).await
        };
        let (start_failed, text, outcome) = match &output {
            Ok(output) => (
                false,
                String::from_utf8_lossy(&output.stdout).into_owned(),
                output.outcome.clone(),
            ),
            Err(_) => (
                true,
                String::new(),
                ExitOutcome::Exited {
                    code: Some(-1),
                    signal: None,
                },
            ),
        };
        let after = if !start_failed
            && matches!(
                outcome,
                ExitOutcome::Exited {
                    code: Some(0),
                    signal: None
                }
            ) {
            tokio::select! {_=cancel.cancelled()=>None,result=self.dependencies.versions.check_one(&id,&work.definition)=>result.ok()}
        } else {
            None
        };
        let result = classify(
            &text,
            &outcome,
            start_failed,
            before
                .as_ref()
                .filter(|r| r.error.is_empty())
                .map(|r| r.version_text.as_str()),
            after
                .as_ref()
                .filter(|r| r.error.is_empty())
                .map(|r| r.version_text.as_str()),
        );
        let finished = Timestamp::now();
        status.state = result.as_str().into();
        status.exit_code = match outcome {
            ExitOutcome::Exited { code, .. } => code.unwrap_or(-1),
            _ => -1,
        };
        status.finished_at = rfc(finished);
        status.version_before = before
            .as_ref()
            .map(|r| r.version_line.clone())
            .unwrap_or_default();
        status.version_after = after
            .as_ref()
            .map(|r| r.version_line.clone())
            .unwrap_or_default();
        let payload = format!(
            "provider: {id}\nargv: {}\nresolved_executable: {}\nstarted_at: {}\nfinished_at: {}\nexit_code: {}\ntimed_out: {}\n{}version_before: {}\nversion_after: {}\n--- output ---\n{text}",
            argv.join(" "),
            work.plan.command.process.executable.display(),
            status.started_at,
            status.finished_at,
            status.exit_code,
            matches!(outcome, ExitOutcome::TimedOut),
            output
                .as_ref()
                .err()
                .map(|e| format!("start_error: {e}\n"))
                .unwrap_or_default(),
            status.version_before,
            status.version_after
        );
        let name = format!(
            "{}_{}.log",
            crate::proto::time::utc(started)
                .map(|time| time
                    .with_timezone(&chrono::Local)
                    .format("%Y%m%d-%H%M%S%.3f")
                    .to_string())
                .unwrap_or_default(),
            crate::orchestration::child_launch::safe_token(&id)
        );
        match (self.dependencies.log_directory)()
            .and_then(|path| Dir::open_or_create_private(&path))
            .and_then(|dir| {
                dir.replace(&name, payload.as_bytes(), 0o600)?;
                Ok(Arc::new(dir))
            }) {
            Ok(directory) => {
                status.log_available = true;
                status.log_name = name;
                status.log_directory = Some(directory);
            }
            Err(_) => (self.dependencies.warning)(
                "CLI update log",
                &SessionError::InvalidRequest("private update log unavailable".into()),
            ),
        }
        job.statuses
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, status);
        if let Some(result) = after.filter(|result| result.error.is_empty()) {
            self.dependencies.versions.refresh_update_result(result);
        } else if result == crate::update::job::Outcome::Latest
            && let Some(result) = before.filter(|result| result.error.is_empty())
        {
            self.dependencies.versions.refresh_update_result(result);
        }
    }
    /// Source log-retention sweep. The Hub's real periodic owner supplies now
    /// and the live setting; zero days disables deletion.
    pub fn clean_logs(&self, now: std::time::SystemTime, days: i64) -> io::Result<()> {
        if days <= 0 {
            return Ok(());
        }
        let directory = match Dir::open(&(self.dependencies.log_directory)()?) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let Some(cutoff) = now.checked_sub(std::time::Duration::from_secs(
            (days as u64).saturating_mul(86400),
        )) else {
            return Ok(());
        };
        for name in directory.entries()? {
            let Ok(file) = directory.open_file(&name, false) else {
                continue;
            };
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            // Windows private handles deliberately omit FILE_SHARE_DELETE.
            // Finish the handle-based metadata read before unlinking the file.
            drop(file);
            if metadata.modified().is_ok_and(|modified| modified < cutoff) {
                let _ = directory.remove_file(&name);
            }
        }
        Ok(())
    }
}
fn read_log(reader: &mut impl Read) -> Result<Vec<u8>, Response> {
    let mut bytes = Vec::new();
    reader
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Response::error(500, "read_failed", "cannot read log"))?;
    Ok(bytes)
}
fn display_argv(plan: &UpdatePlan) -> Vec<String> {
    std::iter::once(plan.requested_executable.clone())
        .chain(
            plan.command
                .process
                .args
                .iter()
                .map(|a| a.to_string_lossy().into_owned()),
        )
        .collect()
}
