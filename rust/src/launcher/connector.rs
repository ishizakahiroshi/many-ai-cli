//! Remote lifecycle on the shared contained process driver. No Go fallback.
use super::{Profile, RemoteTrial, UrlScanner, commands::*, network};
use crate::{
    config::RuntimePaths,
    process::{
        Cancellation, ExitOutcome, ManagedProcess, OutputStream, ProcessEvent, ProcessOutput,
        ProcessPlan, SpawnOptions,
    },
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{broadcast, oneshot};

pub type OutputSink = Arc<dyn Fn(OutputStream, &[u8]) + Send + Sync>;
#[derive(Clone)]
pub struct ConnectorConfig {
    pub paths: RuntimePaths,
    pub cwd: PathBuf,
    pub ssh_executable: PathBuf,
    pub wsl_executable: PathBuf,
    pub environment: BTreeMap<OsString, Option<OsString>>,
    pub remote_trial: Option<RemoteTrial>,
    /// None is quiet. A console sink is selected explicitly by the CLI/UI caller.
    pub output: Option<OutputSink>,
    pub poll_interval: Duration,
    pub poll_tries: usize,
}
impl ConnectorConfig {
    pub fn new(paths: RuntimePaths, cwd: PathBuf) -> Self {
        Self {
            paths,
            cwd,
            ssh_executable: if cfg!(windows) {
                "ssh.exe".into()
            } else {
                "ssh".into()
            },
            wsl_executable: "wsl.exe".into(),
            environment: BTreeMap::new(),
            remote_trial: None,
            output: None,
            poll_interval: Duration::from_millis(500),
            poll_tries: 20,
        }
    }
    pub fn with_console(mut self) -> Self {
        self.output = Some(console_sink());
        self
    }
    pub fn plan(
        &self,
        executable: &std::path::Path,
        args: Vec<String>,
        timeout: Duration,
    ) -> ProcessPlan {
        ProcessPlan {
            executable: executable.into(),
            args: args.into_iter().map(OsString::from).collect(),
            cwd: self.cwd.clone(),
            env: self.environment.clone(),
            stdin: Vec::new(),
            timeout,
            output_cap: 8 * 1024 * 1024,
            pipe_drain_timeout: Duration::from_secs(2),
        }
    }
    pub fn start(&self, mut profile: Profile) -> Result<Connection, String> {
        profile.validate()?;
        profile.normalize();
        match profile.kind.as_str() {
            "ssh" => {}
            "wsl" if cfg!(windows) => {}
            "wsl" => return Err("WSL profiles are only supported on Windows".into()),
            kind => return Err(format!("unsupported profile type {}", super::quote(kind))),
        }
        validate_remote_scope(&self.paths, self.remote_trial.as_ref())?;
        // A tunnel attaches to an existing instance; trial mode must never
        // silently attach to the daily default listener.
        if self.paths.is_trial()
            && profile.mode() == "tunnel"
            && profile.hub_port != i64::from(self.paths.port())
        {
            return Err("trial tunnel port must equal the isolated runtime port".into());
        }
        let (url_tx, url_rx) = oneshot::channel();
        let cancel = Cancellation::default();
        let task_cancel = cancel.clone();
        let mut config = self.clone();
        if let Some(scope) = &mut config.remote_trial {
            scope.connection_id = Some(
                crate::process::random_token()
                    .map_err(|_| "connection identity generation failed")?,
            );
        }
        let task = tokio::spawn(async move {
            let mut sender = Some(url_tx);
            let result = if profile.kind == "wsl" {
                config.run_wsl(&profile, &task_cancel, &mut sender).await
            } else if profile.mode() == "tunnel" {
                config.run_tunnel(&profile, &task_cancel, &mut sender).await
            } else {
                config.run_serve(&profile, &task_cancel, &mut sender).await
            };
            if let Some(sender) = sender {
                let _ = sender
                    .send(Err(result.clone().err().unwrap_or_else(|| {
                        "Connection failed (Hub URL was not received)".into()
                    })));
            }
            result
        });
        Ok(Connection {
            url: Some(url_rx),
            cancel,
            task: Some(task),
            finished: None,
        })
    }
    async fn run_serve(
        &self,
        p: &Profile,
        cancel: &Cancellation,
        sender: &mut Option<oneshot::Sender<Result<String, String>>>,
    ) -> Result<(), String> {
        let base = if p.hub_port == 0 {
            i64::from(
                network::pick_port(if self.paths.is_trial() {
                    self.paths.port()
                } else {
                    47777
                })
                .await,
            )
        } else {
            p.hub_port
        };
        let mut last_error = "ssh serve: max retries exceeded".into();
        for attempt in 0..5 {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let port = base + attempt * 100;
            if port > 65535 || (self.paths.is_trial() && port == 47777) {
                last_error = "ssh serve: port candidate out of range".into();
                break;
            }
            let plan = self.plan(
                &self.ssh_executable,
                ssh_serve_args(p, port, self.remote_trial.as_ref()),
                Duration::ZERO,
            );
            let (mut child, mut events) = ManagedProcess::spawn_owned_with_options(
                plan,
                512,
                SpawnOptions { no_window: true },
            );
            let mut scans = Scanners::default();
            let ready = await_url(
                &mut child,
                &mut events,
                &mut scans,
                self.output.as_ref(),
                cancel,
            )
            .await;
            match ready {
                Ok(url) if url.contains(&format!("127.0.0.1:{port}/")) => {
                    send_ready(sender, url);
                    // Go SSH regards a process exit after ready as normal end.
                    let result = watch_child(
                        &mut child,
                        &mut events,
                        &mut scans,
                        self.output.as_ref(),
                        cancel,
                    )
                    .await;
                    self.cleanup(p, port, false).await;
                    return result.map(|_| ());
                }
                Ok(_) => last_error = format!("port mismatch: expected {port} in remote Hub URL"),
                Err(error) => last_error = error,
            }
            child.close();
            let _ = child.wait().await;
            self.cleanup(p, port, false).await;
            if cancel.is_cancelled() {
                return Ok(());
            }
        }
        Err(last_error)
    }
    async fn run_wsl(
        &self,
        p: &Profile,
        cancel: &Cancellation,
        sender: &mut Option<oneshot::Sender<Result<String, String>>>,
    ) -> Result<(), String> {
        let port = if p.hub_port == 0 {
            i64::from(
                network::pick_port(if self.paths.is_trial() {
                    self.paths.port()
                } else {
                    47777
                })
                .await,
            )
        } else {
            p.hub_port
        };
        let plan = self.plan(
            &self.wsl_executable,
            wsl_serve_args(p, port, self.remote_trial.as_ref()),
            Duration::ZERO,
        );
        let (mut child, mut events) =
            ManagedProcess::spawn_owned_with_options(plan, 512, SpawnOptions { no_window: true });
        let mut scans = Scanners::default();
        let result = match await_url(
            &mut child,
            &mut events,
            &mut scans,
            self.output.as_ref(),
            cancel,
        )
        .await
        {
            Ok(url) => {
                send_ready(sender, url);
                watch_child(
                    &mut child,
                    &mut events,
                    &mut scans,
                    self.output.as_ref(),
                    cancel,
                )
                .await
                .and_then(|o| exit_success(&o, "wsl.exe"))
            }
            Err(error) => Err(error),
        };
        child.close();
        let _ = child.wait().await;
        self.cleanup(p, port, true).await;
        if cancel.is_cancelled() {
            Ok(())
        } else {
            result
        }
    }
    async fn cleanup(&self, p: &Profile, port: i64, wsl: bool) {
        let (executable, args) = if wsl {
            (
                &self.wsl_executable,
                wsl_cleanup_args(p, port, self.remote_trial.as_ref()),
            )
        } else {
            (
                &self.ssh_executable,
                ssh_cleanup_args(p, port, self.remote_trial.as_ref()),
            )
        };
        let plan = self.plan(executable, args, Duration::from_secs(5));
        let (mut process, _) =
            ManagedProcess::spawn_owned_with_options(plan, 16, SpawnOptions { no_window: true });
        let _ = process.wait().await;
    }
    async fn run_tunnel(
        &self,
        p: &Profile,
        cancel: &Cancellation,
        sender: &mut Option<oneshot::Sender<Result<String, String>>>,
    ) -> Result<(), String> {
        let plan = self.plan(&self.ssh_executable, ssh_tunnel_args(p), Duration::ZERO);
        let (mut tunnel, mut events) =
            ManagedProcess::spawn_owned_with_options(plan, 512, SpawnOptions { no_window: true });
        // Observe actual successful process creation before running token_command.
        loop {
            tokio::select! {
                _=cancel.cancelled()=>{tunnel.close(); let _=tunnel.wait().await; return Ok(());},
                result=tunnel.wait()=>return Err(result.err().map(|e|format!("start ssh tunnel: {e}")).unwrap_or_else(||"ssh tunnel exited before ready".into())),
                event=events.recv()=>match event {
                    Ok(ProcessEvent::Started{..})=>break,
                    Ok(ProcessEvent::Output{stream,bytes})=>emit(self.output.as_ref(),stream,&bytes),
                    Err(broadcast::error::RecvError::Lagged(_))=>return Err("ssh tunnel output overflow".into()),
                    Err(broadcast::error::RecvError::Closed)=>{},
                }
            }
        }
        let result = async {
            let token = tunnel_phase(
                self.fetch_token(p, cancel),
                &mut tunnel,
                &mut events,
                self.output.as_ref(),
                cancel,
            )
            .await?;
            tunnel_phase(
                network::poll_until_ready(
                    p.hub_port,
                    &token,
                    cancel,
                    self.poll_interval,
                    self.poll_tries,
                ),
                &mut tunnel,
                &mut events,
                self.output.as_ref(),
                cancel,
            )
            .await
            .map_err(|e| format!("hub not ready: {e}"))?;
            network::post_net_hint(p.hub_port, &token, &p.host, cancel).await;
            if cancel.is_cancelled() {
                return Ok(());
            }
            send_ready(sender, tunnel_hub_url(p.hub_port, &token, &p.host));
            let mut scans = Scanners {
                raw_stderr: true,
                ..Scanners::default()
            };
            watch_child(
                &mut tunnel,
                &mut events,
                &mut scans,
                self.output.as_ref(),
                cancel,
            )
            .await
            .map(|_| ())
        }
        .await;
        tunnel.close();
        let _ = tunnel.wait().await;
        if cancel.is_cancelled() {
            Ok(())
        } else {
            result
        }
    }
    async fn fetch_token(&self, p: &Profile, cancel: &Cancellation) -> Result<String, String> {
        let plan = self.plan(
            &self.ssh_executable,
            ssh_token_args(p),
            Duration::from_secs(5),
        );
        let output = self.run_short(plan, cancel).await?;
        exit_success(&output, "token_command")?;
        if output.stdout_truncated {
            return Err("token_command output exceeds limit".into());
        }
        let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if token.is_empty() {
            return Err("token_command returned empty output".into());
        }
        normalize_hub_token(&token)
            .map_err(|error| format!("token_command returned invalid token: {error}"))?;
        Ok(token)
    }
    pub(crate) async fn run_short(
        &self,
        plan: ProcessPlan,
        cancel: &Cancellation,
    ) -> Result<ProcessOutput, String> {
        let (mut child, _) =
            ManagedProcess::spawn_owned_with_options(plan, 16, SpawnOptions { no_window: true });
        tokio::select! {
            output=child.wait()=>output.map_err(|e|e.to_string()),
            _=cancel.cancelled()=>{child.close();let _=child.wait().await;Err("cancelled".into())}
        }
    }
}
pub struct Connection {
    url: Option<oneshot::Receiver<Result<String, String>>>,
    cancel: Cancellation,
    task: Option<tokio::task::JoinHandle<Result<(), String>>>,
    finished: Option<Result<(), String>>,
}
impl Connection {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn cancellation(&self) -> Cancellation {
        self.cancel.clone()
    }
    pub async fn ready(&mut self) -> Result<String, String> {
        let rx = self
            .url
            .as_mut()
            .ok_or("connection readiness already consumed")?;
        let result = rx
            .await
            .map_err(|_| "connection ended before readiness".to_owned())?;
        self.url = None;
        result
    }
    pub async fn wait(&mut self) -> Result<(), String> {
        if self.finished.is_none() {
            let result = match self.task.as_mut().ok_or("connection task missing")?.await {
                Ok(result) => result,
                Err(_) => Err("connection task interrupted".into()),
            };
            self.task = None;
            self.finished = Some(result);
        }
        self.finished.clone().unwrap()
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        // Do not abort the owner: cancellation must reach scoped remote cleanup.
        // ProcessPlan itself still contains/cleans its exact local descendants.
        self.cancel.cancel();
    }
}
fn send_ready(sender: &mut Option<oneshot::Sender<Result<String, String>>>, url: String) {
    if let Some(sender) = sender.take() {
        let _ = sender.send(Ok(url));
    }
}
pub fn console_sink() -> OutputSink {
    Arc::new(|stream, bytes| {
        let _ = match stream {
            OutputStream::Stdout => io::stdout().write_all(bytes),
            OutputStream::Stderr => io::stderr().write_all(bytes),
        };
    })
}
fn emit(sink: Option<&OutputSink>, stream: OutputStream, bytes: &[u8]) {
    if let Some(sink) = sink {
        sink(stream, bytes);
    }
}
#[derive(Default)]
struct Scanners {
    raw_stderr: bool,
    stdout: UrlScanner,
    stderr: UrlScanner,
}
impl Scanners {
    fn feed(
        &mut self,
        stream: OutputStream,
        bytes: &[u8],
        eof: bool,
        sink: Option<&OutputSink>,
    ) -> Option<String> {
        if self.raw_stderr {
            if stream == OutputStream::Stderr {
                emit(sink, stream, bytes);
            }
            return None;
        }
        let scanner = match stream {
            OutputStream::Stdout => &mut self.stdout,
            OutputStream::Stderr => &mut self.stderr,
        };
        let mut found = None;
        for (line, url) in scanner.feed(bytes, eof) {
            emit(sink, stream, &line);
            if found.is_none() {
                found = url;
            }
        }
        found
    }
    fn finish(&mut self, sink: Option<&OutputSink>) -> Option<String> {
        let stdout = self.feed(OutputStream::Stdout, &[], true, sink);
        let stderr = self.feed(OutputStream::Stderr, &[], true, sink);
        stdout.or(stderr)
    }
}
async fn tunnel_phase<T>(
    future: impl std::future::Future<Output = Result<T, String>>,
    child: &mut ManagedProcess,
    events: &mut broadcast::Receiver<ProcessEvent>,
    sink: Option<&OutputSink>,
    cancel: &Cancellation,
) -> Result<T, String> {
    tokio::pin!(future);
    let mut open = true;
    loop {
        tokio::select! {
            result=&mut future=>return result,
            _=cancel.cancelled()=>return Err("cancelled".into()),
            _=child.wait()=>return Err("ssh tunnel exited before ready".into()),
            event=events.recv(),if open=>match event {
                Ok(ProcessEvent::Output { stream: OutputStream::Stderr, bytes })=>emit(sink,OutputStream::Stderr,&bytes),
                Ok(_)=>{},
                Err(broadcast::error::RecvError::Closed)=>open=false,
                Err(broadcast::error::RecvError::Lagged(_))=>return Err("ssh tunnel output overflow".into()),
            }
        }
    }
}
async fn await_url(
    child: &mut ManagedProcess,
    events: &mut broadcast::Receiver<ProcessEvent>,
    scans: &mut Scanners,
    sink: Option<&OutputSink>,
    cancel: &Cancellation,
) -> Result<String, String> {
    let mut events_open = true;
    loop {
        tokio::select! {
            biased;
            _=cancel.cancelled()=>{child.close();let _=child.wait().await;return Err("cancelled".into());},
            event=events.recv(),if events_open=>match event{
                Ok(ProcessEvent::Output{stream,bytes})=>if let Some(url)=scans.feed(stream,&bytes,false,sink){return Ok(url);},
                Ok(ProcessEvent::Started{..})=>{},
                Err(broadcast::error::RecvError::Closed)=>events_open=false,
                Err(broadcast::error::RecvError::Lagged(_))=>return Err("remote output overflow before Hub URL".into()),
            },
            output=child.wait()=>{let output=output.map_err(|e|format!("start remote connector: {e}"))?;if let Some(url)=scans.finish(sink){return Ok(url);}exit_success(&output,"remote connector")?;return Err("remote connector exited before Hub URL was detected".into());}
        }
    }
}
async fn watch_child(
    child: &mut ManagedProcess,
    events: &mut broadcast::Receiver<ProcessEvent>,
    scans: &mut Scanners,
    sink: Option<&OutputSink>,
    cancel: &Cancellation,
) -> Result<ProcessOutput, String> {
    let mut events_open = true;
    loop {
        tokio::select! {
            biased;
            _=cancel.cancelled()=>{child.close();return child.wait().await.map_err(|e|e.to_string());},
            event=events.recv(),if events_open=>match event{
                Ok(ProcessEvent::Output{stream,bytes})=>{scans.feed(stream,&bytes,false,sink);},Ok(ProcessEvent::Started{..})=>{},
                Err(broadcast::error::RecvError::Closed)=>events_open=false,
                Err(broadcast::error::RecvError::Lagged(_))=>return Err("remote output overflow".into()),
            },
            output=child.wait()=>{scans.finish(sink);return output.map_err(|e|e.to_string());}
        }
    }
}
pub(crate) fn exit_success(output: &ProcessOutput, label: &str) -> Result<(), String> {
    match output.outcome {
        ExitOutcome::Exited { code: Some(0), .. } => Ok(()),
        ExitOutcome::Cancelled => Err(format!("{label}: cancelled")),
        ExitOutcome::TimedOut => Err(format!("{label}: timed out")),
        ExitOutcome::Exited { code, signal } => {
            Err(format!("{label} exited (code {code:?}, signal {signal:?})"))
        }
    }
}
