use many_ai_cli::cli::{self, Command};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = cli::parse(&args, &[]).and_then(|invocation| match invocation.command {
        Command::Version => {
            println!("{}", env!("MANY_AI_BUILD_VERSION"));
            Ok(())
        }
        Command::Help => {
            println!("{}", cli::USAGE);
            Ok(())
        }
        _ => Err(
            "Rust migration candidate: this command is not integrated yet; see PROGRESS.md".into(),
        ),
    });
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
