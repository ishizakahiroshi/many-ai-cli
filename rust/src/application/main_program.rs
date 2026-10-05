//! Main serve/wrap composition. Newly implemented application input; recovered
//! modules alone do not establish worker or runtime acceptance.
mod bootstrap;
mod confirmations;
mod context;
mod dev_assets;
mod existing_hub;
mod instruction_observer;
mod logger;
pub mod model_cache;
pub mod model_catalog;
mod routines;
mod serve;
pub use bootstrap::{ensure_hub, running_port};
pub use context::{MainContext, ServeOptions};
pub use existing_hub::open_existing_hub;
pub use serve::{HubComposition, HubCompositionDependencies};
#[cfg(test)]
mod tests;

use crate::{
    process::Cancellation,
    wrapper::{
        entry::{self, WrapperContext},
        runtime::WrapperResult,
    },
};
use std::io;

/// Run the real native PTY/headless wrapper. Hub discovery/autostart belongs to
/// the broader provider command; an explicit wrap connects to its selected Hub.
pub async fn run_wrap(
    context: &MainContext,
    provider: &str,
    args: &[String],
    cancel: &Cancellation,
) -> io::Result<WrapperResult> {
    let snapshot = context.config.snapshot().map_err(io::Error::other)?;
    entry::run_cli(
        WrapperContext {
            config: &snapshot.config,
            paths: &context.paths,
            cwd: &context.cwd,
            executable: &context.executable,
            environment: &context.environment,
            shell: &context.shell,
            terminal_size: context.terminal_size,
            home_dir: &context.vendor_home,
        },
        provider,
        args,
        cancel,
    )
    .await
}
