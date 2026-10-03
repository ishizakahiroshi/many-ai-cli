//! Executable composition for the standalone launcher and main `connect` path.
//! Side effects use the same launcher store/manager/connector as Hub Servers.
use crate::{
    cli,
    launcher::{self, ConnectionManager, ConnectorConfig, LauncherStore, UiServer},
    process::{Cancellation, OutputStream},
};
use std::io;

pub const LAUNCHER_USAGE: &str = "Usage of many-ai-cli-launcher:\n  -last\n    \tconnect using the last-used profile\n  -profile string\n    \tprofile name to connect (see ~/.many-ai-cli/launcher-profiles.yaml)\n  -ui\n    \topen the profile selection UI in the browser\n";
pub const CONNECT_USAGE: &str = "Usage of connect:\n  -last\n    \tconnect using the last-used profile\n  -profile string\n    \tprofile name to connect (see ~/.many-ai-cli/launcher-profiles.yaml)\n";

fn output(connector: &ConnectorConfig, stream: OutputStream, text: &str) {
    if let Some(sink) = &connector.output {
        sink(stream, text.as_bytes());
    }
}

pub async fn run_launcher(
    args: &[String],
    connector: ConnectorConfig,
    version: &str,
    cancel: &Cancellation,
    browser: &(dyn Fn(&str) -> io::Result<()> + Send + Sync),
) -> Result<(), String> {
    let sink = connector
        .output
        .clone()
        .unwrap_or_else(|| std::sync::Arc::new(|_, _| {}));
    let Some(invocation) = prepare_launcher(args, version, &sink)? else {
        return Ok(());
    };
    run_launcher_invocation(invocation, connector, cancel, browser).await
}

/// Parse and print before resolving a home or touching profile files, as Go does.
pub fn prepare_launcher(
    args: &[String],
    version: &str,
    sink: &launcher::OutputSink,
) -> Result<Option<cli::LauncherInvocation>, String> {
    launcher::configure_console_utf8();
    sink(
        OutputStream::Stdout,
        launcher::startup_banner(version).as_bytes(),
    );
    let invocation = cli::parse_launcher(args).inspect_err(|error| {
        sink(
            OutputStream::Stderr,
            format!("{error}\n{LAUNCHER_USAGE}").as_bytes(),
        );
    })?;
    if invocation.help {
        sink(OutputStream::Stderr, LAUNCHER_USAGE.as_bytes());
        return Ok(None);
    }
    Ok(Some(invocation))
}

pub async fn run_launcher_invocation(
    invocation: cli::LauncherInvocation,
    connector: ConnectorConfig,
    cancel: &Cancellation,
    browser: &(dyn Fn(&str) -> io::Result<()> + Send + Sync),
) -> Result<(), String> {
    let store = LauncherStore::open(connector.paths.clone())
        .map_err(|error| format!("load profiles: {error}"))?;
    let profiles = store
        .load_profiles()
        .map_err(|error| format!("load profiles: {error}"))?;
    profiles
        .validate()
        .map_err(|error| format!("invalid profiles: {error}"))?;
    if !invocation.open_ui && (!invocation.profile.is_empty() || invocation.use_last) {
        let profile = profiles.select(&invocation.profile, invocation.use_last)?;
        return launcher::connect_cli(&store, &connector, profile, cancel, browser).await;
    }

    let allow_browser = connector.paths.automatic_external_actions_allowed();
    let manager = ConnectionManager::new(store, connector.clone());
    let server =
        UiServer::new(manager.clone()).map_err(|error| format!("create ui server: {error}"))?;
    let mut handle = server
        .serve(cancel.clone())
        .await
        .map_err(|error| format!("start ui server: {error}"))?;
    output(
        &connector,
        OutputStream::Stdout,
        &format!("Opening connection selection page: {}\n", handle.url),
    );
    // Trial mode prints the isolated URL for explicit acceptance; it never opens
    // a browser or a remote provider automatically.
    if allow_browser && browser(&handle.url).is_err() {
        output(
            &connector,
            OutputStream::Stderr,
            "Could not open the browser; open the printed URL manually.\n",
        );
    }
    let result = handle.wait().await.map_err(|error| error.to_string());
    // Also clean up if an accept-loop error ended serving before its normal
    // shutdown branch. close_all is idempotent and owns only this manager.
    manager.close_all().await;
    result
}

pub async fn run_connect(
    args: &[String],
    connector: ConnectorConfig,
    cancel: &Cancellation,
    browser: &(dyn Fn(&str) -> io::Result<()> + Send + Sync),
) -> Result<(), String> {
    let invocation = cli::parse_connect(args).inspect_err(|error| {
        output(
            &connector,
            OutputStream::Stderr,
            &format!("{error}\n{CONNECT_USAGE}"),
        );
    })?;
    if invocation.help {
        output(&connector, OutputStream::Stderr, CONNECT_USAGE);
        return Ok(());
    }
    if invocation.profile.is_empty() && !invocation.use_last {
        return Err("connect requires --profile <name> or --last".into());
    }
    launcher::configure_console_utf8();
    let store = LauncherStore::open(connector.paths.clone())
        .map_err(|error| format!("load profiles: {error}"))?;
    let profiles = store
        .load_profiles()
        .map_err(|error| format!("load profiles: {error}"))?;
    profiles
        .validate()
        .map_err(|error| format!("invalid profiles: {error}"))?;
    let profile = profiles.select(&invocation.profile, invocation.use_last)?;
    launcher::connect_cli(&store, &connector, profile, cancel, browser).await
}
