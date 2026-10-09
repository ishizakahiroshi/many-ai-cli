use super::InstructionRootResolver;
use crate::{
    config::RuntimePaths,
    process::{
        Cancellation, ExitOutcome, ManagedProcess, ProcessPlan, SpawnOptions,
        execpath::{NativeFs, Platform, Resolver},
    },
    proto::core::CoreFuture,
};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    time::Duration,
};
pub struct NativeInstructionRootResolver {
    paths: RuntimePaths,
    cwd: PathBuf,
    environment: Vec<String>,
    hub: Cancellation,
}
impl NativeInstructionRootResolver {
    pub fn new(
        paths: RuntimePaths,
        cwd: PathBuf,
        environment: Vec<String>,
        hub: Cancellation,
    ) -> io::Result<Self> {
        if !cwd.is_absolute()
            || !cwd.is_dir()
            || environment
                .iter()
                .any(|v| v.contains('\0') || v.split_once('=').is_none())
        {
            return Err(io::Error::other(
                "instruction resolver requires explicit native context",
            ));
        }
        Ok(Self {
            paths,
            cwd,
            environment,
            hub,
        })
    }
}
impl InstructionRootResolver for NativeInstructionRootResolver {
    fn root<'a>(&'a self, cwd: &'a Path, cancel: &'a Cancellation) -> CoreFuture<'a, PathBuf> {
        Box::pin(async move {
            let fallback = crate::files::scope::clean(&if cwd.is_absolute() {
                cwd.to_path_buf()
            } else {
                self.cwd.join(cwd)
            });
            if self.paths.is_trial() || self.hub.is_cancelled() || cancel.is_cancelled() {
                return fallback;
            }
            let resolver =
                Resolver::new(Platform::native(), &self.environment, &self.cwd, &NativeFs);
            let Ok(executable) = resolver.look_path("git") else {
                return fallback;
            };
            let env = self
                .environment
                .iter()
                .filter_map(|v| v.split_once('='))
                .map(|(k, v)| (OsString::from(k), Some(OsString::from(v))))
                .collect();
            let plan = ProcessPlan {
                executable: executable.into(),
                args: vec!["rev-parse".into(), "--show-toplevel".into()],
                cwd: fallback.clone(),
                env,
                stdin: vec![],
                timeout: Duration::from_millis(250),
                output_cap: 64 * 1024,
                pipe_drain_timeout: Duration::from_millis(100),
            };
            let (mut process, _events) = ManagedProcess::spawn_owned_with_options(
                plan,
                8,
                SpawnOptions {
                    stdin_null: true,
                    env_clear: true,
                    no_window: true,
                    ..Default::default()
                },
            );
            let output = tokio::select! {result=process.wait()=>result,_=cancel.cancelled()=>{process.close();process.wait().await},_=self.hub.cancelled()=>{process.close();process.wait().await}};
            if let Ok(output) = output
                && matches!(output.outcome, ExitOutcome::Exited { code: Some(0), .. })
                && !output.stdout_truncated
            {
                let root = String::from_utf8_lossy(&output.stdout);
                if !root.trim().is_empty() {
                    return crate::files::scope::clean(Path::new(root.trim()));
                }
            }
            fallback
        })
    }
}
