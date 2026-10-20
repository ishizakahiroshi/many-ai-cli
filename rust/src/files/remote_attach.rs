//! Shell sessions often `ssh` into another machine and run an AI CLI there. A
//! path saved on the Hub machine is unreadable from that side, so when the user
//! picks a destination for the session, the attachment is also written to the
//! destination over ssh and the destination's path is injected instead. The
//! same destination is used the other way: a file the AI there reports can be
//! fetched by its path and handed to the browser as a download.
//!
//! - Only an ssh destination typed by the user is accepted (an alias from
//!   ~/.ssh/config, user@host or ssh://user@host:port). Anything that ssh could
//!   read as an option is rejected before a process starts.
//! - BatchMode only: a destination that would ask for a password fails with
//!   ssh's own message instead of waiting for input nobody can type.
//! - Files written to the destination are reclaimed by the next transfer
//!   (older than REMOTE_RETENTION_DAYS), so a killed Hub never leaves them
//!   forever (the rule at the top of internal/doctor/residue.go). Fetching
//!   writes nothing on either side; the Hub keeps the bytes only for the
//!   response.
use super::{Result, err};
use crate::{
    hub::http::Response,
    process::{Cancellation, ExitOutcome, ProcessOutput, ProcessPlan, SpawnOptions},
};
use std::{path::Path, time::Duration};

/// Relative to the destination user's $HOME.
pub const REMOTE_DIR: &str = ".many-ai-cli/remote-attachments";
/// Files at least this many whole days old are removed. find's `-mtime +N`
/// matches an age of N+1 days or more, hence `- 1` in the script.
pub const REMOTE_RETENTION_DAYS: u32 = 7;
/// A larger file is refused on the destination before any of it is sent. The
/// whole file passes through Hub and browser memory, and scp stays the way to
/// move anything bigger.
pub const FETCH_MAX: usize = 50 * 1024 * 1024;
const CONNECT_TIMEOUT_SECS: u32 = 15;
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);
/// Login shells may print to stdout before the command runs; the saved path is
/// taken only from after this marker.
const PATH_MARKER: &str = "many-ai-cli-remote-path:";
/// Printed right before the fetched file for the same reason.
const FILE_MARKER: &str = "many-ai-cli-remote-file:";
/// Exit codes of fetch_script that are not ssh's own failures.
const FETCH_NOT_A_FILE: i32 = 3;
const FETCH_TOO_LARGE: i32 = 4;

pub fn validate_target(target: &str) -> Result<&str> {
    let host = target.strip_prefix("ssh://").unwrap_or(target);
    let valid = !host.is_empty()
        && target.len() <= 255
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-@:[]".contains(&b));
    if valid {
        Ok(target)
    } else {
        Err(err(
            400,
            "invalid_remote_target",
            "remote target must be an ssh destination such as myserver or user@host",
        ))
    }
}

/// Where a file to fetch lives on the destination. A relative path is refused:
/// the Hub cannot know which directory the shell was in over there.
#[derive(Debug, PartialEq)]
pub enum RemotePath<'a> {
    Absolute(&'a str),
    /// The part after `~/`.
    Home(&'a str),
}

pub fn validate_remote_path(path: &str) -> Result<RemotePath<'_>> {
    let parsed = match path.strip_prefix("~/") {
        Some(rest) => RemotePath::Home(rest),
        None if path.starts_with('/') => RemotePath::Absolute(path),
        None => {
            return Err(err(
                400,
                "invalid_remote_path",
                "remote path must start with / or ~/",
            ));
        }
    };
    let rest = match parsed {
        RemotePath::Absolute(p) | RemotePath::Home(p) => p,
    };
    let valid = !rest.is_empty()
        && rest != "/"
        && !rest.ends_with('/')
        && path.len() <= 4096
        && !path.chars().any(char::is_control);
    if valid {
        Ok(parsed)
    } else {
        Err(err(
            400,
            "invalid_remote_path",
            "remote path must name a file on the destination",
        ))
    }
}

/// One line of POSIX sh: csh-family login shells reject a newline inside quotes,
/// and fish treats backslashes in single quotes as escapes, so neither is used.
/// `$1` is the file name.
pub fn remote_script() -> String {
    format!(
        "umask 077; d=\"$HOME/{REMOTE_DIR}\"; mkdir -p \"$d\" || exit 1; \
         find \"$d\" -type f -mtime +{} -exec rm -f {{}} + 2>/dev/null; \
         cat > \"$d/$1\" || exit 1; printf '%s%s' '{PATH_MARKER}' \"$d/$1\"",
        REMOTE_RETENTION_DAYS - 1
    )
}

/// Same constraints as remote_script. `$1` is the path and `$2` is `home` when
/// the path is relative to $HOME.
pub fn fetch_script() -> String {
    format!(
        "f=\"$1\"; [ \"$2\" = home ] && f=\"$HOME/$1\"; \
         [ -f \"$f\" ] || {{ echo \"not a file: $f\" >&2; exit {FETCH_NOT_A_FILE}; }}; \
         s=$(wc -c < \"$f\") || exit 1; \
         [ $s -le {FETCH_MAX} ] || {{ echo \"larger than {} MB: $f\" >&2; exit {FETCH_TOO_LARGE}; }}; \
         printf '%s' '{FILE_MARKER}'; exec cat -- \"$f\"",
        FETCH_MAX / (1024 * 1024)
    )
}

/// ssh joins everything after the destination with spaces and hands it to the
/// destination's login shell, so the script and its arguments are quoted for it.
fn ssh_command(target: &str, script: &str, argv0: &str, script_args: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = ["-T", "-o", "BatchMode=yes", "-o"]
        .map(String::from)
        .to_vec();
    args.push(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"));
    args.extend([
        "--".into(),
        target.into(),
        "sh".into(),
        "-c".into(),
        crate::launcher::shell_quote(script),
        argv0.into(),
    ]);
    args.extend(script_args.iter().map(|a| crate::launcher::shell_quote(a)));
    args
}

pub fn ssh_args(target: &str, name: &str) -> Vec<String> {
    ssh_command(target, &remote_script(), "many-ai-cli-attach", &[name])
}

pub fn fetch_args(target: &str, path: &RemotePath) -> Vec<String> {
    let (path, base) = match path {
        RemotePath::Absolute(p) => (*p, "absolute"),
        RemotePath::Home(p) => (*p, "home"),
    };
    ssh_command(target, &fetch_script(), "many-ai-cli-fetch", &[path, base])
}

pub fn parse_remote_path(stdout: &[u8], name: &str) -> Option<String> {
    let text = std::str::from_utf8(stdout).ok()?;
    let path = text.rsplit_once(PATH_MARKER)?.1.trim();
    let valid = path.starts_with('/')
        && path.len() < 4096
        && path.ends_with(&format!("/{name}"))
        && !path.chars().any(char::is_control);
    valid.then(|| path.to_owned())
}

/// The file is everything after the first marker. Output from a login shell
/// can only come before it, and the file itself may contain the marker.
pub fn file_after_marker(mut stdout: Vec<u8>) -> Option<Vec<u8>> {
    let marker = FILE_MARKER.as_bytes();
    let at = stdout.windows(marker.len()).position(|w| w == marker)?;
    stdout.drain(..at + marker.len());
    Some(stdout)
}

/// ssh reports the reason (host key, permission denied, unknown host) on its
/// last stderr line.
pub fn failure_detail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("");
    line.chars().filter(|c| !c.is_control()).take(200).collect()
}

fn ssh_failed(stderr: &[u8], code: &str) -> Response {
    let detail = failure_detail(stderr);
    err(
        502,
        code,
        if detail.is_empty() {
            "ssh failed"
        } else {
            &detail
        },
    )
}

async fn run_ssh(
    ssh: &Path,
    cwd: &Path,
    args: Vec<String>,
    stdin: Option<&[u8]>,
    output_cap: usize,
    failed_code: &str,
) -> Result<ProcessOutput> {
    let plan = ProcessPlan {
        executable: ssh.into(),
        args: args.into_iter().map(Into::into).collect(),
        cwd: cwd.into(),
        env: Default::default(),
        stdin: stdin.unwrap_or_default().to_vec(),
        timeout: TRANSFER_TIMEOUT,
        output_cap,
        pipe_drain_timeout: Duration::from_secs(2),
    };
    crate::process::run_capped_with_options(
        &plan,
        &Cancellation::default(),
        SpawnOptions {
            no_window: true,
            stdin_null: stdin.is_none(),
            ..SpawnOptions::default()
        },
    )
    .await
    .map_err(|e| err(502, failed_code, &format!("ssh could not be started: {e}")))
}

pub async fn upload(
    ssh: &Path,
    cwd: &Path,
    target: &str,
    name: &str,
    data: &[u8],
) -> Result<String> {
    let output = run_ssh(
        ssh,
        cwd,
        ssh_args(target, name),
        Some(data),
        1024 * 1024,
        "remote_upload_failed",
    )
    .await?;
    match output.outcome {
        ExitOutcome::Exited { code: Some(0), .. } => parse_remote_path(&output.stdout, name)
            .ok_or_else(|| {
                err(
                    502,
                    "remote_upload_failed",
                    "ssh finished but did not report the saved path",
                )
            }),
        ExitOutcome::TimedOut => Err(err(
            504,
            "remote_upload_timeout",
            "ssh did not finish in time",
        )),
        _ => Err(ssh_failed(&output.stderr, "remote_upload_failed")),
    }
}

pub async fn download(ssh: &Path, cwd: &Path, target: &str, path: &str) -> Result<Vec<u8>> {
    let path = validate_remote_path(path)?;
    // Room for whatever a login shell prints before the marker.
    let cap = FETCH_MAX + 64 * 1024;
    let output = run_ssh(
        ssh,
        cwd,
        fetch_args(target, &path),
        None,
        cap,
        "remote_fetch_failed",
    )
    .await?;
    let too_large = || {
        err(
            413,
            "remote_file_too_large",
            &format!(
                "the file is larger than {} MB; copy it with scp instead",
                FETCH_MAX / (1024 * 1024)
            ),
        )
    };
    match output.outcome {
        ExitOutcome::Exited { code: Some(0), .. } if output.stdout_truncated => Err(too_large()),
        ExitOutcome::Exited { code: Some(0), .. } => {
            file_after_marker(output.stdout).ok_or_else(|| {
                err(
                    502,
                    "remote_fetch_failed",
                    "ssh finished but the file did not arrive",
                )
            })
        }
        ExitOutcome::Exited {
            code: Some(FETCH_NOT_A_FILE),
            ..
        } => Err(err(
            404,
            "remote_file_not_found",
            &failure_detail(&output.stderr),
        )),
        ExitOutcome::Exited {
            code: Some(FETCH_TOO_LARGE),
            ..
        } => Err(too_large()),
        ExitOutcome::TimedOut => Err(err(
            504,
            "remote_fetch_timeout",
            "ssh did not finish in time",
        )),
        _ => Err(ssh_failed(&output.stderr, "remote_fetch_failed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_that_ssh_could_read_as_options_are_rejected() {
        for ok in [
            "myserver",
            "user@example.com",
            "ssh://user@example.com:2222",
            "user@[2001:db8::1]",
        ] {
            assert!(validate_target(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "-oProxyCommand=calc",
            "ssh://-oProxyCommand=calc",
            "host name",
            "host;rm",
            "host$(id)",
            "host\nnext",
            "ssh://",
        ] {
            assert!(validate_target(bad).is_err(), "{bad:?}");
        }
        assert!(validate_target(&"a".repeat(256)).is_err());
    }

    #[test]
    fn destination_follows_the_option_terminator_and_the_command_is_quoted() {
        let args = ssh_args("myserver", "20261015_000000001_shot.png");
        let dashdash = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[dashdash + 1], "myserver");
        assert!(args[..dashdash].contains(&"BatchMode=yes".to_owned()));
        assert_eq!(args[dashdash + 2..dashdash + 4], ["sh", "-c"]);
        assert!(args[dashdash + 4].starts_with('\'') && args[dashdash + 4].ends_with('\''));
        assert_eq!(args.last().unwrap(), "'20261015_000000001_shot.png'");
    }

    #[test]
    fn remote_script_stays_on_one_line_without_backslashes() {
        let script = remote_script();
        assert!(!script.contains('\n'));
        assert!(!script.contains('\\'));
        assert!(script.contains(REMOTE_DIR));
        assert!(script.contains(&format!("-mtime +{}", REMOTE_RETENTION_DAYS - 1)));
    }

    #[test]
    fn saved_path_is_read_after_the_marker_only() {
        let name = "20261015_000000001_shot.png";
        let path = format!("/srv/u/{REMOTE_DIR}/{name}");
        assert_eq!(
            parse_remote_path(format!("{PATH_MARKER}{path}").as_bytes(), name),
            Some(path.clone())
        );
        // A login banner printed without a newline must not become part of the path.
        assert_eq!(
            parse_remote_path(format!("welcome{PATH_MARKER}{path}").as_bytes(), name),
            Some(path.clone())
        );
        assert_eq!(parse_remote_path(path.as_bytes(), name), None);
        assert_eq!(
            parse_remote_path(format!("{PATH_MARKER}relative/{name}").as_bytes(), name),
            None
        );
        assert_eq!(
            parse_remote_path(format!("{PATH_MARKER}/srv/u/other.png").as_bytes(), name),
            None
        );
    }

    #[test]
    fn failure_detail_is_the_last_non_empty_stderr_line() {
        assert_eq!(
            failure_detail(b"Warning: x\r\nuser@host: Permission denied (publickey).\r\n\r\n"),
            "user@host: Permission denied (publickey)."
        );
        assert_eq!(failure_detail(b""), "");
        assert_eq!(failure_detail(&[b'x'; 500]).len(), 200);
    }

    #[tokio::test]
    async fn an_unstartable_ssh_is_reported_instead_of_a_local_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing-ssh-for-test.exe");
        let error = upload(&missing, dir.path(), "myserver", "a.txt", b"synthetic")
            .await
            .unwrap_err();
        assert_eq!(error.status, 502);
        assert!(String::from_utf8_lossy(&error.body).contains("remote_upload_failed"));
    }

    #[test]
    fn only_absolute_and_home_paths_naming_a_file_can_be_fetched() {
        assert_eq!(
            validate_remote_path("/srv/u/kobo/out.png").unwrap(),
            RemotePath::Absolute("/srv/u/kobo/out.png")
        );
        assert_eq!(
            validate_remote_path("~/kobo/out file.png").unwrap(),
            RemotePath::Home("kobo/out file.png")
        );
        for bad in [
            "",
            "out.png",
            "kobo/out.png",
            "-rf",
            "~",
            "~/",
            "~user/out.png",
            "/",
            "/srv/u/kobo/",
            "/srv/u/a\nb.png",
        ] {
            assert!(validate_remote_path(bad).is_err(), "{bad:?}");
        }
        assert!(validate_remote_path(&format!("/{}", "a".repeat(4096))).is_err());
    }

    #[test]
    fn fetch_quotes_the_path_after_the_destination() {
        let args = fetch_args("myserver", &RemotePath::Home("it's here.png"));
        let dashdash = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[dashdash + 1], "myserver");
        assert_eq!(args[dashdash + 2..dashdash + 4], ["sh", "-c"]);
        assert_eq!(args[dashdash + 5], "many-ai-cli-fetch");
        assert_eq!(args[dashdash + 6], "'it'\\''s here.png'");
        assert_eq!(args[dashdash + 7], "'home'");
        let absolute = fetch_args("myserver", &RemotePath::Absolute("/tmp/a.png"));
        assert_eq!(absolute.last().unwrap(), "'absolute'");
    }

    #[test]
    fn fetch_script_stays_on_one_line_and_refuses_large_files_remotely() {
        let script = fetch_script();
        assert!(!script.contains('\n'));
        assert!(!script.contains('\\'));
        assert!(script.contains(&format!("-le {FETCH_MAX} ]")));
        assert!(script.contains(&format!("exit {FETCH_TOO_LARGE}")));
        assert!(script.contains(&format!("exit {FETCH_NOT_A_FILE}")));
        assert!(script.ends_with("exec cat -- \"$f\""));
    }

    #[test]
    fn fetched_file_starts_after_the_first_marker() {
        let file = format!("png{FILE_MARKER}inside").into_bytes();
        let mut stdout = b"Welcome to the server\n".to_vec();
        stdout.extend(FILE_MARKER.as_bytes());
        stdout.extend(&file);
        assert_eq!(file_after_marker(stdout), Some(file));
        assert_eq!(
            file_after_marker(FILE_MARKER.as_bytes().to_vec()),
            Some(vec![])
        );
        assert_eq!(file_after_marker(b"no marker".to_vec()), None);
    }

    #[tokio::test]
    async fn an_unstartable_ssh_fails_the_fetch_and_a_relative_path_never_starts_it() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing-ssh-for-test.exe");
        let error = download(&missing, dir.path(), "myserver", "/tmp/a.png")
            .await
            .unwrap_err();
        assert_eq!(error.status, 502);
        assert!(String::from_utf8_lossy(&error.body).contains("remote_fetch_failed"));
        let error = download(&missing, dir.path(), "myserver", "a.png")
            .await
            .unwrap_err();
        assert_eq!(error.status, 400);
    }
}
