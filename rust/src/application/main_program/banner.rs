//! Foreground Hub display. Authentication URLs must never pass through HubLogger.
use crate::config::Config;
use std::io::{self, IsTerminal, Write};

/// configured is the port the Hub was asked to listen on.
pub(super) fn write_startup(config: &Config, port: u16, configured: i64) {
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
    let banner = startup_banner(
        env!("MANY_AI_BUILD_VERSION"),
        port,
        &config.token,
        configured,
    );
    let mut stdout = stdout.lock();
    // A closed terminal must not panic or prevent the Hub from serving.
    let _ = stdout.write_all(banner.as_bytes());
    let _ = stdout.flush();
}

/// What to tell the user when the Hub could not have its configured port
/// (audit N24). The address of the other port is a different site to the
/// browser, and whoever holds the configured port answers the old one.
pub(super) fn moved_port_notice(configured: i64, port: u16) -> Option<String> {
    (configured != i64::from(port)).then(|| {
        format!(
            "Port {configured} (the configured Hub port) is held by another program, so this Hub listens on {port}. A tab or bookmark for 127.0.0.1:{configured} reaches that other program, not this Hub; open the dashboard from many-ai-cli or the address below."
        )
    })
}

fn startup_banner(version: &str, port: u16, token: &str, configured: i64) -> String {
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
    let moved = moved_port_notice(configured, port)
        .map(|notice| format!("Note: {notice}\n\n"))
        .unwrap_or_default();
    format!(
        "\n{LOGO}\nmany-ai-cli Hub {version}\nRuntime: Rust / {} ({})\nGitHub: https://github.com/ishizakahiroshi/many-ai-cli\n\n{moved}WebUI: {base}\nOpen: {base}?{query}\n\nKeep this terminal open while using the Hub.\nClosing this window stops this foreground Hub and disconnects the Web UI.\nExisting wrapper sessions can wait for Hub recovery within their configured reconnect grace period.\nRestart the Hub promptly to reconnect.\nStop: press Ctrl+C here, or run many-ai-cli stop from another terminal.\n\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_n24_a_hub_that_could_not_have_its_port_says_so() {
        // The configured port answers to someone else: the user must know not
        // to open the old address.
        let notice = moved_port_notice(47777, 47778).unwrap();
        assert!(notice.contains("47777") && notice.contains("47778"));
        let banner = startup_banner("v0.0.0", 47778, "synthetic", 47777);
        assert!(banner.contains(&format!("Note: {notice}\n")));
        assert!(banner.find("Note: ").unwrap() < banner.find("WebUI: ").unwrap());
        // On its own port nothing is added.
        assert_eq!(moved_port_notice(47777, 47777), None);
        assert!(!startup_banner("v0.0.0", 47777, "synthetic", 47777).contains("Note: "));
    }
}
