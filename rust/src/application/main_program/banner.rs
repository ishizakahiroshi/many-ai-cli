//! Foreground Hub display. Authentication URLs must never pass through HubLogger.
use crate::config::Config;
use std::io::{self, IsTerminal, Write};

pub(super) fn write_startup(config: &Config, port: u16) {
    let stdout = io::stdout();
    // Autostarted Hubs have null stdin, including Unix helpers that inherit
    // stdout. Neither those helpers nor redirected output receive credentials
    // or foreground-window guidance.
    if !io::stdin().is_terminal() || !stdout.is_terminal() {
        return;
    }
    // Reuse the launcher's native console setup only for the interactive
    // display so Windows consoles render the Unicode logo as UTF-8.
    crate::launcher::configure_console_utf8();
    let banner = startup_banner(env!("MANY_AI_BUILD_VERSION"), port, &config.token);
    let mut stdout = stdout.lock();
    // A closed terminal must not panic or prevent the Hub from serving.
    let _ = stdout.write_all(banner.as_bytes());
    let _ = stdout.flush();
}

fn startup_banner(version: &str, port: u16, token: &str) -> String {
    // Use the connection launcher's logo shape as plain text without its
    // colors, Windows-only runtime label, or connection-specific instructions.
    // Plain text also respects terminals that disable ANSI colors.
    const LOGO: &str = concat!(
        "███╗   ███╗ █████╗ ███╗   ██╗██╗   ██╗       █████╗ ██╗\n",
        "████╗ ████║██╔══██╗████╗  ██║╚██╗ ██╔╝      ██╔══██╗██║\n",
        "██╔████╔██║███████║██╔██╗ ██║ ╚████╔╝ █████╗███████║██║\n",
        "██║╚██╔╝██║██╔══██║██║╚██╗██║  ╚██╔╝  ╚════╝██╔══██║██║\n",
        "██║ ╚═╝ ██║██║  ██║██║ ╚████║   ██║         ██║  ██║██║\n",
        "╚═╝     ╚═╝╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝         ╚═╝  ╚═╝╚═╝\n",
    );
    // Build metadata is not terminal markup. Escape control/non-ASCII input
    // before display; query encoding below separately protects the token URL.
    let version: String = version
        .trim()
        .chars()
        .flat_map(char::escape_default)
        .collect();
    let version = if version.is_empty() {
        "dev".into()
    } else if version == "dev" || version.starts_with('v') {
        version
    } else {
        format!("v{version}")
    };
    let base = format!("http://127.0.0.1:{port}/");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", token)
        .finish();
    format!(
        "\n{LOGO}\nmany-ai-cli Hub {version}\nRuntime: Rust / {} ({})\nGitHub: https://github.com/ishizakahiroshi/many-ai-cli\n\nWebUI: {base}\nOpen: {base}?{query}\n\nKeep this terminal open while using the Hub.\nClosing this window stops this foreground Hub and disconnects the Web UI.\nExisting wrapper sessions can wait for Hub recovery within their configured reconnect grace period.\nRestart the Hub promptly to reconnect.\nStop: press Ctrl+C here, or run many-ai-cli stop from another terminal.\n\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}
