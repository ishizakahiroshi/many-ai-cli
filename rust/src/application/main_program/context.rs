use crate::{
    application::runtime_context,
    cli::TrialOptions,
    config::{ConfigError, ConfigStore, RuntimePaths},
    files::safe_fs::Dir,
    proto::core::TerminalSize,
};
use std::{io, path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct MainContext {
    pub config: Arc<ConfigStore>,
    pub paths: RuntimePaths,
    pub cwd: PathBuf,
    pub executable: PathBuf,
    /// Real installed application home is used only to validate disjoint roots
    /// when a spawned Rust wrapper re-enters main with its inherited trial flags.
    pub application_home: PathBuf,
    pub vendor_home: PathBuf,
    pub environment: Vec<String>,
    pub shell: String,
    pub terminal_size: TerminalSize,
}
impl MainContext {
    #[cfg(all(test, windows))]
    pub(crate) fn trial_environment_for_diagnostics(
        paths: &RuntimePaths,
        installed_home: &std::path::Path,
        environment: Vec<String>,
    ) -> io::Result<Vec<String>> {
        isolate_trial_environment(paths, installed_home, environment)
            .map(|(_, environment)| environment)
    }

    pub fn load(trial: Option<&TrialOptions>) -> io::Result<Self> {
        let application_home = runtime_context::user_home()?;
        let paths = runtime_context::runtime_paths(trial, &application_home)?;
        let cwd = std::env::current_dir()?;
        if paths.is_trial() {
            crate::profile::subscriptions::check_path(&paths, &cwd)?;
        }
        let environment: Vec<String> = std::env::vars_os()
            .filter_map(|(key, value)| {
                // Windows drive-current-directory pseudo entries cannot be passed
                // through the ordinary key=value child environment contract.
                let key = key.into_string().ok()?;
                if key.is_empty() || key.starts_with('=') {
                    return None;
                }
                Some(format!("{key}={}", value.to_string_lossy()))
            })
            .collect();
        let (vendor_home, environment) =
            isolate_trial_environment(&paths, &application_home, environment)?;
        let config = Arc::new(
            ConfigStore::load_or_create(paths.clone(), || {
                crate::process::random_token().map_err(ConfigError::from)
            })
            .map_err(io::Error::other)?,
        );
        let shell = environment_value(&environment, "SHELL")
            .or_else(|| environment_value(&environment, "COMSPEC"))
            .unwrap_or(if cfg!(windows) { "cmd.exe" } else { "/bin/sh" })
            .into();
        Ok(Self {
            config,
            paths,
            cwd,
            executable: std::env::current_exe()?,
            application_home,
            vendor_home,
            environment,
            shell,
            terminal_size: terminal_size(),
        })
    }
}
pub(super) fn environment_value<'a>(environment: &'a [String], key: &str) -> Option<&'a str> {
    environment.iter().rev().find_map(|entry| {
        entry
            .split_once('=')
            .filter(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    })
}
fn isolate_trial_environment(
    paths: &RuntimePaths,
    installed_home: &std::path::Path,
    mut environment: Vec<String>,
) -> io::Result<(PathBuf, Vec<String>)> {
    if !paths.is_trial() {
        return Ok((installed_home.into(), environment));
    }
    let root = Dir::open(paths.root())?;
    let home = root.child_dir("vendor-home", true)?;
    let home_path = home.path().to_path_buf();
    let temporary = root.child_dir("tmp", true)?.path().to_path_buf();
    let data = home.child_dir("appdata", true)?.path().to_path_buf();
    let local = home.child_dir("localappdata", true)?.path().to_path_buf();
    let config = home.child_dir("config", true)?.path().to_path_buf();
    let cache = home.child_dir("cache", true)?.path().to_path_buf();
    let profiles = [
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
        "GROK_HOME",
        "COPILOT_CONFIG_DIR",
        "CURSOR_CONFIG_DIR",
        "OPENCODE_CONFIG_DIR",
        "COMMAND_CODE_HOME",
    ];
    // Preserve only an explicitly confined profile selected by the owning Hub.
    for key in profiles {
        let confined = environment_value(&environment, key)
            .filter(|value| {
                crate::profile::subscriptions::check_path(paths, std::path::Path::new(value))
                    .is_ok()
            })
            .map(str::to_owned);
        let value = match confined {
            Some(value) => value,
            None => home
                .child_dir(&key.to_ascii_lowercase(), true)?
                .path()
                .to_string_lossy()
                .into_owned(),
        };
        environment.retain(|entry| {
            !entry
                .split_once('=')
                .is_some_and(|(name, _)| name.eq_ignore_ascii_case(key))
        });
        environment.push(format!("{key}={value}"));
    }
    let overrides = [
        ("HOME", &home_path),
        ("USERPROFILE", &home_path),
        ("APPDATA", &data),
        ("LOCALAPPDATA", &local),
        ("XDG_CONFIG_HOME", &config),
        ("XDG_CACHE_HOME", &cache),
        ("TMPDIR", &temporary),
        ("TEMP", &temporary),
        ("TMP", &temporary),
    ];
    environment.retain(|entry| {
        !entry.split_once('=').is_some_and(|(key, _)| {
            overrides
                .iter()
                .any(|(name, _)| key.eq_ignore_ascii_case(name))
        })
    });
    environment.extend(
        overrides
            .into_iter()
            .map(|(key, value)| format!("{key}={}", value.display())),
    );
    Ok((home_path, environment))
}
fn terminal_size() -> TerminalSize {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{
            CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE,
        };
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) != 0 {
            let cols = i64::from(info.srWindow.Right) - i64::from(info.srWindow.Left) + 1;
            let rows = i64::from(info.srWindow.Bottom) - i64::from(info.srWindow.Top) + 1;
            if cols > 0 && rows > 0 {
                return TerminalSize { cols, rows };
            }
        }
    }
    #[cfg(unix)]
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == 0
            && size.ws_col > 0
            && size.ws_row > 0
        {
            return TerminalSize {
                cols: i64::from(size.ws_col),
                rows: i64::from(size.ws_row),
            };
        }
    }
    // Go wrapper's fallback for redirected or unavailable terminal descriptors.
    TerminalSize { cols: 80, rows: 24 }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServeOptions {
    pub port: Option<u16>,
    pub open: bool,
    pub dev: bool,
    pub debug: bool,
}
impl ServeOptions {
    /// Go flag parsing stops at the first positional operand. Flags accept one
    /// or two leading dashes and the usual inline bool/int values.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut result = Self::default();
        let mut i = 0;
        while i < args.len() {
            let raw = &args[i];
            if raw == "--" || !raw.starts_with('-') || raw == "-" {
                break;
            }
            let flag = raw
                .strip_prefix("--")
                .or_else(|| raw.strip_prefix('-'))
                .unwrap();
            let (name, inline) = flag
                .split_once('=')
                .map_or((flag, None), |(a, b)| (a, Some(b)));
            match name {
                "open" | "dev" | "debug" => {
                    let value = match inline {
                        None => true,
                        Some("1" | "t" | "T" | "TRUE" | "true" | "True") => true,
                        Some("0" | "f" | "F" | "FALSE" | "false" | "False") => false,
                        _ => return Err(format!("invalid boolean for -{name}")),
                    };
                    match name {
                        "open" => result.open = value,
                        "dev" => result.dev = value,
                        _ => result.debug = value,
                    }
                }
                "port" => {
                    let value = match inline {
                        Some(value) => value,
                        None => {
                            i += 1;
                            args.get(i).ok_or("flag needs an argument: -port")?
                        }
                    };
                    let value: i64 = value.parse().map_err(|_| "invalid value for -port")?;
                    result.port = if value > 0 {
                        Some(u16::try_from(value).map_err(|_| "invalid Hub port")?)
                    } else {
                        None
                    };
                }
                "h" | "help" => return Err("serve help requested".into()),
                _ => return Err(format!("flag provided but not defined: -{name}")),
            }
            i += 1;
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serve_flags_follow_go_boolean_operand_and_last_port_semantics() {
        let args = [
            "--open=false",
            "-dev",
            "-port=49439",
            "-port",
            "0",
            "project",
            "-debug",
        ]
        .map(String::from);
        assert_eq!(
            ServeOptions::parse(&args).unwrap(),
            ServeOptions {
                open: false,
                dev: true,
                debug: false,
                port: None
            }
        );
        assert!(ServeOptions::parse(&["-port=65536".into()]).is_err());
        assert!(ServeOptions::parse(&["--open=yes".into()]).is_err());
    }
    #[test]
    fn trial_environment_replaces_ambient_vendor_paths_without_changing_owned_profile() {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49439, &root.path().join("installed")).unwrap();
        let owned = runtime.join("owned-codex");
        std::fs::create_dir(&owned).unwrap();
        let inherited = vec![
            format!("CODEX_HOME={}", owned.display()),
            format!(
                "CLAUDE_CONFIG_DIR={}",
                root.path().join("installed").display()
            ),
            "PATH=synthetic-tools".into(),
        ];
        let (home, env) =
            isolate_trial_environment(&paths, &root.path().join("installed"), inherited).unwrap();
        assert!(home.starts_with(paths.root()));
        assert_eq!(
            environment_value(&env, "CODEX_HOME"),
            Some(owned.to_str().unwrap())
        );
        for key in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "TMP",
            "TEMP",
            "TMPDIR",
            "CLAUDE_CONFIG_DIR",
        ] {
            let path = std::path::Path::new(environment_value(&env, key).unwrap());
            crate::profile::subscriptions::check_path(&paths, path).unwrap();
        }
    }
}
