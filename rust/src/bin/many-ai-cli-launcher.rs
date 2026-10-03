use many_ai_cli::{
    application::{launcher_program, runtime_context},
    cli, launcher,
    process::Cancellation,
};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (trial, args) = cli::split_trial_options(&args)?;
    let sink = launcher::console_sink();
    let Some(invocation) =
        launcher_program::prepare_launcher(args, env!("MANY_AI_BUILD_VERSION"), &sink)?
    else {
        return Ok(());
    };
    let home = runtime_context::user_home().map_err(|error| error.to_string())?;
    let paths =
        runtime_context::runtime_paths(trial.as_ref(), &home).map_err(|error| error.to_string())?;
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let connector = launcher::ConnectorConfig::new(paths, cwd).with_console();
    let cancel = Cancellation::default();
    let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
        .map_err(|error| error.to_string())?;
    launcher_program::run_launcher_invocation(
        invocation,
        connector,
        &cancel,
        &launcher::open_browser,
    )
    .await
}
