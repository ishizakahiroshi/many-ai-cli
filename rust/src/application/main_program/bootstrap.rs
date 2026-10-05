//! Authenticated loopback discovery and owned Hub startup for provider aliases.
use super::{MainContext, context::environment_value};
use crate::{
    application::hub_runtime::{RuntimeLedger, probe_hub_info},
    config::ConfigStore,
    process::Cancellation,
};
use std::{
    io,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

pub async fn running_port(context: &MainContext) -> io::Result<Option<u16>> {
    let config = context.config.snapshot().map_err(io::Error::other)?.config;
    Ok(RuntimeLedger::open(&context.paths)?
        .running_port(config.hub.port, &config.token)
        .await)
}
fn set_port(context: &mut MainContext, port: u16) -> io::Result<()> {
    let mut config = context.config.snapshot().map_err(io::Error::other)?.config;
    config.hub.port = i64::from(port);
    context.config =
        Arc::new(ConfigStore::new(context.paths.clone(), config).map_err(io::Error::other)?);
    Ok(())
}
struct PendingHub(Option<Child>);
impl Drop for PendingHub {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl PendingHub {
    fn release(mut self) -> io::Result<()> {
        // The long-lived Hub intentionally outlives its requesting wrapper.
        // A direct-child reaper owns Wait; it has no Tokio/runtime dependency.
        // Start the reaper while the guard still owns the child, so a thread
        // allocation failure cannot leak an untracked startup process.
        let child = Arc::new(std::sync::Mutex::new(self.0.take()));
        let reaper = child.clone();
        match std::thread::Builder::new()
            .name("many-ai-hub-reaper".into())
            .spawn(move || {
                if let Some(mut child) = reaper.lock().unwrap_or_else(|p| p.into_inner()).take() {
                    let _ = child.wait();
                }
            }) {
            Ok(_) => Ok(()),
            Err(error) => {
                self.0 = child.lock().unwrap_or_else(|p| p.into_inner()).take();
                Err(error)
            }
        }
    }
}
pub async fn ensure_hub(context: &mut MainContext, cancel: &Cancellation) -> io::Result<()> {
    // Hub-originated wrappers must not probe/stop/restart their parent Hub.
    if environment_value(&context.environment, "MANY_AI_CLI") == Some("1") {
        return Ok(());
    }
    if let Some(port) = running_port(context).await? {
        set_port(context, port)?;
        if !restart_stale(context, port, cancel).await? {
            return Ok(());
        }
    }
    let config = context.config.snapshot().map_err(io::Error::other)?.config;
    let preferred = if context.paths.is_trial() {
        context.paths.port()
    } else {
        u16::try_from(config.hub.port).map_err(io::Error::other)?
    };
    let mut port = preferred;
    let count = if context.paths.is_trial() { 1 } else { 100 };
    for step in 0..count {
        let Some(candidate) = preferred.checked_add(step) else {
            break;
        };
        if let Ok(listener) =
            tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, candidate)).await
        {
            drop(listener);
            port = candidate;
            break;
        }
    }
    set_port(context, port)?;
    if cancel.is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Hub startup cancelled",
        ));
    }
    let mut command = Command::new(&context.executable);
    if context.paths.is_trial() {
        command.args([
            "--trial-root",
            &context.paths.root().to_string_lossy(),
            "--trial-port",
            &port.to_string(),
        ]);
    }
    command
        .args(["serve", "--port", &port.to_string()])
        .current_dir(&context.cwd)
        .env_clear();
    for entry in &context.environment {
        if let Some((key, value)) = entry.split_once('=') {
            command.env(key, value);
        }
    }
    command.env_remove(crate::process::wrapper_startup::STARTUP_JOB_ENV);
    command.env_remove("MANY_AI_CLI_INTERNAL_SPAWN_PROOF");
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Automatic background helpers stay hidden under the execution policy.
        command
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }
    #[cfg(not(windows))]
    {
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    }
    let mut startup = PendingHub(Some(command.spawn()?));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if cancel.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Hub startup cancelled",
            ));
        }
        if probe_hub_info(port, &config.token).await {
            return startup.release();
        }
        if startup.0.as_mut().unwrap().try_wait()?.is_some() {
            return Err(io::Error::other("Hub process exited before readiness"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(io::Error::other(
                "Hub did not become ready within 10 seconds",
            ));
        }
        tokio::select! { _ = cancel.cancelled() => {}, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
    }
}
async fn restart_stale(
    context: &MainContext,
    port: u16,
    cancel: &Cancellation,
) -> io::Result<bool> {
    let config = context.config.snapshot().map_err(io::Error::other)?.config;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(1))
        .build()
        .map_err(io::Error::other)?;
    let endpoint = |path: &str| -> io::Result<url::Url> {
        let mut url =
            url::Url::parse(&format!("http://127.0.0.1:{port}{path}")).map_err(io::Error::other)?;
        url.query_pairs_mut().append_pair("token", &config.token);
        Ok(url)
    };
    let Ok(response) = client.get(endpoint("/api/info")?).send().await else {
        return Ok(false);
    };
    if !response.status().is_success() {
        return Ok(false);
    }
    let Ok(info) = response.json::<serde_json::Value>().await else {
        return Ok(false);
    };
    if info["binary_stale"] != true {
        return Ok(false);
    }
    if info["active_sessions"].as_i64().unwrap_or(0) > 0 || !config.hub.stale_binary_auto_restart {
        eprintln!("Hub binary changed; automatic restart deferred");
        return Ok(false);
    }
    if client
        .post(endpoint("/api/shutdown")?)
        .send()
        .await
        .is_err()
    {
        return Ok(false);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if cancel.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Hub restart cancelled",
            ));
        }
        if !probe_hub_info(port, &config.token).await {
            return Ok(true);
        }
        tokio::select! { _ = cancel.cancelled() => {}, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
    }
    eprintln!("Hub stale restart did not finish; retaining current endpoint");
    Ok(false)
}
