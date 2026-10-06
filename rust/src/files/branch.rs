//! Session Git observations, matching the fixed Go branch/project helpers.
//! Each lookup owns one 250 ms budget, including its fallback command.
use super::FilesService;
use crate::{
    process::{
        Cancellation, ExitOutcome, ProcessOutput, ProcessPlan,
        execpath::{NativeFs, Platform, Resolver},
    },
    proto::core::CoreFuture,
};
use std::{
    ffi::OsString,
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const LOOKUP_TIMEOUT: Duration = Duration::from_millis(250);
const OUTPUT_CAP: usize = 16 * 1024 * 1024;

/// `Some("")` is a definitive absence of a project; `None` must be retried.
pub trait GitBranchSource: Send + Sync {
    fn branch<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, String>;
    fn changes<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, (i64, i64, i64)>;
    fn project<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, Option<String>>;
}

impl FilesService {
    /// Go creates each display lookup from the command name and current PATH.
    /// Keep that behavior separate from the existing Git API's selected binary;
    /// explicit FilesService implementations remain useful for isolated runners.
    pub fn branch_reader(&self) -> std::sync::Arc<dyn GitBranchSource> {
        let mut reader = Self::new(self.hub_cwd.clone(), self.paths.clone());
        reader.git_executable = PathBuf::from("git");
        reader.git_environment = self.git_environment.clone();
        std::sync::Arc::new(reader)
    }
}

impl GitBranchSource for FilesService {
    fn branch<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, String> {
        Box::pin(async move {
            if cwd.trim().is_empty() {
                return String::new();
            }
            let git = Lookup::new(self);
            let Ok(output) = git.run(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).await else {
                return String::new();
            };
            let branch = output.trim();
            if branch != "HEAD" {
                return branch.into();
            }
            match git.run(cwd, &["rev-parse", "--short", "HEAD"]).await {
                Ok(hash) if !hash.trim().is_empty() => format!("detached:{}", hash.trim()),
                _ => String::new(),
            }
        })
    }

    fn changes<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, (i64, i64, i64)> {
        Box::pin(async move {
            if cwd.trim().is_empty() {
                return (0, 0, 0);
            }
            let git = Lookup::new(self);
            let Ok(status) = git.run(cwd, &["status", "--porcelain"]).await else {
                return (0, 0, 0);
            };
            let files = status
                .split('\n')
                .filter(|line| !line.trim().is_empty())
                .count() as i64;
            let Ok(numstat) = git.run(cwd, &["diff", "--numstat", "HEAD"]).await else {
                return (files, 0, 0);
            };
            let (added, deleted) = change_lines(&numstat);
            (files, added, deleted)
        })
    }

    fn project<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, Option<String>> {
        Box::pin(async move {
            if cwd.trim().is_empty() {
                return Some(String::new());
            }
            let git = Lookup::new(self);
            let output = match git
                .run(
                    cwd,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )
                .await
            {
                Ok(output) => output,
                Err(_) => match git.run(cwd, &["rev-parse", "--git-common-dir"]).await {
                    Ok(output) => output,
                    Err(error) => {
                        return (!git.expired() && error.answered()).then(String::new);
                    }
                },
            };
            project_path(cwd, &output)
        })
    }
}

struct Lookup<'a> {
    service: &'a FilesService,
    until: Instant,
}

#[derive(Debug)]
enum LookupError {
    MissingExecutable,
    Start,
    Expired,
    Output(ProcessOutput),
}

impl LookupError {
    fn answered(&self) -> bool {
        match self {
            Self::MissingExecutable => true,
            Self::Output(output) => {
                matches!(output.outcome, ExitOutcome::Exited { .. })
                    && !output.stdout_truncated
                    && !output.stderr_truncated
                    && !output.pipes_forced_closed
            }
            Self::Start | Self::Expired => false,
        }
    }
}

impl<'a> Lookup<'a> {
    fn new(service: &'a FilesService) -> Self {
        Self {
            service,
            until: Instant::now() + LOOKUP_TIMEOUT,
        }
    }

    fn expired(&self) -> bool {
        Instant::now() >= self.until
    }

    fn plan(&self, cwd: &str, args: &[&str]) -> Result<ProcessPlan, LookupError> {
        if self.expired() {
            return Err(LookupError::Expired);
        }
        let executable = lookup_executable(self.service)?;
        let timeout = self.until.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err(LookupError::Expired);
        }
        let mut argv = vec![OsString::from("-C"), OsString::from(cwd)];
        argv.extend(args.iter().map(OsString::from));
        Ok(ProcessPlan {
            executable,
            args: argv,
            // Go passes only -C; a nonexistent target must reach Git, rather
            // than becoming an indistinguishable pre-exec chdir failure.
            cwd: self.service.hub_cwd.clone(),
            env: self.service.git_environment.clone(),
            stdin: vec![],
            timeout,
            output_cap: OUTPUT_CAP,
            pipe_drain_timeout: LOOKUP_TIMEOUT,
        })
    }

    async fn run(&self, cwd: &str, args: &[&str]) -> Result<String, LookupError> {
        let plan = self.plan(cwd, args)?;
        let output = crate::process::run_capped(&plan, &Cancellation::default())
            .await
            .map_err(|_| LookupError::Start)?;
        if output.stdout_truncated || output.stderr_truncated || output.pipes_forced_closed {
            return Err(LookupError::Output(output));
        }
        match output.outcome {
            ExitOutcome::Exited { code: Some(0), .. } => {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }
            _ => Err(LookupError::Output(output)),
        }
    }
}

/// Only PATH lookup absence corresponds to Go exec.ErrNotFound. A path that
/// existed during lookup can still fail to spawn (including raw ENOENT from a
/// missing interpreter or working directory); that failure stays unresolved.
fn lookup_executable(service: &FilesService) -> Result<PathBuf, LookupError> {
    let executable = &service.git_executable;
    let Some(name) = executable.to_str() else {
        return Ok(executable.clone());
    };
    if name.contains('/') || (cfg!(windows) && name.contains(['\\', ':'])) {
        return Ok(executable.clone());
    }
    let mut environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (key, value) in &service.git_environment {
        environment.retain(|(existing, _)| {
            if cfg!(windows) {
                !existing
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&key.to_string_lossy())
            } else {
                existing != key
            }
        });
        if let Some(value) = value {
            environment.push((key.clone(), value.clone()));
        }
    }
    let environment: Vec<String> = environment
        .into_iter()
        .map(|(key, value)| format!("{}={}", key.to_string_lossy(), value.to_string_lossy()))
        .collect();
    Resolver::new(
        Platform::native(),
        &environment,
        &service.hub_cwd,
        &NativeFs,
    )
    .look_path(name)
    .map(PathBuf::from)
    .map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            LookupError::MissingExecutable
        } else {
            LookupError::Start
        }
    })
}

fn change_lines(output: &str) -> (i64, i64) {
    let (mut added, mut deleted) = (0_i64, 0_i64);
    for line in output.split('\n') {
        let mut parts = line.trim().splitn(3, '\t');
        let Some(add) = parts.next() else { continue };
        let Some(del) = parts.next() else { continue };
        // Go Atoi accepts signs, rejects overflow, and leaves each invalid
        // field at zero. Its signed additions wrap rather than panic.
        if let Ok(value) = add.parse::<i64>() {
            added = added.wrapping_add(value);
        }
        if let Ok(value) = del.parse::<i64>() {
            deleted = deleted.wrapping_add(value);
        }
    }
    (added, deleted)
}

fn project_path(cwd: &str, output: &str) -> Option<String> {
    let common = output.trim();
    if common.is_empty() {
        return None;
    }
    let mut common = lexical_clean(Path::new(common));
    if !common.is_absolute() {
        // Go filepath.Join concatenates before cleaning. In particular a
        // Windows rooted, drive-less result must not replace the cwd prefix.
        common = lexical_clean(Path::new(&format!(
            "{cwd}{}{}",
            std::path::MAIN_SEPARATOR,
            common.to_string_lossy()
        )));
    }
    if common.file_name().is_some_and(|name| name == ".git") {
        common = lexical_clean(common.parent().unwrap_or_else(|| Path::new(".")));
    }
    Some(common.to_string_lossy().into_owned())
}

fn lexical_clean(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if result.file_name().is_some_and(|name| name != "..") {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            component => result.push(component.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, fs};

    struct Fixture {
        owned: tempfile::TempDir,
        service: FilesService,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let owned = tempfile::tempdir().unwrap();
            let root = owned.path().join("synthetic repo 日本語");
            let runtime = owned.path().join("runtime");
            fs::create_dir(&root).unwrap();
            fs::create_dir(&runtime).unwrap();
            let paths = crate::config::RuntimePaths::trial(
                &runtime,
                49223,
                &owned.path().join("installed"),
            )
            .unwrap();
            let mut service = FilesService::new(owned.path().into(), paths);
            let empty = owned.path().join("empty-config");
            let hooks = owned.path().join("empty-hooks");
            fs::write(&empty, b"").unwrap();
            fs::create_dir(&hooks).unwrap();
            service.git_environment = BTreeMap::from([
                ("GIT_CONFIG_GLOBAL".into(), Some(empty.into_os_string())),
                ("GIT_CONFIG_NOSYSTEM".into(), Some("1".into())),
                ("GIT_CONFIG_COUNT".into(), Some("3".into())),
                ("GIT_CONFIG_KEY_0".into(), Some("core.hooksPath".into())),
                ("GIT_CONFIG_VALUE_0".into(), Some(hooks.into_os_string())),
                ("GIT_CONFIG_KEY_1".into(), Some("commit.gpgSign".into())),
                ("GIT_CONFIG_VALUE_1".into(), Some("false".into())),
                ("GIT_CONFIG_KEY_2".into(), Some("core.autocrlf".into())),
                ("GIT_CONFIG_VALUE_2".into(), Some("false".into())),
                ("GIT_AUTHOR_NAME".into(), Some("Synthetic Author".into())),
                (
                    "GIT_AUTHOR_EMAIL".into(),
                    Some("synthetic@example.invalid".into()),
                ),
                ("GIT_COMMITTER_NAME".into(), Some("Synthetic Author".into())),
                (
                    "GIT_COMMITTER_EMAIL".into(),
                    Some("synthetic@example.invalid".into()),
                ),
                ("GIT_INDEX_FILE".into(), None),
                ("GIT_DIR".into(), None),
                ("GIT_WORK_TREE".into(), None),
                ("GIT_COMMON_DIR".into(), None),
                ("GIT_OBJECT_DIRECTORY".into(), None),
                ("GIT_ALTERNATE_OBJECT_DIRECTORIES".into(), None),
                ("GIT_NAMESPACE".into(), None),
                ("GIT_SHALLOW_FILE".into(), None),
                ("GIT_TEMPLATE_DIR".into(), None),
                ("GIT_CONFIG".into(), None),
                ("GIT_CONFIG_PARAMETERS".into(), None),
            ]);
            Self {
                owned,
                service,
                root,
            }
        }

        async fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let mut argv = vec![OsString::from("-C"), cwd.as_os_str().into()];
            argv.extend(args.iter().map(OsString::from));
            let plan = ProcessPlan {
                executable: self.service.git_executable.clone(),
                args: argv,
                cwd: self.owned.path().into(),
                env: self.service.git_environment.clone(),
                stdin: vec![],
                // Fixture creation is outside the production lookup budget.
                timeout: Duration::from_secs(20),
                output_cap: OUTPUT_CAP,
                pipe_drain_timeout: LOOKUP_TIMEOUT,
            };
            let output = crate::process::run_capped(&plan, &Cancellation::default())
                .await
                .unwrap();
            assert_eq!(
                output.outcome,
                ExitOutcome::Exited {
                    code: Some(0),
                    signal: None
                },
                "owned fixture Git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                !output.stdout_truncated && !output.stderr_truncated && !output.pipes_forced_closed
            );
            String::from_utf8(output.stdout).unwrap().trim().into()
        }

        async fn init(&self) {
            self.git(&self.root, &["init", "-b", "synthetic"]).await;
        }

        async fn commit(&self) {
            self.git(&self.root, &["add", "--all"]).await;
            self.git(&self.root, &["commit", "-m", "synthetic fixture"])
                .await;
        }

        fn cwd(&self) -> &str {
            self.root.to_str().unwrap()
        }
    }

    #[tokio::test]
    async fn display_reader_resolves_command_path_without_reusing_git_api_selection() {
        let mut f = Fixture::new();
        let empty_path = f.owned.path().join("empty-path");
        fs::create_dir(&empty_path).unwrap();
        let configured = f.owned.path().join("configured-but-absent-git");
        f.service.git_executable = configured.clone();
        f.service
            .git_environment
            .insert("PATH".into(), Some(empty_path.into_os_string()));
        // An explicit missing path is a startup failure. The production display
        // reader uses Go's bare command lookup, whose PATH absence is definitive.
        assert_eq!(f.service.project(f.root.to_str().unwrap()).await, None);
        assert_eq!(
            f.service
                .branch_reader()
                .project(f.root.to_str().unwrap())
                .await,
            Some(String::new())
        );
        assert_eq!(f.service.git_executable, configured);
    }

    #[tokio::test]
    async fn project_roots_cover_repository_subdirectory_worktree_bare_and_nonrepo() {
        let fixture = Fixture::new();
        fixture.init().await;
        fs::write(fixture.root.join("readme.txt"), "base\n").unwrap();
        fixture.commit().await;
        let expected = project_path(
            fixture.cwd(),
            &fixture
                .git(
                    &fixture.root,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )
                .await,
        )
        .unwrap();
        assert_eq!(
            fixture.service.project(fixture.cwd()).await,
            Some(expected.clone())
        );

        let subdir = fixture.root.join("inside").join("deep");
        fs::create_dir_all(&subdir).unwrap();
        assert_eq!(
            fixture.service.project(subdir.to_str().unwrap()).await,
            Some(expected.clone())
        );

        let worktree = fixture.owned.path().join("synthetic worktree");
        fixture
            .git(
                &fixture.root,
                &["worktree", "add", "-b", "side", worktree.to_str().unwrap()],
            )
            .await;
        assert_eq!(
            fixture.service.project(worktree.to_str().unwrap()).await,
            Some(expected)
        );
        assert_eq!(
            fixture.service.branch(worktree.to_str().unwrap()).await,
            "side"
        );

        let bare = fixture.owned.path().join("synthetic bare.git");
        fs::create_dir(&bare).unwrap();
        fixture.git(&bare, &["init", "--bare"]).await;
        let bare_expected = lexical_clean(Path::new(
            &fixture
                .git(
                    &bare,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )
                .await,
        ));
        assert_eq!(
            fixture.service.project(bare.to_str().unwrap()).await,
            Some(bare_expected.to_string_lossy().into_owned())
        );

        let outside = fixture.owned.path().join("nonrepo");
        fs::create_dir(&outside).unwrap();
        assert_eq!(
            fixture.service.project(outside.to_str().unwrap()).await,
            Some(String::new())
        );
        assert_eq!(fixture.service.branch(outside.to_str().unwrap()).await, "");
        assert_eq!(
            fixture.service.changes(outside.to_str().unwrap()).await,
            (0, 0, 0)
        );
        let missing = fixture.owned.path().join("missing-target");
        assert_eq!(
            fixture.service.project(missing.to_str().unwrap()).await,
            Some(String::new())
        );
    }

    #[tokio::test]
    async fn branches_and_stats_cover_unborn_detached_switch_text_binary_and_untracked() {
        let fixture = Fixture::new();
        fixture.init().await;
        fs::write(fixture.root.join("text.txt"), "one\ntwo\n").unwrap();
        assert_eq!(fixture.service.branch(fixture.cwd()).await, "");
        assert_eq!(fixture.service.changes(fixture.cwd()).await, (1, 0, 0));
        fs::write(fixture.root.join("binary.dat"), b"\0old\0").unwrap();
        fixture.commit().await;
        assert_eq!(fixture.service.branch(fixture.cwd()).await, "synthetic");
        assert_eq!(fixture.service.changes(fixture.cwd()).await, (0, 0, 0));

        fixture
            .git(&fixture.root, &["checkout", "-b", "changed"])
            .await;
        assert_eq!(fixture.service.branch(fixture.cwd()).await, "changed");
        fixture
            .git(&fixture.root, &["checkout", "--detach", "HEAD"])
            .await;
        let short = fixture
            .git(&fixture.root, &["rev-parse", "--short", "HEAD"])
            .await;
        assert_eq!(
            fixture.service.branch(fixture.cwd()).await,
            format!("detached:{short}")
        );

        fs::write(fixture.root.join("text.txt"), "one\nthree\nfour\n").unwrap();
        fs::write(fixture.root.join("binary.dat"), b"\0new\0").unwrap();
        fs::write(fixture.root.join("untracked.txt"), "not counted as added\n").unwrap();
        assert_eq!(fixture.service.changes(fixture.cwd()).await, (3, 2, 1));
    }

    #[tokio::test]
    async fn project_distinguishes_missing_path_lookup_from_spawn_failures() {
        let mut fixture = Fixture::new();
        fixture.service.git_executable = "synthetic-nonexistent-git-command".into();
        fixture
            .service
            .git_environment
            .insert("PATH".into(), Some(fixture.owned.path().into()));
        assert_eq!(
            fixture.service.project(fixture.cwd()).await,
            Some(String::new())
        );
        fixture.service.git_executable = fixture.owned.path().join("nonexistent-absolute-git");
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
        fixture.service.git_executable = super::super::resolve_git();
        fixture.service.hub_cwd = fixture.owned.path().join("missing-process-cwd");
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
        for blank in ["", " \n\t"] {
            assert_eq!(fixture.service.project(blank).await, Some(String::new()));
            assert_eq!(fixture.service.branch(blank).await, "");
            assert_eq!(fixture.service.changes(blank).await, (0, 0, 0));
        }
    }

    #[test]
    fn typed_outcomes_do_not_turn_transient_or_incomplete_output_into_nonrepo() {
        let output = ProcessOutput {
            stdout: vec![],
            stderr: vec![],
            stdout_truncated: false,
            stderr_truncated: false,
            pipes_forced_closed: false,
            outcome: ExitOutcome::Exited {
                code: Some(128),
                signal: None,
            },
        };
        assert!(LookupError::Output(output.clone()).answered());
        assert!(LookupError::MissingExecutable.answered());
        assert!(!LookupError::Start.answered());
        assert!(!LookupError::Expired.answered());
        for outcome in [ExitOutcome::TimedOut, ExitOutcome::Cancelled] {
            assert!(
                !LookupError::Output(ProcessOutput {
                    outcome,
                    ..output.clone()
                })
                .answered()
            );
        }
        assert!(
            !LookupError::Output(ProcessOutput {
                stdout_truncated: true,
                ..output.clone()
            })
            .answered()
        );
        assert!(
            !LookupError::Output(ProcessOutput {
                stderr_truncated: true,
                ..output.clone()
            })
            .answered()
        );
        assert!(
            !LookupError::Output(ProcessOutput {
                pipes_forced_closed: true,
                ..output
            })
            .answered()
        );
    }

    #[test]
    fn command_plan_keeps_argv_environment_and_one_lookup_budget() {
        let fixture = Fixture::new();
        let lookup = Lookup::new(&fixture.service);
        let first = lookup
            .plan("target with spaces", &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap();
        let second = lookup
            .plan("target with spaces", &["rev-parse", "--short", "HEAD"])
            .unwrap();
        assert_eq!(
            first.args,
            [
                "-C",
                "target with spaces",
                "rev-parse",
                "--abbrev-ref",
                "HEAD"
            ]
            .map(OsString::from)
        );
        assert_eq!(first.cwd, fixture.service.hub_cwd);
        assert_eq!(first.env, fixture.service.git_environment);
        assert_eq!(first.output_cap, OUTPUT_CAP);
        assert_eq!(first.pipe_drain_timeout, LOOKUP_TIMEOUT);
        assert!(first.timeout <= LOOKUP_TIMEOUT && second.timeout <= first.timeout);
        let expired = Lookup {
            service: &fixture.service,
            until: Instant::now(),
        };
        assert!(matches!(
            expired.plan("cwd", &["status"]),
            Err(LookupError::Expired)
        ));
    }

    #[test]
    fn numstat_matches_go_independent_signed_parsing_binary_and_overflow() {
        assert_eq!(
            change_lines("2\t3\ttext\n-\t-\tbinary\n\nmalformed\n+4\t-2\tother\nno\t7\n8\tbad\n"),
            (14, 8)
        );
        assert_eq!(
            change_lines("9223372036854775808\t-9223372036854775809\tbad\n"),
            (0, 0)
        );
        assert_eq!(
            change_lines("9223372036854775807\t-9223372036854775808\ta\n1\t-1\tb\n"),
            (i64::MIN, i64::MAX)
        );
    }

    #[test]
    fn project_paths_are_lexically_cleaned_without_case_folding_or_io() {
        let root = if cfg!(windows) {
            r"C:\Synthetic\Repo"
        } else {
            "/Synthetic/Repo"
        };
        let cwd = Path::new(root).join("inside");
        assert_eq!(
            project_path(cwd.to_str().unwrap(), "../.git/./\n"),
            Some(root.into())
        );
        assert_eq!(project_path(root, ".git"), Some(root.into()));
        assert_eq!(project_path(root, " \n"), None);
        assert_eq!(
            project_path("relative/inside", "../../bare.git"),
            Some(
                lexical_clean(Path::new("bare.git"))
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert_eq!(project_path("relative", "../.GIT"), Some(".GIT".into()));
        assert_eq!(
            lexical_clean(Path::new("../../a/../b")),
            PathBuf::from("../../b")
        );
    }

    #[cfg(unix)]
    fn script(fixture: &mut Fixture, content: &str) {
        use std::os::unix::fs::PermissionsExt;
        let executable = fixture.owned.path().join("owned-git-helper");
        fs::write(&executable, format!("#!/bin/sh\n{content}\n")).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        fixture.service.git_executable = executable;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn owned_helper_exercises_legacy_project_fallback_empty_output_and_timeout() {
        let mut fixture = Fixture::new();
        script(
            &mut fixture,
            "if [ \"$4\" = --path-format=absolute ]; then exit 129; fi\nprintf '../.git\\n'",
        );
        assert_eq!(
            fixture.service.project(fixture.cwd()).await,
            Some(fixture.owned.path().to_string_lossy().into_owned())
        );
        script(&mut fixture, "exit 0");
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
        script(&mut fixture, "while :; do :; done");
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
        script(
            &mut fixture,
            "if [ \"$4\" = --abbrev-ref ]; then printf 'HEAD\\n'; else exit 1; fi",
        );
        assert_eq!(fixture.service.branch(fixture.cwd()).await, "");
        script(
            &mut fixture,
            "if [ \"$3\" = status ]; then printf ' M one\\n\\n?? two\\n'; else exit 128; fi",
        );
        assert_eq!(fixture.service.changes(fixture.cwd()).await, (2, 0, 0));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn existing_helper_with_missing_interpreter_is_unresolved() {
        let mut fixture = Fixture::new();
        script(&mut fixture, "exit 0");
        fs::write(
            &fixture.service.git_executable,
            format!(
                "#!{}\n",
                fixture.owned.path().join("missing-interpreter").display()
            ),
        )
        .unwrap();
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
    }
}
