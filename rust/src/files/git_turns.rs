use super::*;
use std::sync::Arc;
#[derive(Clone, Debug, Serialize)]
pub struct GitTurnSnapshot {
    pub turn: i64,
    pub started_at: String,
    pub ended_at: String,
    #[serde(skip)]
    pub start_tree: String,
    #[serde(skip)]
    pub end_tree: String,
    #[serde(rename = "files_changed")]
    pub files: i64,
    pub added: i64,
    pub removed: i64,
}
struct SessionTurns {
    binding: SessionBinding,
    gate: Arc<tokio::sync::Mutex<()>>,
    start: Option<(String, String)>,
    turns: Vec<GitTurnSnapshot>,
}
struct AwaitCommit {
    binding: SessionBinding,
    deadline: Timestamp,
    language: String,
    buffer: String,
    progressed: bool,
}
#[derive(Default)]
pub struct GitTurnState {
    sessions: BTreeMap<i64, SessionTurns>,
    awaiting: BTreeMap<i64, AwaitCommit>,
}
#[derive(Debug)]
pub struct GitTurnCompleted {
    pub snapshot: GitTurnSnapshot,
    pub git_root: PathBuf,
    pub diff: Value,
    pub websocket: Value,
    /// A tree snapshot can be recorded even when its summary command fails,
    /// matching Go's fallback. The Hub logs this already-sanitized detail.
    pub diff_error: Option<String>,
    // Keep next turn behind handoff persistence/broadcast performed by the Hub.
    // Dropping the completed result releases the per-incarnation capture gate.
    _completion: tokio::sync::OwnedMutexGuard<()>,
}
pub(super) fn format_turn_timestamp(time: Timestamp) -> Result<String> {
    crate::proto::time::format_rfc3339(time)
        .map_err(|_| err(500, "invalid_timestamp", "invalid turn timestamp"))
}
impl FilesService {
    /// Must be awaited BEFORE the matching confirmed input is delivered. End
    /// capture may run asynchronously, but the next start waits on the same gate.
    /// The returned completed turn is then handed to the real handoff writer and
    /// broadcaster; no user branch or real index is modified.
    pub async fn observe_git_turn(
        &self,
        core: &dyn SessionCore,
        binding: SessionBinding,
        started_at: &str,
        ended_at: Option<&str>,
    ) -> Result<Option<GitTurnCompleted>> {
        if !core.is_current(binding) {
            return Err(err(409, "stale_binding", "session binding changed"));
        }
        let gate = {
            let mut state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
            let entry = state
                .sessions
                .entry(binding.session.0)
                .or_insert_with(|| SessionTurns {
                    binding,
                    gate: Arc::new(tokio::sync::Mutex::new(())),
                    start: None,
                    turns: Vec::new(),
                });
            if entry.binding.incarnation != binding.incarnation {
                *entry = SessionTurns {
                    binding,
                    gate: Arc::new(tokio::sync::Mutex::new(())),
                    start: None,
                    turns: vec![],
                };
            }
            entry.binding = binding;
            entry.gate.clone()
        };
        let _gate = gate.lock_owned().await;
        if !core.is_current(binding) {
            return Err(err(409, "stale_binding", "session binding changed"));
        }
        let start = {
            let state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
            state
                .sessions
                .get(&binding.session.0)
                .and_then(|s| s.start.clone())
        };
        if ended_at.is_none() && start.is_some() {
            return Ok(None);
        }
        if ended_at.is_some() && start.is_none() {
            return Ok(None);
        }
        let git = Git::new(self, 5);
        let capture = async {
            let (root, _) = git.resolve(core, binding.session.0).await?;
            let tree = git.worktree_tree(&root).await?;
            Ok::<_, Response>((root, tree))
        }
        .await;
        let (root, tree) = match capture {
            Ok(value) => value,
            Err(error) => {
                if ended_at.is_some() {
                    self.clear_turn_start(binding);
                }
                return Err(error);
            }
        };
        if core
            .details(binding.session)
            .is_none_or(|current| current.binding.incarnation != binding.incarnation)
        {
            return Err(err(
                409,
                "stale_binding",
                "session binding changed during capture",
            ));
        }
        if ended_at.is_none() {
            let mut state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = state.sessions.get_mut(&binding.session.0) {
                s.start = Some((tree, started_at.into()));
            }
            return Ok(None);
        }
        let (start_tree, started) = start.unwrap();
        let git = Git::new(self, 5);
        // Source git_turns.go records the immutable trees even if summary
        // extraction fails, with zero counts and a logged diagnostic.
        let (diff, diff_error) = match git.diff(&root, &root, Some((&start_tree, &tree))).await {
            Ok(diff) => (diff, None),
            Err(error) => {
                let detail = serde_json::from_slice::<Value>(&error.body)
                    .ok()
                    .and_then(|v| v["detail"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "git turn summary failed".into());
                (
                    json!({"ok":true,"git_root":root,"repo_name":root.file_name(),"branch":"","head_hash":"","files":null,"summary":{"files_changed":0,"added":0,"removed":0}}),
                    Some(detail),
                )
            }
        };
        let summary = &diff["summary"];
        let mut state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        let s = state
            .sessions
            .get_mut(&binding.session.0)
            .ok_or_else(|| err(409, "stale_binding", "session capture removed"))?;
        if s.binding.incarnation != binding.incarnation {
            return Err(err(409, "stale_binding", "session binding changed"));
        }
        let turn = s.turns.last().map(|s| s.turn + 1).unwrap_or(1);
        let ended = ended_at
            .and_then(super::super::time::parse)
            .unwrap_or_else(Timestamp::now);
        let ended = format_turn_timestamp(ended)?;
        let started = match super::super::time::parse(&started) {
            Some(started) => format_turn_timestamp(started)?,
            None => started,
        };
        let snapshot = GitTurnSnapshot {
            turn,
            started_at: started.clone(),
            ended_at: ended.clone(),
            start_tree,
            end_tree: tree,
            files: summary["files_changed"].as_i64().unwrap_or(0),
            added: summary["added"].as_i64().unwrap_or(0),
            removed: summary["removed"].as_i64().unwrap_or(0),
        };
        s.turns.push(snapshot.clone());
        if s.turns.len() > 100 {
            s.turns.remove(0);
        }
        s.start = None;
        let websocket = json!({"type":"git_turn","session_id":binding.session.0,"turn":turn,"started_at":started,"ended_at":ended,"files_changed":snapshot.files,"added":snapshot.added,"removed":snapshot.removed});
        Ok(Some(GitTurnCompleted {
            snapshot,
            git_root: root,
            diff,
            websocket,
            diff_error,
            _completion: _gate,
        }))
    }
    fn clear_turn_start(&self, binding: SessionBinding) {
        let mut state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state
            .sessions
            .get_mut(&binding.session.0)
            .filter(|s| s.binding.incarnation == binding.incarnation)
        {
            s.start = None;
        }
    }
    pub fn forget_git_session(&self, binding: SessionBinding) {
        let mut state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .sessions
            .get(&binding.session.0)
            .is_some_and(|s| s.binding.incarnation == binding.incarnation)
        {
            state.sessions.remove(&binding.session.0);
        }
        if state
            .awaiting
            .get(&binding.session.0)
            .is_some_and(|s| s.binding.incarnation == binding.incarnation)
        {
            state.awaiting.remove(&binding.session.0);
        }
    }
    pub(super) fn list_turns(
        &self,
        core: &dyn SessionCore,
        sid: i64,
        root: &Path,
    ) -> Result<Value> {
        let details = core
            .details(LiveSessionId(sid))
            .ok_or_else(|| err(400, "bad_session", "session not found"))?;
        let state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        let turns = state
            .sessions
            .get(&sid)
            .filter(|s| s.binding.incarnation == details.binding.incarnation)
            .map(|s| s.turns.clone())
            .unwrap_or_default();
        Ok(json!({"ok":true,"git_root":root,"repo_name":root.file_name(),"turns":turns}))
    }
    pub(super) fn select_turn(
        &self,
        core: &dyn SessionCore,
        sid: i64,
        turn: i64,
    ) -> Result<GitTurnSnapshot> {
        let details = core
            .details(LiveSessionId(sid))
            .ok_or_else(|| err(400, "bad_session", "session not found"))?;
        let state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        state
            .sessions
            .get(&sid)
            .filter(|s| s.binding.incarnation == details.binding.incarnation)
            .and_then(|s| s.turns.iter().find(|s| s.turn == turn))
            .cloned()
            .ok_or_else(|| err(404, "not_found", "turn snapshot not found"))
    }
    pub(super) async fn start_ai_commit(
        &self,
        core: &dyn SessionCore,
        sid: i64,
        language: &str,
    ) -> Result<Response> {
        let details = core
            .details(LiveSessionId(sid))
            .ok_or_else(|| err(404, "session_not_found", "session not found"))?;
        if details.snapshot.provider == "shell" || details.snapshot.provider.is_empty() {
            return Err(err(400, "not_ai_session", "session is not an AI agent"));
        }
        if !details.connected {
            return Err(err(409, "no_wrapper", "AI session is not connected"));
        }
        let now = Timestamp::now();
        let binding = details.binding;
        let ja = language.is_empty() || language.eq_ignore_ascii_case("ja");
        {
            let mut state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
            state.awaiting.insert(
                sid,
                AwaitCommit {
                    binding,
                    deadline: now + Duration::from_secs(180),
                    language: language.into(),
                    buffer: String::new(),
                    progressed: false,
                },
            );
        }
        let request = InputRequest {
            bytes: format!("\x1b[200~{}\x1b[201~\r", ai_prompt(ja)).into_bytes(),
            authority: InputAuthority::Internal,
        };
        let cancel = TaskCancellation::default();
        let receipt = core.submit(binding, request, now, &cancel).await;
        if !matches!(
            receipt.disposition,
            InputDisposition::TransportWritten { .. } | InputDisposition::Deferred { .. }
        ) {
            self.turns
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .awaiting
                .remove(&sid);
            return Err(err(
                409,
                "no_wrapper",
                "AI session input could not be delivered",
            ));
        }
        Ok(Response::json(
            200,
            &json!({"ok":true,"subject":"","body":"","pending":true}),
        ))
    }
    /// The Hub passes ANSI-stripped provider output. Returned messages must be
    /// broadcast by the same ordered Hub output lane, after binding validation.
    pub fn observe_commit_output(
        &self,
        core: &dyn SessionCore,
        binding: SessionBinding,
        clean_text: &str,
        now: Timestamp,
    ) -> Vec<crate::proto::Message> {
        if !core.is_current(binding) {
            return vec![];
        }
        let mut state = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        let Some(p) = state.awaiting.get_mut(&binding.session.0) else {
            return vec![];
        };
        if p.binding.incarnation != binding.incarnation {
            return vec![];
        }
        if now > p.deadline {
            let ja = p.language.is_empty() || p.language.eq_ignore_ascii_case("ja");
            state.awaiting.remove(&binding.session.0);
            return vec![crate::proto::Message {
                r#type: "commit_msg_error".into(),
                session_id: binding.session.0,
                reason: if ja {
                    "AI からの応答がタイムアウトしました。"
                } else {
                    "Timed out waiting for the AI response."
                }
                .into(),
                ..Default::default()
            }];
        }
        p.buffer.push_str(clean_text);
        if p.buffer.len() > 64 * 1024 {
            let mut start = p.buffer.len() - 64 * 1024;
            while !p.buffer.is_char_boundary(start) {
                start += 1;
            }
            p.buffer.drain(..start);
        }
        let progressed = !p.progressed && !clean_text.trim().is_empty();
        p.progressed |= progressed;
        if let Some((subject, body)) = extract_markers(&p.buffer) {
            state.awaiting.remove(&binding.session.0);
            return vec![crate::proto::Message {
                r#type: "commit_msg_suggested".into(),
                session_id: binding.session.0,
                commit_subject: sanitize_message(&mask_reply(&subject), 200),
                commit_body: sanitize_message(&mask_reply(&body), 8192),
                ..Default::default()
            }];
        }
        if progressed {
            vec![crate::proto::Message {
                r#type: "commit_msg_progress".into(),
                session_id: binding.session.0,
                ..Default::default()
            }]
        } else {
            vec![]
        }
    }
}
// Git for Windows interprets GIT_INDEX_FILE itself; the canonical Win32
// verbatim drive spelling is not an accepted Git index filename. Convert only
// a held directory's spelling, and reject any conversion that resolves to a
// different directory before Git receives it.
fn git_index_path(dir: &super::super::safe_fs::Dir, name: &str) -> std::io::Result<PathBuf> {
    super::super::safe_fs::basename(name)?;
    #[cfg(windows)]
    {
        use std::{
            ffi::OsString,
            path::{Component, Prefix},
        };
        let mut components = dir.path().components();
        let prefix = match components.next() {
            Some(Component::Prefix(prefix)) => prefix.kind(),
            _ => {
                return Err(std::io::Error::other(
                    "Git index directory has no Windows prefix",
                ));
            }
        };
        let mut plain = OsString::new();
        match prefix {
            Prefix::VerbatimDisk(drive) => plain.push(format!("{}:", char::from(drive))),
            Prefix::VerbatimUNC(server, share) => {
                plain.push(r"\\");
                plain.push(server);
                plain.push(r"\");
                plain.push(share);
            }
            Prefix::Disk(_) | Prefix::UNC(_, _) => return Ok(dir.path().join(name)),
            _ => {
                return Err(std::io::Error::other(
                    "Git cannot represent this index directory",
                ));
            }
        }
        let mut plain = PathBuf::from(plain);
        for component in components {
            plain.push(component.as_os_str());
        }
        // In particular, never silently normalize a verbatim-only trailing
        // dot/space component into a different ordinary Win32 directory.
        if std::fs::canonicalize(&plain)? != std::fs::canonicalize(dir.path())? {
            return Err(std::io::Error::other(
                "Git index path changes directory identity",
            ));
        }
        Ok(plain.join(name))
    }
    #[cfg(not(windows))]
    Ok(dir.path().join(name))
}

impl Git<'_> {
    pub(super) async fn worktree_tree(&self, root: &Path) -> Result<String> {
        let base = super::super::safe_fs::Dir::open(self.service.paths.root())
            .map_err(|e| super::super::io_error(e, "snapshot_failed"))?;
        let temporary = base
            .child_dir("tmp", true)
            .map_err(|e| super::super::io_error(e, "snapshot_failed"))?;
        let name = format!(
            "many-ai-cli-git-turn-{}",
            crate::process::random_token()
                .map_err(|e| super::super::io_error(e, "snapshot_failed"))?
        );
        let dir = temporary
            .child_dir(&name, true)
            .map_err(|e| super::super::io_error(e, "snapshot_failed"))?;
        let result = async {
            let index = git_index_path(&dir, "index")
                .map_err(|_| "Git cannot represent the private index path".to_owned())?;
            let env = BTreeMap::from([("GIT_INDEX_FILE".into(), Some(index.as_os_str().into()))]);
            let head = self
                .run(root, &["rev-parse", "--verify", "HEAD^{tree}"])
                .await
                .is_ok();
            self.env(
                root,
                &if head {
                    vec!["read-tree", "HEAD"]
                } else {
                    vec!["read-tree", "--empty"]
                },
                env.clone(),
                false,
            )
            .await?;
            self.env(root, &["add", "-A", "--", "."], env.clone(), false)
                .await?;
            let out = self.env(root, &["write-tree"], env, false).await?;
            let tree = out.trim();
            if !valid_revision(tree) {
                return Err("git write-tree returned an invalid object id".into());
            }
            Ok(tree.to_owned())
        }
        .await;
        drop(dir);
        let cleanup = temporary.remove_tree(&name);
        if let Err(e) = cleanup {
            return Err(super::super::io_error(e, "snapshot_cleanup_failed"));
        }
        result.map_err(command_error)
    }
}
const OPEN: &str = "[MANY-AI-CLI-COMMIT]";
const CLOSE: &str = "[/MANY-AI-CLI-COMMIT]";
pub fn ai_prompt(ja: bool) -> String {
    if ja {
        format!(
            "[many-ai-cli] このリポジトリの未コミットの変更について、`git --no-pager diff HEAD` と `git status --short` で内容を確認し、Conventional Commits 形式（feat/fix/refactor/docs/test/chore など）のコミットメッセージを 1 つ提案してください。前置きや説明は一切付けず、{OPEN} を単独行で出力し、その次の行に subject（1 行）、必要なら空行を挟んで本文（複数行可）、最後に {CLOSE} を単独行で出力してください。各マーカー行は行頭・装飾なし・コードブロックで囲まないこと。"
        )
    } else {
        format!(
            "[many-ai-cli] For the uncommitted changes in this repository, inspect them with `git --no-pager diff HEAD` and `git status --short`, then propose one commit message in Conventional Commits form (feat/fix/refactor/docs/test/chore, etc.). Do not add any preamble or explanation: output {OPEN} on its own line, then the subject on the next line, optionally a blank line and a body (multiple lines allowed), and finally {CLOSE} on its own line. Keep each marker line at the start of the line, undecorated, and not wrapped in a code block."
        )
    }
}
pub fn extract_markers(buffer: &str) -> Option<(String, String)> {
    let lines: Vec<_> = buffer
        .lines()
        .map(|l| {
            l.trim_end_matches('\r').trim_start_matches([
                '│', '┃', '┆', '┊', '╎', '▏', '|', '⏺', '●', '•', '◦', '·', ' ', '\t',
            ])
        })
        .collect();
    let start = lines.iter().rposition(|l| l.trim().starts_with(OPEN))?;
    let remainder = lines[start].trim().strip_prefix(OPEN)?.trim();
    if let Some((head, _)) = remainder.split_once(CLOSE) {
        return if head.trim().is_empty() {
            None
        } else {
            Some((head.trim().into(), String::new()))
        };
    }
    let mut inner = vec![];
    if !remainder.is_empty() {
        inner.push(remainder);
    }
    let mut closed = false;
    for line in &lines[start + 1..] {
        if let Some((leading, _)) = line.split_once(CLOSE) {
            if !leading.trim().is_empty() {
                inner.push(leading.trim());
            }
            closed = true;
            break;
        }
        inner.push(line);
    }
    if !closed {
        return None;
    }
    while inner.first().is_some_and(|s| s.trim().is_empty()) {
        inner.remove(0);
    }
    while inner.last().is_some_and(|s| s.trim().is_empty()) {
        inner.pop();
    }
    let subject = inner.first()?.trim();
    if subject.is_empty() {
        return None;
    }
    Some((subject.into(), inner[1..].join("\n").trim().into()))
}
fn mask_reply(value: &str) -> String {
    crate::storage::mask_secrets(value)
}
