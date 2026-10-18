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
//! `git_processes_are_planned_only_here` fails when production code builds a
//! Git `ProcessPlan` directly instead of coming through [`GitCommand::plan`].
use super::ProcessPlan;
use std::{collections::BTreeMap, ffi::OsString, fmt, path::PathBuf, time::Duration};

pub const OPTIONAL_LOCKS: &str = "GIT_OPTIONAL_LOCKS";

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
        let mut env = self.env;
        env.insert(OPTIONAL_LOCKS.into(), Some("0".into()));
        Ok(ProcessPlan {
            executable: self.executable,
            args: self.args,
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
        }
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
