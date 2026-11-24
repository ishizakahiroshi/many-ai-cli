//! Outbound, owner-decided answers to exact live yes/no markers. The journal
//! contains only opaque request and routing identifiers and the outcome to
//! report after a restart. It never restores authority to write to a terminal.
use super::*;
use crate::{
    approval::identity::digest,
    config::private_io,
    terminal::session::{
        SessionEngine,
        iris_confirmation::{IrisAnswerOutcome, iris_confirmation_question},
    },
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, sync::Weak};

const POLL: Duration = Duration::from_secs(5);
const LIMIT: usize = 256;
// The worker retains terminal receipts for seven days after a pending/
// approved entry expires. Do not keep unreachable device receipts forever.
const RETAIN_SECS: u64 = 9 * 24 * 60 * 60;
fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
const JOURNAL: &str = "iris-confirmation-receipts.json";

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Outcome {
    Expired,
    Unknown,
    Applied,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    destination: String,
    outcome: Outcome,
    created_at: u64,
}

type Receipts = BTreeMap<String, Receipt>;

struct LiveRequest {
    id: String,
    destination: String,
    binding: SessionBinding,
    record: ImmutableApprovalRecord,
    payload: Value,
    registered: bool,
    finished: bool,
}
impl LiveRequest {
    fn still_permitted(&self, board: &BoardSync, core: &SessionEngine) -> bool {
        let Ok(Some(config)) = load(&board.file) else {
            return false;
        };
        core.details(self.binding.session)
            .and_then(|d| source(&config, Path::new(&d.snapshot.cwd)))
            .is_some_and(|(dst, _)| dst == self.destination)
    }
    fn current(&self, core: &SessionEngine) -> bool {
        let record = self.record.data();
        core.iris_confirmation_is_current(
            self.binding,
            &record.candidate,
            &record.block,
            record.detected_at,
        )
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn load_receipts(path: &Path) -> io::Result<Receipts> {
    let mut file = match std::fs::File::open(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Receipts::new()),
        opened => opened?,
    };
    if file.metadata()?.len() > 128 * 1024 {
        return Err(io::Error::other("receipt journal too large"));
    }
    let receipts: Receipts = serde_json::from_reader(&mut file)?;
    if receipts.len() > LIMIT || receipts.keys().any(|id| !valid_id(id)) {
        return Err(io::Error::other("invalid confirmation receipt"));
    }
    Ok(receipts)
}

fn save(path: &Path, receipts: &Receipts) -> io::Result<()> {
    private_io::write_atomic(path, &serde_json::to_vec(receipts)?)
}

fn route(config: &FileConfig, repo: &RepoEntry) -> Option<(String, Target)> {
    let key_file = repo.confirmation_key_file.as_ref()?;
    if !key_file.is_absolute() || key_file == &config.key_file {
        return None;
    }
    let project = project_of(repo);
    if !shareable(&project, 80) {
        return None;
    }
    let device_key = read_key(key_file)?;
    if device_key == read_key(&config.key_file)? {
        return None;
    }
    let credential = digest(&device_key);
    let destination = digest(&serde_json::to_string(&(&config.url, &credential, &project)).ok()?);
    Some((
        destination,
        Target {
            url: config.url.clone(),
            key_file: key_file.clone(),
            project,
            item_id: String::new(),
            fence: None,
        },
    ))
}

fn source(config: &FileConfig, cwd: &Path) -> Option<(String, Target)> {
    if !cwd.is_absolute()
        || cwd
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    {
        return None;
    }
    let cwd = cwd.canonicalize().ok()?;
    // Configured and current directories can be junctions or symlinks. Match
    // their actual locations; most-specific entries also express opt-out.
    let (_, repo) = config
        .repos
        .iter()
        .filter_map(|repo| {
            let root = repo.path.canonicalize().ok()?;
            cwd.starts_with(&root)
                .then(|| (root.components().count(), repo))
        })
        .max_by_key(|(depth, _)| *depth)?;
    route(config, repo)
}

fn text_ok(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.encode_utf16().count() <= max
        && !value.chars().any(char::is_control)
        && mask_secrets(value) == value
}

fn shareable(value: &str, max: usize) -> bool {
    text_ok(value, max)
        && !value.contains(['/', '\\', '`', '@'])
        && !value
            .split_whitespace()
            .any(|word| word.len() > 128 && word.is_ascii())
}

fn make_payload(
    id: &str,
    session: LiveSessionId,
    agent: &str,
    label: &str,
    target: &Target,
    question: &str,
) -> Value {
    let task = if shareable(label, 160) {
        label.to_owned()
    } else {
        format!("Session {}", session.0)
    };
    let share = shareable(question, 1000) && shareable(agent, 80);
    json!({
        "request_id": id, "project": target.project, "task": task,
        "agent": if shareable(agent, 80) { agent } else { "AI" },
        "question": if share { question } else { "PCで確認してください" },
        "local_only": !share,
    })
}

fn response_state<'a>(value: &'a Value, id: &str) -> Option<&'a str> {
    (value["ok"] == true && value["request_id"].as_str() == Some(id))
        .then(|| value["state"].as_str())
        .flatten()
}

struct SyncRun {
    path: PathBuf,
    receipts: Receipts,
    live: Vec<LiveRequest>,
}
impl SyncRun {
    fn new(path: PathBuf) -> io::Result<Self> {
        Ok(Self {
            receipts: load_receipts(&path)?,
            path,
            live: Vec::new(),
        })
    }

    async fn tick(
        &mut self,
        board: &BoardSync,
        core: &SessionEngine,
        cancel: &TaskCancellation,
    ) -> io::Result<()> {
        let Ok(Some(config)) = load(&board.file) else {
            self.live.clear();
            return Ok(());
        };
        let destinations: HashMap<_, _> = config
            .repos
            .iter()
            .filter_map(|repo| route(&config, repo))
            .collect();
        self.live.retain(|entry| {
            entry.current(core)
                && core
                    .details(entry.binding.session)
                    .and_then(|details| source(&config, Path::new(&details.snapshot.cwd)))
                    .is_some_and(|(dst, _)| dst == entry.destination)
        });
        self.close_retired(board, &destinations).await?;
        self.collect(&config, core, &destinations)?;
        self.sync_live(board, core, &destinations, cancel).await
    }

    async fn close_retired(
        &mut self,
        board: &BoardSync,
        destinations: &HashMap<String, Target>,
    ) -> io::Result<()> {
        // A received decision cannot survive losing its in-memory live binding.
        // On restart finish only the acknowledgement, never the old submission.
        let retired: Vec<_> = self
            .receipts
            .iter()
            .filter(|(id, _)| !self.live.iter().any(|req| req.id == **id && !req.finished))
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect();
        let now = unix_secs();
        for (id, receipt) in retired {
            if now.saturating_sub(receipt.created_at) >= RETAIN_SECS {
                self.receipts.remove(&id);
                save(&self.path, &self.receipts)?;
                continue;
            }
            let Some(target) = destinations.get(&receipt.destination) else {
                continue;
            };
            let args = json!({"request_id":id,"outcome":receipt.outcome});
            let Ok(mut reply) = board
                .structured_reply(target, "iris_close_confirmation", args)
                .await
            else {
                continue;
            };
            if reply["ok"] == false && reply["error"]["code"] == "CONFIRMATION_CONFLICT" {
                let Ok(current) = board
                    .structured_reply(target, "iris_confirmation_status", json!({"request_id":id}))
                    .await
                else {
                    continue;
                };
                reply = current;
            }
            if response_state(&reply, &id)
                .is_some_and(|s| matches!(s, "expired" | "applied" | "unknown"))
                || reply["ok"] == false && reply["error"]["code"] == "NOT_FOUND"
            {
                self.receipts.remove(&id);
                save(&self.path, &self.receipts)?;
            }
        }
        Ok(())
    }

    // Rotating or removing an unreachable credential must not exhaust the
    // inbox for other devices. Evicted closures carry no authority to apply.
    fn reserve_receipt_slot(&mut self, destinations: &HashMap<String, Target>) -> bool {
        if self.receipts.len() < LIMIT {
            return true;
        }
        let oldest = self
            .receipts
            .iter()
            .filter(|(id, _)| {
                !self
                    .live
                    .iter()
                    .any(|req| req.id == id.as_str() && !req.finished)
            })
            .min_by_key(|(_, r)| (destinations.contains_key(&r.destination), r.created_at))
            .map(|(id, _)| id.clone());
        if let Some(id) = oldest {
            self.receipts.remove(&id);
            true
        } else {
            false
        }
    }

    fn collect(
        &mut self,
        config: &FileConfig,
        core: &SessionEngine,
        destinations: &HashMap<String, Target>,
    ) -> io::Result<()> {
        for session in core.active_ids() {
            if self.live.len() >= LIMIT {
                break;
            }
            let Some(details) = core.details(session) else {
                continue;
            };
            let Some(record) = details.approval.record.as_ref() else {
                continue;
            };
            let Some(question) = iris_confirmation_question(record.data()) else {
                continue;
            };
            let cwd = Path::new(&details.snapshot.cwd);
            let data = record.data();
            if !core.iris_confirmation_is_current(
                details.binding,
                &data.candidate,
                &data.block,
                data.detected_at,
            ) {
                continue;
            }
            let Some((destination, target)) = source(config, cwd) else {
                continue;
            };
            if self.live.iter().any(|r| {
                r.binding == details.binding
                    && r.destination == destination
                    && r.record.data().candidate.same_candidate(&data.candidate)
                    && r.record.data().detected_at == data.detected_at
                    && r.record.data().block == data.block
            }) {
                continue;
            }
            if !self.reserve_receipt_slot(destinations) {
                break;
            }
            let mut bytes = [0u8; 16];
            getrandom::fill(&mut bytes).map_err(|_| io::Error::other("random id unavailable"))?;
            let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            self.receipts.insert(
                id.clone(),
                Receipt {
                    destination: destination.clone(),
                    outcome: Outcome::Expired,
                    created_at: unix_secs(),
                },
            );
            save(&self.path, &self.receipts)?;
            self.live.push(LiveRequest {
                payload: make_payload(
                    &id,
                    details.binding.session,
                    &details.snapshot.provider,
                    &details.snapshot.label,
                    &target,
                    &question,
                ),
                id,
                destination,
                binding: details.binding,
                record: record.clone(),
                registered: false,
                finished: false,
            });
        }
        Ok(())
    }

    async fn sync_live(
        &mut self,
        board: &BoardSync,
        core: &SessionEngine,
        destinations: &HashMap<String, Target>,
        cancel: &TaskCancellation,
    ) -> io::Result<()> {
        for entry in &mut self.live {
            if entry.finished || !entry.current(core) {
                continue;
            }
            let Some(target) = destinations.get(&entry.destination) else {
                continue;
            };
            // Each preceding request may have waited on the network. Recheck
            // disclosure consent at the point of sending this question.
            if !entry.still_permitted(board, core) {
                entry.finished = true;
                continue;
            }
            let (tool, args) = if entry.registered {
                ("iris_confirmation_status", json!({"request_id":entry.id}))
            } else {
                ("iris_request_confirmation", entry.payload.clone())
            };
            let Ok(reply) = board.structured_reply(target, tool, args).await else {
                continue;
            };
            let Some(state) = response_state(&reply, &entry.id) else {
                if entry.registered && reply["ok"] == false && reply["error"]["code"] == "NOT_FOUND"
                {
                    entry.finished = true;
                }
                continue;
            };
            entry.registered = true;
            match state {
                "pending" => continue,
                "approved" if entry.payload["local_only"] == false => {}
                "expired" | "applied" | "unknown" => {
                    entry.finished = true;
                    self.receipts.remove(&entry.id);
                    save(&self.path, &self.receipts)?;
                    continue;
                }
                _ => continue,
            }
            // Recheck opt-in after the slow network call, and persist ambiguity
            // before *any* PTY write. A crash in either direction cannot replay.
            if !entry.still_permitted(board, core) || !entry.current(core) {
                entry.finished = true;
                continue;
            }
            let receipt = self
                .receipts
                .get_mut(&entry.id)
                .ok_or_else(|| io::Error::other("missing receipt"))?;
            receipt.outcome = Outcome::Unknown;
            save(&self.path, &self.receipts)?;
            let data = entry.record.data();
            let outcome = core
                .apply_iris_confirmation(
                    entry.binding,
                    &data.candidate,
                    &data.block,
                    data.detected_at,
                    cancel,
                )
                .await;
            self.receipts.get_mut(&entry.id).unwrap().outcome = match outcome {
                IrisAnswerOutcome::Applied => Outcome::Applied,
                IrisAnswerOutcome::Expired => Outcome::Expired,
                IrisAnswerOutcome::Unknown => Outcome::Unknown,
            };
            entry.finished = true;
            save(&self.path, &self.receipts)?;
        }
        Ok(())
    }
}

impl BoardSync {
    pub fn start_confirmations(self: &Arc<Self>, core: Weak<SessionEngine>) {
        if !self.enabled {
            return;
        }
        let Ok(permit) = self.tasks.effect_permit() else {
            return;
        };
        let cancel = permit.cancellation();
        let owner = self.clone();
        let _receipt = permit.start(async move {
            let Ok(mut run) = SyncRun::new(owner.root().join(JOURNAL)) else {
                owner.warn_once("confirmation receipts unreadable; remote answers are paused");
                return;
            };
            loop {
                let Some(engine) = core.upgrade() else { return };
                let check = run.tick(&owner,&engine,&cancel);
                tokio::select! {
                    _ = cancel.token().cancelled() => return,
                    result = check => if result.is_err() {
                        owner.warn_once("confirmation receipts unavailable; remote answers are paused");
                        return;
                    }
                }
                drop(engine);
                tokio::select! {
                    _ = cancel.token().cancelled() => return,
                    _ = tokio::time::sleep(POLL) => {}
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_route_requires_repo_opt_in_and_dedicated_credential() {
        let root = tempfile::tempdir().unwrap();
        let outer = root.path().join("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        let key = root.path().join("device.key");
        let writer = root.path().join("writer.key");
        std::fs::write(&key, "synthetic-device").unwrap();
        std::fs::write(&writer, "fixture-ordinary").unwrap();
        let mut cfg: FileConfig = serde_json::from_value(json!({"schema_version":1,
        "url":"https://example.invalid/board/mcp","key_file":writer,"repos":[
            {"path":outer,"project":"Demo","confirmation_key_file":key},
            {"path":inner,"project":"Local"}
        ]}))
        .unwrap();
        assert!(source(&cfg, &inner).is_none());
        assert!(source(&cfg, &inner.join("..")).is_none());
        let sibling = root.path().join("private");
        std::fs::create_dir(&sibling).unwrap();
        assert!(source(&cfg, &outer.join("../private")).is_none());
        #[cfg(unix)]
        {
            let link = outer.join("indirect");
            std::os::unix::fs::symlink(&sibling, &link).unwrap();
            assert!(source(&cfg, &link).is_none());
        }
        assert!(source(&cfg, &root.path().join("outer-other")).is_none());
        let (before, _) = source(&cfg, &outer).unwrap();
        std::fs::write(&key, "second-synthetic-device").unwrap();
        assert_ne!(source(&cfg, &outer).unwrap().0, before);
        std::fs::write(&key, "fixture-ordinary").unwrap();
        assert!(source(&cfg, &outer).is_none());
        cfg.repos[0].confirmation_key_file = Some(writer);
        assert!(source(&cfg, &outer).is_none());
    }

    #[test]
    fn confirmation_local_only_hides_sensitive_and_overlong_questions() {
        let target = Target {
            url: String::new(),
            key_file: PathBuf::new(),
            project: "Demo".into(),
            item_id: String::new(),
            fence: None,
        };
        for question in [
            "Read C:/private/notes?",
            &"x".repeat(1001),
            "Use API_KEY=example-value?",
        ] {
            let payload = make_payload(
                "id",
                LiveSessionId(7),
                "agent",
                "C:/private/task",
                &target,
                question,
            );
            assert_eq!(payload["local_only"], true);
            assert_eq!(payload["task"], "Session 7");
            assert_ne!(payload["question"], question);
        }
        assert_eq!(
            make_payload(
                "id",
                LiveSessionId(7),
                "agent",
                "Edit article",
                &target,
                "Read the draft?"
            )["task"],
            "Edit article"
        );
    }

    #[test]
    fn confirmation_restart_loads_only_receipts_without_live_authority() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(JOURNAL);
        let id = "a".repeat(32);
        let receipts = Receipts::from([(
            id.clone(),
            Receipt {
                destination: "device".into(),
                outcome: Outcome::Unknown,
                created_at: unix_secs(),
            },
        )]);
        save(&path, &receipts).unwrap();
        let run = SyncRun::new(path.clone()).unwrap();
        assert!(run.live.is_empty());
        assert!(matches!(run.receipts[&id].outcome, Outcome::Unknown));
        let bad = Receipts::from([(
            "invalid".into(),
            Receipt {
                destination: "device".into(),
                outcome: Outcome::Expired,
                created_at: unix_secs(),
            },
        )]);
        save(&path, &bad).unwrap();
        assert!(SyncRun::new(path).is_err());
    }

    #[test]
    fn confirmation_rotation_reclaims_unreachable_receipts_at_capacity() {
        let root = tempfile::tempdir().unwrap();
        let mut run = SyncRun::new(root.path().join(JOURNAL)).unwrap();
        for i in 0..LIMIT {
            run.receipts.insert(
                format!("{i:032x}"),
                Receipt {
                    destination: "old".into(),
                    outcome: Outcome::Unknown,
                    created_at: i as u64,
                },
            );
        }
        assert!(run.reserve_receipt_slot(&HashMap::new()));
        assert_eq!(run.receipts.len(), LIMIT - 1);
        assert!(!run.receipts.contains_key(&format!("{:032x}", 0)));
    }

    #[test]
    fn confirmation_status_must_identify_the_requested_question() {
        assert_eq!(
            response_state(
                &json!({"ok":true,"request_id":"id","state":"approved"}),
                "id"
            ),
            Some("approved")
        );
        for reply in [
            json!({"ok":false,"request_id":"id","state":"approved"}),
            json!({"ok":true,"request_id":"another","state":"approved"}),
            json!({"state":"approved"}),
        ] {
            assert!(response_state(&reply, "id").is_none());
        }
    }
}
