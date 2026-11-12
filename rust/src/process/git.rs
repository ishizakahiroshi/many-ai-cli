//! The one place a Hub-launched Git process is planned.
//!
//! Every plan carries `GIT_OPTIONAL_LOCKS=0`. Observing commands then cannot
//! take `.git/index.lock` for an opportunistic refresh, so one stopped at its
//! deadline, or with its session, cannot strand the lock and block the user's
//! own `git add` / `git commit`. Writing commands take their required locks
//! regardless, so the setting costs them nothing; observing is the default.
//!
//! Porcelain `git diff` against the work tree rewrites the index for stat-only
//! changes even with that setting (measured 2026-10-18, git 2.54.0). An
//! observing plan therefore refuses it; compare with the work tree through
//! plumbing `diff-index`, which never writes.
//!
//! A folder the user opens may hold a `.git` someone else wrote (an unpacked
//! archive, a shared folder). Unless the user asked for the action in the Git
//! tab ([`GitCommand::user_action`]), a plan therefore switches off the known
//! ways that repository's config can name a program. Command-scope settings
//! outrank the repository's own config.
//!
//! - Fixed settings, put on the plan here (audit U13): the fsmonitor hook,
//!   hooks in a hooks directory, `gpg.program` for `log`, and diff drivers.
//! - Programs the config names by a name of its own choosing (audit N09, N10):
//!   config-defined hooks (`hook.<name>.command`) and filter drivers
//!   (`filter.<name>.clean` / `smudge` / `process`). Git has no setting that
//!   turns these off as a class, so the one place every owned process runs
//!   (`process::run_observed`) first asks Git for the names
//!   ([`named_programs_query`]) and overrides each ([`without_named_programs`]).
//!   If the names cannot be read, the command does not run.
//!
//! What this does not cover: a kind of program-naming setting that is not
//! listed here, a name added between the lookup and the command (a process
//! that can write `.git/config` while the Hub looks), and a submodule's own
//! config. A Hub Git never fetches on its own (audit N11): a transport is a
//! program the repository's config names as well.
//!
//! `git_processes_are_planned_only_here` fails when production code builds a
//! Git `ProcessPlan` directly instead of coming through [`GitCommand::plan`].
use super::ProcessPlan;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fmt,
    path::PathBuf,
    sync::LazyLock,
    time::Duration,
};

pub const OPTIONAL_LOCKS: &str = "GIT_OPTIONAL_LOCKS";
pub const CONFIG_COUNT: &str = "GIT_CONFIG_COUNT";

/// `core.hooksPath` for a Hub Git: a path below this program's own file. A
/// file has no entries, so Git finds no hook there (neither `.git/hooks` nor a
/// directory the repository's config names), and nothing has to be created,
/// kept empty or removed.
pub fn no_hooks_path() -> &'static OsStr {
    static PATH: LazyLock<OsString> = LazyLock::new(|| {
        std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from(if cfg!(windows) { "NUL" } else { "/dev/null" }))
            .join("no-hooks")
            .into_os_string()
    });
    &PATH
}

/// Settings forced on a Hub Git that is not the user's own action.
fn forced_config() -> [(&'static str, &'static OsStr); 3] {
    [
        ("core.fsmonitor", OsStr::new("false")),
        ("core.hooksPath", no_hooks_path()),
        ("log.showSignature", OsStr::new("false")),
    ]
}

/// Appends the forced settings after any the caller, or the Hub's own
/// environment, already passes through `GIT_CONFIG_COUNT`, so those still apply.
fn force_config(env: &mut BTreeMap<OsString, Option<OsString>>) {
    append_config(env, forced_config());
}

/// Turns the repository's Git hooks off for the programs a process starts:
/// its Git commands look for hooks in `no_hooks_path`, after the settings the
/// process would otherwise get through `GIT_CONFIG_COUNT`. A bounded child
/// without a sandbox gets this, so `git commit` cannot run a hook it edited
/// (U49).
pub fn disable_hooks(env: &mut BTreeMap<OsString, Option<OsString>>) {
    append_config(env, [("core.hooksPath", no_hooks_path())]);
}

fn config_count(env: &BTreeMap<OsString, Option<OsString>>) -> usize {
    let passed = match env.get(OsStr::new(CONFIG_COUNT)) {
        Some(value) => value.clone(),
        None => std::env::var_os(CONFIG_COUNT),
    };
    passed
        .and_then(|value| value.to_str().and_then(|v| v.trim().parse::<usize>().ok()))
        .unwrap_or(0)
}

fn append_config<K: Into<OsString>, V: Into<OsString>>(
    env: &mut BTreeMap<OsString, Option<OsString>>,
    forced: impl IntoIterator<Item = (K, V)>,
) {
    let mut index = config_count(env);
    for (key, value) in forced {
        env.insert(format!("GIT_CONFIG_KEY_{index}").into(), Some(key.into()));
        env.insert(
            format!("GIT_CONFIG_VALUE_{index}").into(),
            Some(value.into()),
        );
        index += 1;
    }
    env.insert(CONFIG_COUNT.into(), Some(index.to_string().into()));
}

/// Whether the plan carries the forced settings, that is: it came from
/// [`GitCommand::plan`] and is not the user's own action.
fn unattended(plan: &ProcessPlan) -> bool {
    let value = |key: String| plan.env.get(OsStr::new(&key)).cloned().flatten();
    let forced = |name: &str, expected: &OsStr| {
        (0..config_count(&plan.env)).any(|index| {
            value(format!("GIT_CONFIG_KEY_{index}")).is_some_and(|key| key == name)
                && value(format!("GIT_CONFIG_VALUE_{index}")).is_some_and(|v| v == expected)
        })
    };
    plan.env.get(OsStr::new(OPTIONAL_LOCKS)) == Some(&Some("0".into()))
        && forced("core.fsmonitor", OsStr::new("false"))
        && forced("core.hooksPath", no_hooks_path())
}

/// Subcommands that only read refs, objects, ignore rules or config. They run
/// no hook and no filter, so they skip the lookup of names; most are display
/// lookups on a 250 ms budget. Anything not listed gets the lookup.
const PROGRAM_FREE: &[&str] = &[
    "check-ignore",
    "for-each-ref",
    "merge-base",
    "rev-list",
    "rev-parse",
];

fn program_free(args: &[OsString], index: usize, name: &str) -> bool {
    PROGRAM_FREE.contains(&name)
        || (name == "remote"
            && args
                .get(index + 1)
                .is_some_and(|operation| operation.as_os_str() == OsStr::new("get-url")))
}

/// The config sections whose subsection is a name the repository chooses and
/// whose settings name a program, with the settings that switch that name off.
/// An empty value clears a command (and the list of a hook's events).
const NAMED_PROGRAMS: &[(&str, &[(&str, &str)])] = &[
    ("hook", &[("enabled", "false"), ("event", "")]),
    (
        "filter",
        &[
            ("clean", ""),
            ("smudge", ""),
            ("process", ""),
            ("required", "false"),
        ],
    ),
];
const NAMES_PATTERN: &str = r"^(hook|filter)\.";
const NAMES_CAP: usize = 1024 * 1024;
const NAMES_TIMEOUT: Duration = Duration::from_secs(5);

/// For an unattended Hub Git that can run a hook or a filter: the Git that
/// lists every `hook.*` and `filter.*` setting name in effect for the same
/// repository (it reads config only, and runs none of those programs).
/// `None` when the plan needs no lookup.
pub(super) fn named_programs_query(plan: &ProcessPlan) -> Option<ProcessPlan> {
    if !unattended(plan) {
        return None;
    }
    let (index, name) = subcommand(&plan.args)?;
    if program_free(&plan.args, index, &name) {
        return None;
    }
    let mut args = plan.args[..index].to_vec();
    args.extend(
        [
            "config",
            "--null",
            "--name-only",
            "--get-regexp",
            NAMES_PATTERN,
        ]
        .map(OsString::from),
    );
    Some(ProcessPlan {
        executable: plan.executable.clone(),
        args,
        cwd: plan.cwd.clone(),
        env: plan.env.clone(),
        stdin: Vec::new(),
        timeout: if plan.timeout.is_zero() {
            NAMES_TIMEOUT
        } else {
            plan.timeout.min(NAMES_TIMEOUT)
        },
        output_cap: NAMES_CAP,
        pipe_drain_timeout: plan.pipe_drain_timeout,
    })
}

/// `plan` with every hook and filter driver the lookup listed switched off.
/// `Err` is what the command reports instead of running: the lookup's own
/// result when Git could not answer (a missing directory or a broken config
/// stops the command the same way), or an error when the list is unusable.
pub(super) fn without_named_programs(
    plan: &ProcessPlan,
    listed: super::ProcessOutput,
    spent: Duration,
) -> Result<ProcessPlan, std::io::Result<super::ProcessOutput>> {
    let unanswered = |outcome| {
        Err(Ok(super::ProcessOutput {
            stdout: Vec::new(),
            stderr: listed.stderr.clone(),
            stdout_truncated: false,
            stderr_truncated: listed.stderr_truncated,
            pipes_forced_closed: false,
            outcome,
        }))
    };
    let unusable = || {
        Err(Err(std::io::Error::other(
            "git config names of hooks and filters could not be read",
        )))
    };
    let names: &[u8] = match listed.outcome {
        super::ExitOutcome::Exited { code: Some(0), .. } => &listed.stdout,
        // Exit 1: no such setting.
        super::ExitOutcome::Exited { code: Some(1), .. } => &[],
        ref other => return unanswered(other.clone()),
    };
    if listed.stdout_truncated || listed.pipes_forced_closed {
        return unusable();
    }
    let Ok(names) = std::str::from_utf8(names) else {
        return unusable();
    };
    let mut overrides = std::collections::BTreeSet::new();
    for key in names.split('\0') {
        for (section, settings) in NAMED_PROGRAMS {
            // `<section>.<name>.<setting>`: the name may itself hold dots.
            let Some(name) = key
                .strip_prefix(section)
                .and_then(|rest| rest.strip_prefix('.'))
                .and_then(|rest| rest.rsplit_once('.'))
                .map(|(name, _)| name)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            for (setting, value) in *settings {
                overrides.insert((format!("{section}.{name}.{setting}"), *value));
            }
        }
    }
    let mut plan = plan.clone();
    if !plan.timeout.is_zero() {
        // Zero would mean "no deadline", so a used-up budget is a timeout.
        plan.timeout = plan.timeout.saturating_sub(spent);
        if plan.timeout.is_zero() {
            return unanswered(super::ExitOutcome::TimedOut);
        }
    }
    append_config(&mut plan.env, overrides);
    Ok(plan)
}

/// Subcommands that can show a diff. They get `--no-ext-diff --no-textconv`
/// right after the subcommand, so a `diff.<driver>.command` or
/// `diff.<driver>.textconv` the repository names never runs.
const DIFFING: &[&str] = &[
    "diff",
    "diff-files",
    "diff-index",
    "diff-tree",
    "log",
    "show",
];

fn no_external_diff(args: &mut Vec<OsString>) {
    if let Some((index, name)) = subcommand(args)
        && DIFFING.contains(&name.as_str())
    {
        args.splice(
            index + 1..index + 1,
            [
                OsString::from("--no-ext-diff"),
                OsString::from("--no-textconv"),
            ],
        );
    }
}

/// Whether a command may lock or rewrite the index, work tree, refs or config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Observe,
    Write,
}

/// Subcommands that may write. Anything else observes. A missing entry only
/// loses the leftover-lock follow-up for that command: every plan still skips
/// optional locks.
const WRITING: &[&str] = &[
    "add",
    "am",
    "apply",
    "branch",
    "checkout",
    "cherry-pick",
    "clean",
    "clone",
    "commit",
    "config",
    "fetch",
    "gc",
    "init",
    "maintenance",
    "merge",
    "mv",
    "notes",
    "pull",
    "push",
    "read-tree",
    "rebase",
    "reset",
    "restore",
    "revert",
    "rm",
    "stash",
    "submodule",
    "switch",
    "tag",
    "update-index",
    "update-ref",
    "worktree",
    "write-tree",
];

/// Global options before the subcommand that take their value as the next
/// argument (`-C <path>`, `-c <name>=<value>`, ...).
const GLOBAL_WITH_VALUE: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
    "--super-prefix",
    "--exec-path",
];

fn subcommand(args: &[OsString]) -> Option<(usize, String)> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].to_string_lossy();
        if !arg.starts_with('-') {
            return Some((index, arg.into_owned()));
        }
        index += if GLOBAL_WITH_VALUE.contains(&arg.as_ref()) {
            2
        } else {
            1
        };
    }
    None
}

/// The access a Git argv implies, judged from its subcommand.
pub fn access(args: &[OsString]) -> Access {
    match subcommand(args) {
        Some((_, name)) if WRITING.contains(&name.as_str()) => Access::Write,
        _ => Access::Observe,
    }
}

/// `git diff` with fewer than two revisions compares the index or the work
/// tree. Only `--cached` / `--staged` (index against a tree) and `--no-index`
/// leave the index alone; a range (`a..b`, `a...b`) names two revisions.
fn rewrites_index(args: &[OsString]) -> bool {
    let Some((index, name)) = subcommand(args) else {
        return false;
    };
    if name != "diff" {
        return false;
    }
    let mut revisions = 0;
    for arg in &args[index + 1..] {
        let arg = arg.to_string_lossy();
        match arg.as_ref() {
            "--" => break,
            "--cached" | "--staged" | "--no-index" => return false,
            option if option.starts_with('-') => {}
            range if range.contains("..") => revisions += 2,
            _ => revisions += 1,
        }
    }
    revisions < 2
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused(pub &'static str);
impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Refused {}

/// A Git process before it becomes a [`ProcessPlan`]. The fields mean the same
/// as the plan's; `executable` is the resolved Git.
#[derive(Clone)]
pub struct GitCommand {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: BTreeMap<OsString, Option<OsString>>,
    pub stdin: Vec<u8>,
    pub timeout: Duration,
    pub output_cap: usize,
    pub pipe_drain_timeout: Duration,
    /// The user's own action from the Git tab (commit, fetch, pull, push). It
    /// runs with the repository's settings and hooks, as `git` itself would,
    /// so the user's pre-commit checks still run. Everything else is `false`.
    pub user_action: bool,
}

impl GitCommand {
    pub fn access(&self) -> Access {
        access(&self.args)
    }

    pub fn plan(self) -> Result<ProcessPlan, Refused> {
        if self.access() == Access::Observe && rewrites_index(&self.args) {
            return Err(Refused(
                "porcelain git diff against the work tree rewrites the index; use diff-index",
            ));
        }
        let mut args = self.args;
        let mut env = self.env;
        env.insert(OPTIONAL_LOCKS.into(), Some("0".into()));
        if !self.user_action {
            force_config(&mut env);
            no_external_diff(&mut args);
        }
        Ok(ProcessPlan {
            executable: self.executable,
            args,
            cwd: self.cwd,
            env,
            stdin: self.stdin,
            timeout: self.timeout,
            output_cap: self.output_cap,
            pipe_drain_timeout: self.pipe_drain_timeout,
        })
    }
}

/// The bare command name callers resolve through PATH.
pub const COMMAND: &str = "git";

/// A writing Git the Hub stopped before it finished (deadline or cancel). It
/// may have left `.git/index.lock`; the Hub looks (`files::git_lock`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stopped {
    /// The `-C` directory, or the process directory without one.
    pub directory: PathBuf,
    pub started: std::time::SystemTime,
    pub stopped: std::time::SystemTime,
}

type StopSink = std::sync::Arc<dyn Fn(Stopped) + Send + Sync>;
static STOP_SINK: std::sync::Mutex<Option<StopSink>> = std::sync::Mutex::new(None);

/// Receive every [`Stopped`] writing Git from now on (the Hub sets one).
pub fn on_stopped(sink: StopSink) {
    *STOP_SINK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sink);
}

/// The [`Stopped`] a finished plan reports, if it is a writing Git from this
/// entry that did not finish on its own.
pub fn stopped(
    plan: &ProcessPlan,
    outcome: &super::ExitOutcome,
    started: std::time::SystemTime,
) -> Option<Stopped> {
    if !matches!(
        outcome,
        super::ExitOutcome::TimedOut | super::ExitOutcome::Cancelled
    ) {
        return None;
    }
    let ours = plan.env.get(std::ffi::OsStr::new(OPTIONAL_LOCKS)) == Some(&Some("0".into()));
    let git = plan
        .executable
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case(COMMAND));
    if !ours || !git || access(&plan.args) != Access::Write {
        return None;
    }
    let mut directory = plan.cwd.clone();
    let mut index = 0;
    while index < plan.args.len() {
        let arg = plan.args[index].to_string_lossy();
        if !arg.starts_with('-') {
            break;
        }
        if arg == "-C"
            && let Some(next) = plan.args.get(index + 1)
        {
            let next = PathBuf::from(next);
            directory = if next.is_absolute() {
                next
            } else {
                directory.join(next)
            };
        }
        index += if GLOBAL_WITH_VALUE.contains(&arg.as_ref()) {
            2
        } else {
            1
        };
    }
    Some(Stopped {
        directory,
        started,
        stopped: std::time::SystemTime::now(),
    })
}

/// Tell the Hub's sink about a stopped writing Git (no sink: nothing happens).
pub(super) fn report(
    plan: &ProcessPlan,
    outcome: &super::ExitOutcome,
    started: std::time::SystemTime,
) {
    let Some(stopped) = stopped(plan, outcome, started) else {
        return;
    };
    let sink = STOP_SINK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(sink) = sink {
        sink(stopped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn command(args: &[&str]) -> GitCommand {
        GitCommand {
            executable: "git".into(),
            args: argv(args),
            cwd: ".".into(),
            env: BTreeMap::from([("SYNTHETIC".into(), Some("kept".into()))]),
            stdin: vec![],
            timeout: Duration::from_secs(1),
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_millis(10),
            user_action: false,
        }
    }

    fn forced(plan: &ProcessPlan) -> BTreeMap<OsString, OsString> {
        let value = |key: String| {
            plan.env
                .get(OsStr::new(&key))
                .cloned()
                .flatten()
                .unwrap_or_else(|| panic!("{key} is missing"))
        };
        let count: usize = value(CONFIG_COUNT.into())
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        (0..count)
            .map(|index| {
                (
                    value(format!("GIT_CONFIG_KEY_{index}")),
                    value(format!("GIT_CONFIG_VALUE_{index}")),
                )
            })
            .collect()
    }

    #[test]
    fn audit_u49_disabled_hooks_follow_the_settings_a_child_inherits() {
        let mut env = BTreeMap::from([
            (OsString::from(CONFIG_COUNT), Some(OsString::from("1"))),
            ("GIT_CONFIG_KEY_0".into(), Some("core.quotePath".into())),
            ("GIT_CONFIG_VALUE_0".into(), Some("false".into())),
        ]);
        disable_hooks(&mut env);
        assert_eq!(env[OsStr::new(CONFIG_COUNT)], Some("2".into()));
        assert_eq!(
            env[OsStr::new("GIT_CONFIG_KEY_1")],
            Some("core.hooksPath".into())
        );
        assert_eq!(
            env[OsStr::new("GIT_CONFIG_VALUE_1")].as_deref(),
            Some(no_hooks_path())
        );
        // A count the child does not get starts the entry at 0.
        let mut env = BTreeMap::from([(OsString::from(CONFIG_COUNT), None)]);
        disable_hooks(&mut env);
        assert_eq!(
            env[OsStr::new("GIT_CONFIG_KEY_0")],
            Some("core.hooksPath".into())
        );
        assert_eq!(env[OsStr::new(CONFIG_COUNT)], Some("1".into()));
    }

    #[test]
    fn hub_git_runs_no_program_the_repository_names() {
        // Audit U13. The caller's own GIT_CONFIG_COUNT entries stay in front.
        let mut diff = command(&["-C", "repo", "diff-index", "--numstat", "HEAD", "--"]);
        diff.env.extend([
            (CONFIG_COUNT.into(), Some("1".into())),
            ("GIT_CONFIG_KEY_0".into(), Some("core.quotePath".into())),
            ("GIT_CONFIG_VALUE_0".into(), Some("false".into())),
        ]);
        let plan = diff.plan().unwrap();
        let config = forced(&plan);
        assert_eq!(config.len(), 4);
        assert_eq!(config[OsStr::new("core.quotePath")], "false");
        assert_eq!(config[OsStr::new("core.fsmonitor")], "false");
        assert_eq!(config[OsStr::new("log.showSignature")], "false");
        assert_eq!(config[OsStr::new("core.hooksPath")], no_hooks_path());
        assert_eq!(
            plan.args,
            argv(&[
                "-C",
                "repo",
                "diff-index",
                "--no-ext-diff",
                "--no-textconv",
                "--numstat",
                "HEAD",
                "--"
            ])
        );
        // A removed count starts the forced entries at 0; non-diff argv stays.
        let mut status = command(&["-C", "repo", "-c", "a=b", "status", "--porcelain"]);
        status.env.insert(CONFIG_COUNT.into(), None);
        let plan = status.plan().unwrap();
        assert_eq!(forced(&plan).len(), 3);
        assert_eq!(
            plan.args,
            argv(&["-C", "repo", "-c", "a=b", "status", "--porcelain"])
        );
        for args in [
            &["log", "-1"][..],
            &["show", "-s", "HEAD"],
            &["diff", "a..b"],
        ] {
            let plan = command(args).plan().unwrap();
            assert_eq!(
                &plan.args[1..3],
                &argv(&["--no-ext-diff", "--no-textconv"])[..]
            );
        }
        // No hook can exist below a regular file.
        assert!(
            std::path::Path::new(no_hooks_path())
                .parent()
                .unwrap()
                .is_file()
        );
    }

    #[test]
    fn the_users_own_git_tab_action_keeps_the_repositorys_settings() {
        let mut commit = command(&["-C", "repo", "commit", "-m", "x"]);
        commit.user_action = true;
        let plan = commit.plan().unwrap();
        assert!(
            !plan
                .env
                .keys()
                .any(|key| key.to_string_lossy().starts_with("GIT_CONFIG"))
        );
        assert_eq!(plan.args, argv(&["-C", "repo", "commit", "-m", "x"]));
        let mut diff = command(&["diff", "--cached", "--quiet"]);
        diff.user_action = true;
        assert_eq!(
            diff.plan().unwrap().args,
            argv(&["diff", "--cached", "--quiet"])
        );
    }

    fn listed(code: i32, stdout: &[u8]) -> crate::process::ProcessOutput {
        crate::process::ProcessOutput {
            stdout: stdout.to_vec(),
            stderr: b"synthetic stderr".to_vec(),
            stdout_truncated: false,
            stderr_truncated: false,
            pipes_forced_closed: false,
            outcome: crate::process::ExitOutcome::Exited {
                code: Some(code),
                signal: None,
            },
        }
    }

    #[test]
    fn only_an_unattended_git_that_can_run_a_program_asks_for_the_names() {
        // Audit N09, N10. The lookup keeps the global options, so it reads the
        // same repository with the same settings.
        let plan = command(&[
            "-C",
            "repo",
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain",
        ])
        .plan()
        .unwrap();
        let query = named_programs_query(&plan).unwrap();
        assert_eq!(
            query.args,
            argv(&[
                "-C",
                "repo",
                "-c",
                "core.quotePath=false",
                "config",
                "--null",
                "--name-only",
                "--get-regexp",
                r"^(hook|filter)\."
            ])
        );
        assert_eq!(query.env, plan.env);
        assert_eq!(query.cwd, plan.cwd);
        assert_eq!(query.executable, plan.executable);
        assert_eq!(query.timeout, Duration::from_secs(1));
        assert!(query.stdin.is_empty());
        for asks in [
            &["add", "-A"][..],
            &["read-tree", "HEAD"],
            &["write-tree"],
            &["diff-index", "-M", "--numstat", "HEAD"],
            &["worktree", "add", "-b", "x", "path", "HEAD"],
            &["log", "-1"],
            &["symbolic-ref", "-q", "HEAD"],
            &["remote", "update"],
            &["ls-files", "-m"],
        ] {
            assert!(
                named_programs_query(&command(asks).plan().unwrap()).is_some(),
                "{asks:?}"
            );
        }
        for free in [
            &["rev-parse", "--show-toplevel"][..],
            &["-C", "repo", "merge-base", "--is-ancestor", "a", "b"],
            &["remote", "get-url", "origin"],
            &["check-ignore", "--stdin", "-z"],
            &["--version"],
            &[],
        ] {
            assert!(
                named_programs_query(&command(free).plan().unwrap()).is_none(),
                "{free:?}"
            );
        }
        // The user's own action runs the repository's hooks and filters.
        let mut commit = command(&["commit", "-m", "x"]);
        commit.user_action = true;
        assert!(named_programs_query(&commit.plan().unwrap()).is_none());
        // A plan that did not come from this entry is not this entry's to change.
        let mut foreign = plan.clone();
        foreign.env.remove(OsStr::new(OPTIONAL_LOCKS));
        assert!(named_programs_query(&foreign).is_none());
        // A command without a deadline still bounds its lookup.
        let mut open = plan;
        open.timeout = Duration::ZERO;
        assert_eq!(named_programs_query(&open).unwrap().timeout, NAMES_TIMEOUT);
    }

    #[test]
    fn listed_hooks_and_filters_are_switched_off_after_the_forced_settings() {
        // Audit N09, N10. Names keep their case, spaces and dots.
        let plan = command(&["-C", "repo", "add", "-A"]).plan().unwrap();
        let names = b"hook.audit.command\0hook.audit.event\0hook.Two Words.v2.event\0\
            filter.x.clean\0filter.lfs.process\0hook.jobs\0filter.\0core.hookspath\0";
        let off = without_named_programs(&plan, listed(0, names), Duration::from_millis(100))
            .ok()
            .unwrap();
        let config = forced(&off);
        assert_eq!(config.len(), 3 + 2 * 2 + 2 * 4);
        assert_eq!(config[OsStr::new("core.hooksPath")], no_hooks_path());
        for hook in ["audit", "Two Words.v2"] {
            assert_eq!(config[OsStr::new(&format!("hook.{hook}.enabled"))], "false");
            assert_eq!(config[OsStr::new(&format!("hook.{hook}.event"))], "");
        }
        for filter in ["x", "lfs"] {
            for setting in ["clean", "smudge", "process"] {
                assert_eq!(
                    config[OsStr::new(&format!("filter.{filter}.{setting}"))],
                    ""
                );
            }
            assert_eq!(
                config[OsStr::new(&format!("filter.{filter}.required"))],
                "false"
            );
        }
        assert_eq!(off.args, plan.args);
        assert_eq!(off.timeout, Duration::from_millis(900));
        // No such setting (exit 1): the plan is unchanged.
        let none = without_named_programs(&plan, listed(1, b""), Duration::ZERO)
            .ok()
            .unwrap();
        assert_eq!(none.env, plan.env);
        // No deadline stays no deadline.
        let mut open = plan.clone();
        open.timeout = Duration::ZERO;
        let open = without_named_programs(&open, listed(1, b""), Duration::from_secs(9))
            .ok()
            .unwrap();
        assert!(open.timeout.is_zero());
    }

    #[test]
    fn without_the_names_the_command_does_not_run() {
        use crate::process::ExitOutcome;
        let plan = command(&["-C", "absent", "status", "--porcelain"])
            .plan()
            .unwrap();
        let instead = |listed, spent| {
            without_named_programs(&plan, listed, spent)
                .err()
                .expect("the command must not be planned")
        };
        // Git could not answer: its own exit and message stand for the command's.
        let failed = instead(listed(128, b"ignored"), Duration::ZERO).unwrap();
        assert_eq!(
            failed.outcome,
            ExitOutcome::Exited {
                code: Some(128),
                signal: None
            }
        );
        assert_eq!(failed.stderr, b"synthetic stderr");
        assert!(failed.stdout.is_empty());
        for outcome in [ExitOutcome::TimedOut, ExitOutcome::Cancelled] {
            let mut stopped = listed(0, b"hook.audit.event\0");
            stopped.outcome = outcome.clone();
            assert_eq!(instead(stopped, Duration::ZERO).unwrap().outcome, outcome);
        }
        // The lookup used the whole budget.
        assert_eq!(
            instead(listed(1, b""), Duration::from_secs(1))
                .unwrap()
                .outcome,
            ExitOutcome::TimedOut
        );
        // A list that is cut short or unreadable names nothing reliably.
        let mut truncated = listed(0, b"hook.audit.event\0");
        truncated.stdout_truncated = true;
        assert!(instead(truncated, Duration::ZERO).is_err());
        let mut forced_closed = listed(0, b"hook.audit.event\0");
        forced_closed.pipes_forced_closed = true;
        assert!(instead(forced_closed, Duration::ZERO).is_err());
        assert!(instead(listed(0, b"hook.\xff.event\0"), Duration::ZERO).is_err());
    }

    #[test]
    fn every_plan_skips_optional_locks_and_keeps_the_rest() {
        for args in [
            &["-C", "repo", "status", "--porcelain"][..],
            &["-C", "repo", "-c", "core.quotePath=false", "add", "-A"],
            &["--version"],
        ] {
            let plan = command(args).plan().unwrap();
            assert_eq!(
                plan.env.get(OsString::from(OPTIONAL_LOCKS).as_os_str()),
                Some(&Some("0".into()))
            );
            assert_eq!(
                plan.env.get(OsString::from("SYNTHETIC").as_os_str()),
                Some(&Some("kept".into()))
            );
            assert_eq!(plan.args, argv(args));
            assert_eq!(plan.executable, PathBuf::from("git"));
            assert_eq!(plan.timeout, Duration::from_secs(1));
        }
        // A caller cannot turn the setting back on.
        let mut enabled = command(&["status"]);
        enabled.env.insert(OPTIONAL_LOCKS.into(), Some("1".into()));
        assert_eq!(
            enabled
                .plan()
                .unwrap()
                .env
                .get(OsString::from(OPTIONAL_LOCKS).as_os_str()),
            Some(&Some("0".into()))
        );
    }

    #[test]
    fn access_follows_the_subcommand_after_global_options() {
        let cases = [
            (&["-C", "repo", "status", "--short"][..], Access::Observe),
            (&["-C", "add", "rev-parse", "HEAD"], Access::Observe),
            (
                &["-c", "core.quotePath=false", "diff-index", "HEAD"],
                Access::Observe,
            ),
            (&["--no-optional-locks", "log", "-1"], Access::Observe),
            (&["--git-dir", "commit", "show"], Access::Observe),
            (&["-C", "repo", "add", "-A"], Access::Write),
            (
                &["-C", "repo", "-c", "a=b", "commit", "-m", "x"],
                Access::Write,
            ),
            (
                &["-C", "repo", "worktree", "add", "-b", "x", "path"],
                Access::Write,
            ),
            (&["read-tree", "HEAD"], Access::Write),
            (&["pull", "--ff-only"], Access::Write),
            (&["--version"], Access::Observe),
            (&[], Access::Observe),
        ];
        for (args, expected) in cases {
            assert_eq!(access(&argv(args)), expected, "{args:?}");
        }
    }

    #[test]
    fn observing_refuses_porcelain_diff_that_reads_the_work_tree() {
        for refused in [
            &["-C", "repo", "diff", "--numstat", "HEAD", "--"][..],
            &["diff", "HEAD", "--", "file"],
            &["diff", "--stat", "HEAD"],
            &["diff"],
            &["diff", "--", "HEAD", "other"],
        ] {
            assert_eq!(
                command(refused)
                    .plan()
                    .err()
                    .map(|e| e.0.contains("diff-index")),
                Some(true),
                "{refused:?}"
            );
        }
        for allowed in [
            &["diff", "--name-only", "base..HEAD"][..],
            &["diff", "a...b"],
            &["diff", "--no-color", "tree-a", "tree-b", "--", "file"],
            &["diff", "--cached", "HEAD"],
            &["diff", "--staged"],
            &["diff", "--no-index", "a", "b"],
            &["diff-index", "-M", "--numstat", "HEAD"],
            &["status", "--porcelain"],
        ] {
            assert!(command(allowed).plan().is_ok(), "{allowed:?}");
        }
    }

    #[test]
    fn only_a_writing_git_from_this_entry_that_was_stopped_is_reported() {
        let started = std::time::SystemTime::now();
        let write = command(&["-C", "repo dir", "-c", "a=b", "commit", "-a"])
            .plan()
            .unwrap();
        let reported = stopped(&write, &crate::process::ExitOutcome::TimedOut, started).unwrap();
        assert_eq!(reported.directory, PathBuf::from(".").join("repo dir"));
        assert_eq!(reported.started, started);
        assert!(stopped(&write, &crate::process::ExitOutcome::Cancelled, started).is_some());
        let finished = crate::process::ExitOutcome::Exited {
            code: Some(1),
            signal: None,
        };
        assert!(stopped(&write, &finished, started).is_none());
        // Reads are harmless to stop; plans not built here are not ours.
        let read = command(&["status"]).plan().unwrap();
        assert!(stopped(&read, &crate::process::ExitOutcome::TimedOut, started).is_none());
        let mut foreign = write.clone();
        foreign
            .env
            .remove(OsString::from(OPTIONAL_LOCKS).as_os_str());
        assert!(stopped(&foreign, &crate::process::ExitOutcome::TimedOut, started).is_none());
        let mut other = write;
        other.executable = "ssh".into();
        assert!(stopped(&other, &crate::process::ExitOutcome::TimedOut, started).is_none());
        // Without -C the process directory is the repository.
        let mut plain = command(&["add", "-A"]).plan().unwrap();
        plain.cwd = "work".into();
        assert_eq!(
            stopped(&plain, &crate::process::ExitOutcome::TimedOut, started)
                .unwrap()
                .directory,
            PathBuf::from("work")
        );
    }

    // ---- the single-entry check ----

    /// Inline `#[cfg(test)] mod name {` blocks are cut from the scanned text;
    /// test-only files are skipped by name.
    fn production(text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let mut out = Vec::new();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index].trim();
            if line == "#[cfg(test)]" {
                let next = lines.get(index + 1).map(|v| v.trim()).unwrap_or("");
                if next.starts_with("mod ") && next.ends_with('{') {
                    // Skip to the brace that closes this module.
                    let mut depth = 0i32;
                    let mut cursor = index + 1;
                    while cursor < lines.len() {
                        depth += lines[cursor].matches('{').count() as i32;
                        depth -= lines[cursor].matches('}').count() as i32;
                        cursor += 1;
                        if depth <= 0 {
                            break;
                        }
                    }
                    index = cursor;
                    continue;
                }
            }
            out.push(lines[index]);
            index += 1;
        }
        out.join("\n")
    }

    fn mentions_git(window: &str) -> bool {
        let lower = window.to_ascii_lowercase();
        lower.contains("git_executable")
            || lower.contains("\"git\"")
            || lower.contains("\"git.exe\"")
            || lower.contains("git_optional_locks")
            || lower.contains("worktreegit")
            || lower
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(|word| word == "git")
    }

    /// Direct Git plans in one file's production text.
    fn direct_git_plans(text: &str) -> Vec<usize> {
        let lines: Vec<&str> = text.lines().collect();
        let mut found = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if line.contains("look_path(\"git\")") {
                found.push(index + 1);
                continue;
            }
            if !line.contains("ProcessPlan {") || line.contains("struct ProcessPlan") {
                continue;
            }
            // The literal plus the twelve lines that prepare it.
            let start = index.saturating_sub(12);
            let mut end = index;
            let mut depth = 0i32;
            while end < lines.len() - 1 {
                depth += lines[end].matches('{').count() as i32;
                depth -= lines[end].matches('}').count() as i32;
                if depth <= 0 {
                    break;
                }
                end += 1;
            }
            let window = lines[start..=end].join("\n");
            if mentions_git(&window) {
                found.push(index + 1);
            }
        }
        found
    }

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    fn test_only(relative: &Path) -> bool {
        let name = relative.file_name().unwrap().to_string_lossy();
        name == "tests.rs"
            || name.ends_with("_tests.rs")
            || name.starts_with("tests_")
            || relative.components().any(|c| c.as_os_str() == "tests")
            // Oracle builders for the headless contract tests (`#[cfg(test)] mod`).
            || name == "windows_contracts.rs"
    }

    #[test]
    fn git_processes_are_planned_only_here() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let entry = Path::new("process").join("git.rs");
        let mut offenders = Vec::new();
        for file in files {
            let relative = file.strip_prefix(&src).unwrap().to_path_buf();
            if relative == entry || test_only(&relative) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            for line in direct_git_plans(&production(&text)) {
                offenders.push(format!("{}:{line}", relative.display()));
            }
        }
        assert!(
            offenders.is_empty(),
            "Git processes must be planned through process::git::GitCommand: {offenders:?}"
        );
    }

    #[test]
    fn the_check_finds_direct_plans_and_ignores_tests_and_other_programs() {
        let direct = "fn a() {\n    let plan = ProcessPlan {\n        executable: self.service.git_executable.clone(),\n        args,\n    };\n}\n";
        assert_eq!(direct_git_plans(direct), vec![2]);
        let nearby = "fn b() {\n    env.insert(\"GIT_OPTIONAL_LOCKS\".into(), None);\n    Ok(ProcessPlan {\n        executable,\n    })\n}\n";
        assert_eq!(direct_git_plans(nearby), vec![3]);
        assert_eq!(
            direct_git_plans("let e = resolver.look_path(\"git\")?;"),
            vec![1]
        );
        let other = "fn c() {\n    let plan = ProcessPlan {\n        executable: \"ssh.exe\".into(),\n        args,\n    };\n}\n";
        assert!(direct_git_plans(other).is_empty());
        let in_tests = "fn d() {}\n#[cfg(test)]\nmod tests {\n    fn e() {\n        let p = ProcessPlan {\n            executable: \"git\".into(),\n        };\n    }\n}\nfn after() {}\n";
        let kept = production(in_tests);
        assert!(
            kept.contains("fn d()") && kept.contains("fn after()") && !kept.contains("ProcessPlan")
        );
        assert!(direct_git_plans(&kept).is_empty());
    }
}
