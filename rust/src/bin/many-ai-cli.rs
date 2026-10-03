use many_ai_cli::{
    application::{launcher_program, runtime_context},
    cli::{self, Command},
    config::{ConfigError, ConfigStore},
    launcher,
    process::{self, Cancellation},
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
    let invocation = cli::parse(&args, &[])?;
    match invocation.command {
        Command::Version => println!("{}", env!("MANY_AI_BUILD_VERSION")),
        Command::Help => println!("{}", cli::USAGE),
        Command::Connect(args) => {
            let home = runtime_context::user_home().map_err(|error| error.to_string())?;
            let paths = runtime_context::runtime_paths(invocation.trial.as_ref(), &home)
                .map_err(|error| error.to_string())?;
            // The fixed main application loads configuration before connect
            // parsing, unlike the independent launcher's help-only path.
            let _config = ConfigStore::load_or_create(paths.clone(), || {
                process::random_token().map_err(ConfigError::from)
            })
            .map_err(|error| error.to_string())?;
            let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
            let connector = launcher::ConnectorConfig::new(paths, cwd).with_console();
            let cancel = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
                .map_err(|error| error.to_string())?;
            launcher_program::run_connect(&args, connector, &cancel, &launcher::open_browser)
                .await?;
        }
        Command::ShellInit => {
            let home = runtime_context::user_home().map_err(|error| error.to_string())?;
            let paths = runtime_context::runtime_paths(invocation.trial.as_ref(), &home)
                .map_err(|error| error.to_string())?;
            let config = ConfigStore::load_or_create(paths, || {
                process::random_token().map_err(ConfigError::from)
            })
            .map_err(|error| error.to_string())?;
            let snapshot = config.snapshot().map_err(|error| error.to_string())?;
            print!(
                "{}",
                many_ai_cli::wrapper::shell::init_script_for_config(&snapshot.config)
            );
        }
        _ => {
            return Err(
                "Rust migration candidate: this command is not integrated yet; see PROGRESS.md"
                    .into(),
            );
        }
    }
    Ok(())
}
