use super::*;
use crate::process::{
    self, ExitOutcome, ProcessPlan, SpawnOptions,
    execpath::{NativeFs, Platform, Resolver},
};
pub struct NativeDiagnosticIo {
    pub paths: RuntimePaths,
    pub cwd: PathBuf,
    pub environment: Vec<String>,
}
impl DiagnosticIo for NativeDiagnosticIo {
    fn look_path(&self, name: &str) -> io::Result<String> {
        Resolver::new(Platform::native(), &self.environment, &self.cwd, &NativeFs).look_path(name)
    }
    fn command<'a>(
        &'a self,
        executable: &'a str,
        args: Vec<String>,
        cwd: &'a Path,
        timeout: Duration,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<ProbeOutput>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                let root = self.paths.root().canonicalize()?;
                if !crate::files::scope::under_roots(
                    Path::new(executable),
                    std::slice::from_ref(&root),
                ) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "trial diagnostic requires contained synthetic CLI",
                    ));
                }
                let executable_path = Path::new(executable).canonicalize()?;
                let working_directory = cwd.canonicalize()?;
                if !executable_path.starts_with(&root) || !working_directory.starts_with(&root) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "trial diagnostic requires contained synthetic CLI",
                    ));
                }
            }
            let resolved = Resolver::new(Platform::native(), &self.environment, cwd, &NativeFs)
                .resolve(executable, &args);
            let plan = ProcessPlan {
                executable: resolved.executable.into(),
                args: resolved.args.into_iter().map(Into::into).collect(),
                cwd: cwd.into(),
                env: self
                    .environment
                    .iter()
                    .filter_map(|s| s.split_once('='))
                    .map(|(k, v)| (k.into(), Some(v.into())))
                    .collect(),
                stdin: vec![],
                timeout,
                output_cap: 1024 * 1024,
                pipe_drain_timeout: Duration::from_secs(2),
            };
            let output = process::run_capped_with_options(
                &plan,
                cancel,
                SpawnOptions {
                    no_window: true,
                    env_clear: true,
                    stdin_null: true,
                    ..Default::default()
                },
            )
            .await?;
            Ok(ProbeOutput {
                success: matches!(output.outcome, ExitOutcome::Exited { code: Some(0), .. })
                    && !output.stdout_truncated,
                bytes: output.stdout,
            })
        })
    }
    fn http_status<'a>(
        &'a self,
        url: &'a str,
        timeout: Duration,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<u16>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial requires injected diagnostic HTTP",
                ));
            }
            let client = reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .map_err(io::Error::other)?;
            tokio::select! {_ = cancel.cancelled()=>Err(io::Error::new(io::ErrorKind::Interrupted,"diagnostic cancelled")),reply=client.get(url).send()=>reply.map(|r|r.status().as_u16()).map_err(io::Error::other)}
        })
    }
    fn pid_alive(&self, pid: u32) -> bool {
        crate::process::pid_alive(i64::from(pid))
    }
}
impl super::bug_report::BugReportIo for NativeDiagnosticIo {
    fn look_path(&self, name: &str) -> io::Result<String> {
        DiagnosticIo::look_path(self, name)
    }
    fn create_secret_gist<'a>(
        &'a self,
        executable: &'a str,
        markdown: &'a str,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<String>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial reports cannot publish external gists",
                ));
            }
            let args = vec![
                "gist".into(),
                "create".into(),
                "--secret".into(),
                "--filename".into(),
                "many-ai-cli-report-log.md".into(),
            ];
            let resolved =
                Resolver::new(Platform::native(), &self.environment, &self.cwd, &NativeFs)
                    .resolve(executable, &args);
            let plan = ProcessPlan {
                executable: resolved.executable.into(),
                args: resolved.args.into_iter().map(Into::into).collect(),
                cwd: self.cwd.clone(),
                env: self
                    .environment
                    .iter()
                    .filter_map(|s| s.split_once('='))
                    .map(|(k, v)| (k.into(), Some(v.into())))
                    .collect(),
                stdin: super::report::redact(markdown).into_bytes(),
                timeout: Duration::from_secs(30),
                output_cap: 4096,
                pipe_drain_timeout: Duration::from_secs(2),
            };
            let out = process::run_capped_with_options(
                &plan,
                cancel,
                SpawnOptions {
                    no_window: true,
                    env_clear: true,
                    stderr_null: true,
                    ..Default::default()
                },
            )
            .await?;
            if out.stdout_truncated {
                return Err(io::Error::other("gist command output too large"));
            }
            if !matches!(out.outcome, ExitOutcome::Exited { code: Some(0), .. }) {
                return Err(io::Error::other("gist command failed"));
            }
            Ok(String::from_utf8_lossy(&out.stdout).trim().into())
        })
    }
}
