//! Git commands preserve argv boundaries, bounded output and one operation budget.
//! No network/process action is performed merely by constructing this service.
use super::{FilesService, Result, err, scope};
use crate::proto::time::Timestamp;
use crate::{
    hub::http::{Request, Response},
    process::{Cancellation, ExitOutcome, ProcessPlan},
    proto::{
        core::*,
        wire::{Field, GoWire, Schema},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[path = "git_suggest.rs"]
mod suggest;
#[path = "git_turns.rs"]
mod turns;
pub use turns::{GitTurnCompleted, GitTurnEndReservation, GitTurnSnapshot, GitTurnState};
#[derive(Default, Deserialize)]
#[serde(default)]
struct GitRequest {
    session: i64,
    subject: String,
    body: String,
    mode: String,
    language: String,
}
macro_rules! git_schema {
    ($name:ident,$go:literal,{$($key:literal:$kind:literal),*}) => {
        #[derive(Deserialize)] #[serde(transparent)] struct $name(GitRequest);
        impl GoWire for $name { const GO_TYPE:&'static str=$go; const SCHEMAS:&'static[Schema]=&[Schema{name:$go,fields:&[$(Field{name:$key,kind:$kind}),*]}]; }
    };
}
git_schema!(GitActionBody,"GitAction",{"session":"int","token":"string"});
git_schema!(GitCommitBody,"GitCommit",{"session":"int","token":"string","subject":"string","body":"string"});
git_schema!(GitSuggestBody,"GitSuggest",{"session":"int","token":"string","mode":"string","language":"string"});
#[derive(Clone, Debug, Serialize)]
pub(super) struct Ref {
    kind: String,
    name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    hash: String,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct StatusFile {
    pub status: String,
    pub path: String,
    pub added: Option<i64>,
    pub removed: Option<i64>,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct DiffFile {
    pub status: String,
    pub path: String,
    pub added: i64,
    pub removed: i64,
    pub diff: String,
}
pub(super) struct Git<'a> {
    service: &'a FilesService,
    until: Instant,
}
#[cfg(test)]
fn snapshot_test_diagnostic(fields: std::fmt::Arguments<'_>) {
    let thread = std::thread::current();
    let case = if thread.name()
        == Some(
            "application::event_observer::tests::end_reservation_release_and_warm_reconnect_preserve_the_capture_incarnation",
        ) {
        "warm_reconnect"
    } else {
        "other"
    };
    crate::logging::write_diagnostic(&format!(
        "git_turn_snapshot_diagnostic case={case} {fields}\n"
    ));
}
impl<'a> Git<'a> {
    fn new(service: &'a FilesService, seconds: u64) -> Self {
        Self {
            service,
            until: Instant::now() + Duration::from_secs(seconds),
        }
    }
    async fn run(&self, cwd: &Path, args: &[&str]) -> std::result::Result<String, String> {
        self.env(cwd, args, BTreeMap::new(), false).await
    }
    async fn env(
        &self,
        cwd: &Path,
        args: &[&str],
        env: BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>>,
        combined: bool,
    ) -> std::result::Result<String, String> {
        #[cfg(test)]
        let diagnostic_started = Instant::now();
        #[cfg(test)]
        let diagnostic_operation = match args {
            ["rev-parse", "--show-toplevel"] => "resolve",
            ["rev-parse", "--verify", "HEAD^{tree}"] => "head_tree",
            ["read-tree", "HEAD"] => "read_head",
            ["read-tree", "--empty"] => "read_empty",
            ["add", "-A", "--", "."] => "stage_worktree",
            ["write-tree"] => "write_tree",
            _ => "other",
        };
        let timeout = self.until.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            #[cfg(test)]
            snapshot_test_diagnostic(format_args!(
                "operation={diagnostic_operation} stage=budget_exhausted"
            ));
            return Err("git command timed out".into());
        }
        let mut all = vec![
            "-C".into(),
            cwd.as_os_str().into(),
            "-c".into(),
            "core.quotePath=false".into(),
        ];
        all.extend(args.iter().map(std::ffi::OsString::from));
        let mut command_env = self.service.git_environment.clone();
        command_env.extend(env);
        let plan = ProcessPlan {
            executable: self.service.git_executable.clone(),
            args: all,
            cwd: cwd.into(),
            env: command_env,
            stdin: vec![],
            timeout,
            output_cap: 16 * 1024 * 1024,
            pipe_drain_timeout: Duration::from_millis(250),
        };
        let out = crate::process::run_capped(&plan, &Cancellation::default())
            .await
            .map_err(|e| {
                #[cfg(test)]
                snapshot_test_diagnostic(format_args!(
                    "operation={diagnostic_operation} stage=process_io io_kind={:?} os_code={:?} budget_ms={} elapsed_ms={}",
                    e.kind(),
                    e.raw_os_error(),
                    timeout.as_millis(),
                    diagnostic_started.elapsed().as_millis()
                ));
                format!("git {}: {e}", args.join(" "))
            })?;
        let mut bytes = out.stdout;
        if combined {
            bytes.extend(&out.stderr);
        }
        if out.stdout_truncated || out.stderr_truncated || out.pipes_forced_closed {
            #[cfg(test)]
            snapshot_test_diagnostic(format_args!(
                "operation={diagnostic_operation} stage=output outcome={:?} stdout_truncated={} stderr_truncated={} pipes_forced_closed={} budget_ms={} elapsed_ms={}",
                out.outcome,
                out.stdout_truncated,
                out.stderr_truncated,
                out.pipes_forced_closed,
                timeout.as_millis(),
                diagnostic_started.elapsed().as_millis()
            ));
            return Err("git command output exceeded limit or pipes failed to close".into());
        }
        match out.outcome {
            ExitOutcome::Exited { code: Some(0), .. } => {
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            }
            _ => {
                #[cfg(test)]
                snapshot_test_diagnostic(format_args!(
                    "operation={diagnostic_operation} stage=exit outcome={:?} budget_ms={} elapsed_ms={}",
                    out.outcome,
                    timeout.as_millis(),
                    diagnostic_started.elapsed().as_millis()
                ));
                let output = if combined { bytes } else { out.stderr };
                Err(format!(
                    "git {}: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&output).trim()
                ))
            }
        }
    }
    async fn resolve(&self, core: &dyn SessionCore, sid: i64) -> Result<(PathBuf, PathBuf)> {
        let session = core.snapshot(LiveSessionId(sid)).ok_or_else(|| {
            err(
                400,
                "bad_session",
                &format!("session not found (sid={sid})"),
            )
        })?;
        if session.cwd.trim().is_empty() {
            return Err(err(
                400,
                "no_cwd",
                &format!("session has no cwd (sid={sid})"),
            ));
        }
        let cwd = PathBuf::from(session.cwd);
        let root = self
            .run(&cwd, &["rev-parse", "--show-toplevel"])
            .await
            .map_err(|e| {
                err(
                    400,
                    "not_git_repo",
                    &sanitize_error(&format!("not_git_repo: {e}")),
                )
            })?;
        if root.trim().is_empty() {
            return Err(err(400, "not_git_repo", "not_git_repo"));
        }
        Ok((PathBuf::from(root.trim()), cwd))
    }
    async fn branch(&self, cwd: &Path) -> String {
        let branch = self
            .run(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
            .await
            .unwrap_or_default()
            .trim()
            .to_owned();
        if branch == "HEAD" {
            let short = self
                .run(cwd, &["rev-parse", "--short", "HEAD"])
                .await
                .unwrap_or_default();
            format!("detached:{}", short.trim())
        } else {
            branch
        }
    }
    async fn head(&self, cwd: &Path) -> String {
        self.run(cwd, &["rev-parse", "HEAD"])
            .await
            .unwrap_or_default()
            .trim()
            .into()
    }
}
pub fn valid_revision(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._/-".contains(&c))
}
fn command_error(e: String) -> Response {
    err(500, "git_command_failed", &sanitize_error(&e))
}
pub fn sanitize_error(s: &str) -> String {
    let mut v = s.to_owned();
    for (pattern, repl) in [
        (r#"[A-Za-z][A-Za-z0-9+.\-]*://[^\s\"'<>|]*"#, "<url>"),
        (r#"\\\\[^\s\"'<>|]+"#, "<path>"),
        (r#"[A-Za-z]:[\\/][^\s\"'<>|]*"#, "<path>"),
        (r#"(^|[\s\"'(\[=:,])/[^\s\"'<>|)\]:]*"#, "${1}<path>"),
    ] {
        if let Ok(re) = regex::Regex::new(pattern) {
            v = re.replace_all(&v, repl).into_owned();
        }
    }
    let lines: Vec<_> = v.trim().lines().collect();
    let mut out = lines[..lines.len().min(12)].join("\n");
    let trunc = lines.len() > 12 || out.len() > 900;
    if out.len() > 900 {
        let mut n = 900;
        while !out.is_char_boundary(n) {
            n -= 1;
        }
        out.truncate(n);
    }
    if trunc {
        out.push_str(" … (truncated; full output is in the Hub log)");
    }
    out
}
impl FilesService {
    pub(super) async fn git_handle(&self, r: &Request, core: &dyn SessionCore) -> Result<Response> {
        let post = r.method == "POST";
        let q: GitRequest = if post {
            let body = &r.body[..r.body.len().min(1024 * 1024)];
            match r.path.as_str() {
                "/api/git-commit-all" => {
                    crate::proto::decode_http_json::<GitCommitBody>(body).map(|v| v.0)
                }
                "/api/git-commit-message" => {
                    crate::proto::decode_http_json::<GitSuggestBody>(body).map(|v| v.0)
                }
                _ => crate::proto::decode_http_json::<GitActionBody>(body).map(|v| v.0),
            }
            .map_err(|_| err(400, "bad_request", "invalid json"))?
        } else {
            GitRequest {
                session: r
                    .query("session")
                    .parse()
                    .map_err(|_| err(400, "bad_request", "session is required"))?,
                ..Default::default()
            }
        };
        let timeout = match r.path.as_str() {
            "/api/git-commit-all" => 120,
            "/api/git-push" => 60,
            "/api/git-fetch" | "/api/git-pull" => 30,
            _ => 5,
        };
        let git = Git::new(self, timeout);
        if r.path == "/api/git-show" {
            let hash = r.query("hash");
            if hash.trim().is_empty() {
                return Err(err(400, "bad_request", "hash is required"));
            }
            if !valid_revision(hash.trim()) {
                return Err(err(400, "bad_request", "invalid hash format"));
            }
        }
        if r.path == "/api/git-log" {
            let rev = r.query("ref");
            if !rev.trim().is_empty() && rev.trim() != "--all" && !valid_revision(rev.trim()) {
                return Err(err(400, "bad_request", "invalid ref format"));
            }
        }
        if r.path == "/api/git-commit-all" && sanitize_message(&q.subject, 200).is_empty() {
            return Err(err(400, "bad_request", "subject is required"));
        }
        if matches!(
            r.path.as_str(),
            "/api/git-commit-all" | "/api/git-commit-message"
        ) && q.session <= 0
        {
            return Err(err(400, "bad_request", "session is required"));
        }
        let (root, cwd) = git.resolve(core, q.session).await?;
        let value = match r.path.as_str() {
            "/api/git-log" => git.log(r, &root, &cwd).await?,
            "/api/git-show" => git.show(r.query("hash").trim(), &root, &cwd).await?,
            "/api/git-refs" => git.refs(&cwd).await?,
            "/api/git-status" => git.status(&root, &cwd).await?,
            "/api/git-diff" => git.diff(&root, &cwd, None).await?,
            "/api/git-turns" => self.list_turns(core, q.session, &root)?,
            "/api/git-turn-diff" => {
                let turn = r
                    .query("turn")
                    .parse::<i64>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| err(400, "bad_request", "turn must be a positive integer"))?;
                let selected = self.select_turn(core, q.session, turn)?;
                git.diff(
                    &root,
                    &root,
                    Some((&selected.start_tree, &selected.end_tree)),
                )
                .await?
            }
            "/api/git-fetch" | "/api/git-pull" | "/api/git-push" => {
                let command = r.path.strip_prefix("/api/git-").unwrap();
                let args = if command == "pull" {
                    vec!["pull", "--ff-only"]
                } else {
                    vec![command]
                };
                let env = BTreeMap::from([
                    ("GIT_TERMINAL_PROMPT".into(), Some("0".into())),
                    ("GIT_ASKPASS".into(), Some("echo".into())),
                ]);
                let out = git.env(&cwd, &args, env, true).await.map_err(|e| {
                    let (status, code) = classify_error(command, &e);
                    err(status, code, &sanitize_error(&e))
                })?;
                if out.is_empty() {
                    json!({"ok":true})
                } else {
                    json!({"ok":true,"output":out})
                }
            }
            "/api/git-commit-all" => git.commit(&root, &cwd, &q).await?,
            "/api/git-commit-message" => {
                let status = git
                    .run(&cwd, &["status", "--short", "--porcelain=v1", "-z"])
                    .await
                    .map_err(command_error)?;
                let files = parse_status(&status);
                if files.is_empty() {
                    return Err(err(400, "no_changes", "working tree has no changes"));
                }
                if q.mode.eq_ignore_ascii_case("ai") {
                    return self.start_ai_commit(core, q.session, &q.language).await;
                }
                let stat = git
                    .run(&cwd, &["diff", "--stat", "HEAD", "--"])
                    .await
                    .unwrap_or_default();
                let mut diff = git
                    .run(&cwd, &["diff", "HEAD", "--"])
                    .await
                    .unwrap_or_default();
                let notice = if diff.len() > 48 * 1024 {
                    truncate(&mut diff, 48 * 1024);
                    "Diff context truncated to 48 KiB."
                } else {
                    ""
                };
                let weights = parse_numstat(
                    &git.run(&cwd, &["diff", "--numstat", "--no-renames", "HEAD", "--"])
                        .await
                        .unwrap_or_default(),
                );
                let mut weights: BTreeMap<String, i64> =
                    weights.into_iter().map(|(k, (a, b))| (k, a + b)).collect();
                for f in &files {
                    if f.status == "??" && !weights.contains_key(&f.path) {
                        let path = cwd.join(&f.path);
                        let n = scope::parent_capability(&path)
                            .and_then(|(d, n)| d.read(&n, 1024 * 1024))
                            .ok()
                            .filter(|b| !b.contains(&0))
                            .map(|b| {
                                b.iter().filter(|b| **b == b'\n').count()
                                    + usize::from(!b.is_empty() && !b.ends_with(b"\n"))
                            })
                            .unwrap_or(0);
                        weights.insert(f.path.clone(), n as i64);
                    }
                }
                let (subject, body) =
                    suggest::suggest(&files, stat.trim(), &diff, notice, &q.language, &weights);
                json!({"ok":true,"subject":subject,"body":body})
            }
            _ => return Err(err(404, "not_found", "not found")),
        };
        Ok(Response::json(200, &value))
    }
}
impl Git<'_> {
    async fn log(&self, r: &Request, root: &Path, cwd: &Path) -> Result<Value> {
        let raw = r.query("ref");
        let rev = if raw.trim().is_empty() {
            "HEAD"
        } else {
            raw.trim()
        };
        let limit = r
            .query("limit")
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .unwrap_or(100)
            .min(1000);
        let skip = r.query("skip").parse::<i64>().unwrap_or(0).max(0);
        let head = self.head(cwd).await;
        let branch = self.branch(cwd).await;
        let mut commits = vec![];
        let mut more = false;
        if !head.is_empty() || rev == "--all" {
            let max = format!("--max-count={limit}");
            let sk = format!("--skip={skip}");
            let mut args = vec![
                "log",
                &max,
                &sk,
                "--date-order",
                "--decorate=full",
                "--pretty=format:%H%x09%h%x09%P%x09%an%x09%ae%x09%aI%x09%D%x09%s",
                rev,
            ];
            if rev != "--all" {
                args.push("--");
            }
            let out = self.run(cwd, &args).await.map_err(command_error)?;
            for line in out.lines() {
                let p: Vec<_> = line.trim_end_matches('\r').splitn(8, '\t').collect();
                if p.len() < 8 {
                    continue;
                }
                commits.push(json!({"hash":p[0],"short_hash":p[1],"parents":p[2].split_whitespace().collect::<Vec<_>>(),"author_name":p[3],"author_email":p[4],"author_date":p[5],"refs":parse_decorate(p[6]),"subject":p[7]}));
            }
            let mut args = vec!["rev-list", "--count", rev];
            if rev != "--all" {
                args.push("--");
            }
            if let Ok(out) = self.run(cwd, &args).await
                && let Ok(total) = out.trim().parse::<i64>()
            {
                more = skip + (commits.len() as i64) < total;
            }
        }
        Ok(
            json!({"ok":true,"git_root":root,"branch":branch,"head_hash":head,"commits":commits,"limit":limit,"skip":skip,"has_more":more}),
        )
    }
    async fn refs(&self, cwd: &Path) -> Result<Value> {
        let out = self
            .run(
                cwd,
                &["for-each-ref", "--format=%(refname)%09%(objectname:short)"],
            )
            .await
            .map_err(command_error)?;
        let mut refs = Vec::new();
        for line in out.lines() {
            let Some((name, hash)) = line.split_once('\t') else {
                continue;
            };
            for (prefix, kind) in [
                ("refs/heads/", "local"),
                ("refs/remotes/", "remote"),
                ("refs/tags/", "tag"),
            ] {
                if let Some(n) = name.strip_prefix(prefix) {
                    if n.is_empty() || (kind == "remote" && n.ends_with("/HEAD")) {
                        break;
                    }
                    refs.push(Ref {
                        kind: kind.into(),
                        name: n.into(),
                        hash: hash.trim_end_matches('\r').into(),
                    });
                    break;
                }
            }
        }
        let head = if let Ok(out) = self.run(cwd, &["symbolic-ref", "--short", "HEAD"]).await {
            out.trim().to_owned()
        } else {
            let out = self
                .run(cwd, &["rev-parse", "--short", "HEAD"])
                .await
                .unwrap_or_default();
            if out.trim().is_empty() {
                String::new()
            } else {
                format!("detached:{}", out.trim())
            }
        };
        let mut v = json!({"ok":true,"head":head,"refs":refs});
        if let Ok(origin) = self.run(cwd, &["remote", "get-url", "origin"]).await
            && let Some(url) = github_url(origin.trim())
        {
            v["github_url"] = json!(url);
        }
        Ok(v)
    }
    async fn status(&self, root: &Path, cwd: &Path) -> Result<Value> {
        let out = self
            .run(cwd, &["status", "--short", "--porcelain=v1", "-z"])
            .await
            .map_err(command_error)?;
        let mut files = parse_status(&out);
        let stats = parse_numstat(
            &self
                .run(cwd, &["diff", "--numstat", "HEAD", "--"])
                .await
                .unwrap_or_default(),
        );
        for f in &mut files {
            if let Some((a, b)) = stats.get(&f.path) {
                f.added = Some(*a);
                f.removed = Some(*b);
            }
        }
        let branch = self.branch(cwd).await;
        let head = self.head(cwd).await;
        let (mut ahead, mut behind, mut upstream) = (0, 0, false);
        if let Ok(out) = self
            .run(
                cwd,
                &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
            )
            .await
        {
            let counts: Vec<_> = out.split_whitespace().collect();
            if counts.len() == 2
                && let (Ok(b), Ok(a)) = (counts[0].parse::<i64>(), counts[1].parse::<i64>())
            {
                behind = b;
                ahead = a;
                upstream = true;
            }
        }
        let sum = json!({"files_changed":files.len(),"added":files.iter().filter_map(|f|f.added).sum::<i64>(),"removed":files.iter().filter_map(|f|f.removed).sum::<i64>()});
        Ok(
            json!({"ok":true,"git_root":root,"repo_name":root.file_name(),"branch":branch,"head_hash":head,"has_changes":!files.is_empty(),"ahead":ahead,"behind":behind,"has_upstream":upstream,"files":files,"summary":sum}),
        )
    }
    async fn show(&self, hash: &str, root: &Path, cwd: &Path) -> Result<Value> {
        let meta = self
            .run(
                cwd,
                &[
                    "show",
                    "-s",
                    "--decorate=full",
                    "--pretty=format:%H%x09%P%x09%an%x09%ae%x09%aI%x09%D%x09%s%x1f%b",
                    hash,
                    "--",
                ],
            )
            .await
            .map_err(command_error)?;
        let (head, body) = meta
            .trim_end_matches(['\r', '\n'])
            .split_once('\x1f')
            .unwrap_or((meta.trim_end(), ""));
        let p: Vec<_> = head.splitn(7, '\t').collect();
        if p.len() < 7 {
            return Err(err(
                500,
                "git_command_failed",
                "malformed git show meta output",
            ));
        }
        let out = self
            .run(
                cwd,
                &["show", "--name-status", "--pretty=format:", hash, "--"],
            )
            .await
            .map_err(command_error)?;
        let mut files = parse_names(&out);
        let stats = parse_numstat(
            &self
                .run(cwd, &["show", "--numstat", "--pretty=format:", hash, "--"])
                .await
                .unwrap_or_default(),
        );
        for f in &mut files {
            if let Some((a, b)) = stats.get(&f.path) {
                f.added = *a;
                f.removed = *b;
            }
            if let Ok(d) = self
                .run(root, &["show", "--pretty=format:", hash, "--", &f.path])
                .await
            {
                f.diff = cap_diff(d);
            }
        }
        Ok(
            json!({"ok":true,"hash":p[0],"parents":p[1].split_whitespace().collect::<Vec<_>>(),"author_name":p[2],"author_email":p[3],"author_date":p[4],"subject":p[6],"body":body.trim(),"refs":parse_decorate(p[5]),"files":files}),
        )
    }
    pub(super) async fn diff(
        &self,
        root: &Path,
        cwd: &Path,
        trees: Option<(&str, &str)>,
    ) -> Result<Value> {
        let head = match trees {
            Some((_, end)) => end.to_owned(),
            None => self.head(cwd).await,
        };
        let mut files = if let Some((a, b)) = trees {
            parse_names(
                &self
                    .run(root, &["diff", "--name-status", a, b, "--"])
                    .await
                    .map_err(command_error)?,
            )
        } else {
            parse_status(
                &self
                    .run(
                        cwd,
                        &[
                            "status",
                            "--short",
                            "--porcelain=v1",
                            "-z",
                            "--untracked-files=all",
                        ],
                    )
                    .await
                    .map_err(command_error)?,
            )
            .into_iter()
            .map(|f| DiffFile {
                status: f.status,
                path: f.path,
                added: 0,
                removed: 0,
                diff: String::new(),
            })
            .collect()
        };
        let stats = if let Some((a, b)) = trees {
            parse_numstat(
                &self
                    .run(root, &["diff", "--numstat", a, b, "--"])
                    .await
                    .unwrap_or_default(),
            )
        } else if !head.is_empty() {
            parse_numstat(
                &self
                    .run(cwd, &["diff", "--numstat", "HEAD", "--"])
                    .await
                    .unwrap_or_default(),
            )
        } else {
            BTreeMap::new()
        };
        for f in &mut files {
            if trees.is_none() && (f.status == "??" || head.is_empty()) {
                if f.status == "??" {
                    f.status = "A".into();
                }
                (f.diff, f.added) = untracked(root, &f.path);
            } else {
                if let Some((a, b)) = stats.get(&f.path) {
                    f.added = *a;
                    f.removed = *b;
                }
                let args = if let Some((a, b)) = trees {
                    vec!["diff", a, b, "--", &f.path]
                } else {
                    vec!["diff", "HEAD", "--", &f.path]
                };
                if let Ok(out) = self.run(root, &args).await {
                    f.diff = cap_diff(out);
                }
            }
        }
        let summary = json!({"files_changed":files.len(),"added":files.iter().map(|f|f.added).sum::<i64>(),"removed":files.iter().map(|f|f.removed).sum::<i64>()});
        Ok(
            json!({"ok":true,"git_root":root,"repo_name":root.file_name(),"branch":self.branch(cwd).await,"head_hash":head,"files":files,"summary":summary}),
        )
    }
    async fn commit(&self, root: &Path, cwd: &Path, q: &GitRequest) -> Result<Value> {
        let subject = sanitize_message(&q.subject, 200);
        let body = sanitize_message(&q.body, 8192);
        let changed = parse_status(
            &self
                .run(cwd, &["status", "--short", "--porcelain=v1", "-z"])
                .await
                .map_err(command_error)?,
        )
        .len();
        if changed == 0 {
            return Err(err(400, "no_changes", "working tree has no changes"));
        }
        self.run(root, &["add", "-A"]).await.map_err(commit_error)?;
        if self
            .run(root, &["diff", "--cached", "--quiet"])
            .await
            .is_ok()
        {
            return Err(err(400, "no_changes", "no staged changes after git add -A"));
        }
        let mut args = vec!["commit", "-m", &subject];
        if !body.is_empty() {
            args.extend(["-m", &body]);
        }
        self.run(root, &args).await.map_err(commit_error)?;
        Ok(
            json!({"ok":true,"hash":self.head(root).await,"short_hash":self.run(root,&["rev-parse","--short","HEAD"]).await.unwrap_or_default().trim(),"subject":subject,"files_changed":changed}),
        )
    }
}
fn commit_error(e: String) -> Response {
    let (status, code) = classify_error("commit", &e);
    err(status, code, &sanitize_error(&e))
}
pub fn classify_error(command: &str, output: &str) -> (u16, &'static str) {
    let m = output.to_lowercase();
    let has = |p: &[&str]| p.iter().any(|p| m.contains(p));
    match command {
        "push" if has(&["non-fast-forward", "fetch first", "rejected"]) => {
            (409, "rejected_non_fast_forward")
        }
        "push" if has(&["no upstream", "no configured push destination"]) => (400, "no_upstream"),
        "push"
            if has(&[
                "authentication failed",
                "could not read username",
                "permission denied",
                "publickey",
            ]) =>
        {
            (502, "auth_failed")
        }
        "pull" if has(&["not possible to fast-forward", "non-fast-forward"]) => {
            (409, "not_fast_forward")
        }
        "pull"
            if (m.contains("local changes") && m.contains("overwritten"))
                || has(&[
                    "would be overwritten by merge",
                    "please commit your changes or stash them",
                ]) =>
        {
            (409, "local_changes")
        }
        "pull" if has(&["no tracking information", "no upstream"]) => (400, "no_upstream"),
        "commit"
            if has(&[
                "author identity unknown",
                "please tell me who you are",
                "unable to auto-detect email address",
            ]) =>
        {
            (400, "commit_identity_missing")
        }
        "commit" if m.contains("nothing to commit") => (400, "no_changes"),
        _ => (500, "git_command_failed"),
    }
}
pub fn sanitize_message(s: &str, cap: usize) -> String {
    let mut s: String = s
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| (*c >= ' ' && *c != '\x7f') || matches!(c, '\t' | '\n'))
        .collect::<String>()
        .trim()
        .into();
    truncate(&mut s, cap);
    s
}
fn truncate(s: &mut String, cap: usize) {
    if s.len() > cap {
        let mut n = cap;
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        s.truncate(n);
    }
}
fn cap_diff(s: String) -> String {
    let mut s = s.trim_start_matches('\n').to_owned();
    if s.len() > 256 * 1024 {
        truncate(&mut s, 256 * 1024);
        s.push_str("\n(truncated)");
    }
    s
}
pub(super) fn parse_status(raw: &str) -> Vec<StatusFile> {
    let entries: Vec<_> = raw.split('\0').collect();
    let mut files = vec![];
    let mut i = 0;
    while i < entries.len() {
        let e = entries[i];
        i += 1;
        if e.len() < 4 || !e.is_char_boundary(3) {
            continue;
        }
        let status = e[..2].trim();
        let short = if status == "??" {
            status
        } else {
            &status[..status.len().min(1)]
        };
        let path = &e[3..];
        if status.contains(['R', 'C']) && i < entries.len() && !entries[i].is_empty() {
            i += 1;
        }
        files.push(StatusFile {
            status: short.into(),
            path: path.into(),
            added: None,
            removed: None,
        });
    }
    files
}
fn parse_decorate(s: &str) -> Option<Vec<Ref>> {
    if s.trim().is_empty() {
        return None;
    }
    let mut refs = vec![];
    for part in s.split(',') {
        let mut p = part.trim();
        if let Some((_, rhs)) = p.split_once("->") {
            p = rhs.trim();
        }
        if p.is_empty() || p == "HEAD" {
            continue;
        }
        let (kind, name) = if let Some(n) = p.strip_prefix("refs/heads/") {
            ("local", n)
        } else if let Some(n) = p.strip_prefix("refs/remotes/") {
            ("remote", n)
        } else if let Some(n) = p.strip_prefix("refs/tags/") {
            ("tag", n)
        } else if let Some(n) = p.strip_prefix("tag:") {
            ("tag", n.trim().trim_start_matches("refs/tags/"))
        } else if p.starts_with("origin/") {
            ("remote", p)
        } else {
            ("local", p)
        };
        if !name.is_empty() {
            refs.push(Ref {
                kind: kind.into(),
                name: name.into(),
                hash: String::new(),
            });
        }
    }
    Some(refs)
}
fn parse_names(s: &str) -> Vec<DiffFile> {
    s.lines()
        .filter_map(|l| {
            let p: Vec<_> = l.trim_end_matches('\r').split('\t').collect();
            if p.len() < 2 {
                return None;
            }
            let status = &p[0][..p[0].len().min(1)];
            let path = if matches!(status, "R" | "C") && p.len() >= 3 {
                p[2]
            } else {
                p[1]
            };
            Some(DiffFile {
                status: status.into(),
                path: path.into(),
                added: 0,
                removed: 0,
                diff: String::new(),
            })
        })
        .collect()
}
fn rename_path(s: &str) -> String {
    if let (Some(l), Some(r)) = (s.find('{'), s.find('}'))
        && r > l
    {
        if let Some((_, new)) = s[l + 1..r].split_once(" => ") {
            return format!("{}{}{}", &s[..l], new.trim(), &s[r + 1..])
                .replace("//", "/")
                .trim_end_matches('/')
                .into();
        }
        return s.into();
    }
    s.rsplit_once(" => ")
        .map(|(_, p)| p.trim().to_owned())
        .unwrap_or_else(|| s.into())
}
fn parse_numstat(s: &str) -> BTreeMap<String, (i64, i64)> {
    s.lines()
        .filter_map(|l| {
            let p: Vec<_> = l.trim_end_matches('\r').splitn(3, '\t').collect();
            if p.len() != 3 {
                return None;
            }
            Some((
                rename_path(p[2]),
                (p[0].parse().unwrap_or(0), p[1].parse().unwrap_or(0)),
            ))
        })
        .collect()
}
fn github_url(remote: &str) -> Option<String> {
    let ssh_prefix = "git@github.com:"; // secrets-scan: allow git@github.com -- public GitHub SSH service principal, not a personal address
    let path = if let Some(p) = remote.strip_prefix(ssh_prefix) {
        p.to_owned()
    } else {
        let url = url::Url::parse(remote).ok()?;
        if url.host_str() != Some("github.com") {
            return None;
        }
        url.path().into()
    };
    let path = path.trim_start_matches('/').trim_end_matches(".git");
    if path.is_empty() {
        None
    } else {
        Some(format!("https://github.com/{path}"))
    }
}
fn untracked(root: &Path, rel: &str) -> (String, i64) {
    let path = scope::clean(&root.join(rel));
    if !path.starts_with(root) {
        return (String::new(), 0);
    }
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        let head = format!(
            "diff --git a/{rel} b/{rel}\nnew file mode 120000\n--- /dev/null\n+++ b/{rel}\n"
        );
        return (
            match std::fs::read_link(path) {
                Ok(p) => format!("{head}Symlink -> {}\n", p.display()),
                Err(_) => format!("{head}Symlink (new)\n"),
            },
            0,
        );
    }
    let data = scope::parent_capability(&path).and_then(|(d, n)| d.read(&n, 256 * 1024 + 1));
    match data {
        Ok(b) => add_diff(rel, &b),
        Err(_) => (String::new(), 0),
    }
}
fn add_diff(path: &str, data: &[u8]) -> (String, i64) {
    let mut out = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
    );
    if data[..data.len().min(8000)].contains(&0) {
        out.push_str("Binary file (new)\n");
        return (out, 0);
    }
    if data.is_empty() {
        return (out, 0);
    }
    let text = String::from_utf8_lossy(data);
    let lines: Vec<_> = text
        .strip_suffix('\n')
        .unwrap_or(&text)
        .split('\n')
        .collect();
    out.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
    let mut truncated = false;
    for line in &lines {
        if out.len() > 256 * 1024 {
            truncated = true;
            break;
        }
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    if truncated {
        out.push_str("(truncated)\n");
    } else if !text.ends_with('\n') {
        out.push_str("\\ No newline at end of file\n");
    }
    (out, lines.len() as i64)
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
