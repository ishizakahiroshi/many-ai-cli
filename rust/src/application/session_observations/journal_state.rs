//! Source journal discovery and retained timers. Only event IDs/counters live
//! here; result bodies never enter an allocation or a published observation.
use super::{
    journal::{self, FileState},
    subagents,
};
use crate::{
    config::RuntimePaths,
    proto::{WorkflowProgress, time::Timestamp},
};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    time::Duration,
};
type Files = BTreeMap<PathBuf, FileState>;
struct Group {
    directory: PathBuf,
    files: Files,
    started: i64,
    done: i64,
}
fn counts(files: &Files) -> (i64, i64, Option<Timestamp>, Option<Timestamp>) {
    let mut started = 0;
    let mut done = 0;
    let mut event = None;
    let mut modified = None;
    for state in files.values() {
        started += state.started.len() as i64;
        done += state.results.len() as i64;
        event = event.max(
            state
                .last_event
                .and_then(|v| Timestamp::from_system_time(v).ok()),
        );
        modified = modified.max(
            state
                .modified
                .and_then(|v| Timestamp::from_system_time(v).ok()),
        );
    }
    (started, done, event, modified)
}
fn scan(
    paths: &RuntimePaths,
    directory: &Path,
    detected: Timestamp,
    existing: &Files,
) -> io::Result<Files> {
    let workflow = directory.join("subagents/workflows");
    let entries = subagents::entries(paths, &workflow)?;
    let mut files = existing.clone();
    let cutoff = detected
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(detected);
    for entry in entries {
        if !entry.is_dir || !entry.name.starts_with("wf_") {
            continue;
        }
        let path = workflow.join(entry.name).join("journal.jsonl");
        let Ok(file) = subagents::open_artifact(paths, &path) else {
            continue;
        };
        let Ok(metadata) = file.metadata() else {
            continue;
        };
        let Ok(modified) = metadata
            .modified()
            .and_then(|v| Timestamp::from_system_time(v).map_err(io::Error::other))
        else {
            continue;
        };
        let known = files.contains_key(&path);
        if !known && modified < cutoff {
            continue;
        }
        let prior = files.get(&path).cloned().unwrap_or_default();
        let Ok((state, _)) = journal::tail_opened(file, prior, &journal::Budget::default()) else {
            continue;
        };
        if !known
            && !state.started.is_empty()
            && state.results.len() == state.started.len()
            && modified < detected
        {
            continue;
        }
        if !state.started.is_empty() || known {
            files.insert(path, state);
        }
    }
    Ok(files)
}
fn discover(paths: &RuntimePaths, project: &Path, detected: Timestamp) -> io::Result<Vec<Group>> {
    let mut groups = Vec::new();
    for entry in subagents::entries(paths, project)? {
        if !entry.is_dir {
            continue;
        }
        let directory = project.join(entry.name);
        let Ok(files) = scan(paths, &directory, detected, &Files::new()) else {
            continue;
        };
        let (started, done, _, _) = counts(&files);
        if started > 0 {
            groups.push(Group {
                directory,
                files,
                started,
                done,
            });
        }
    }
    Ok(groups)
}
fn select(mut groups: Vec<Group>, vt_done: i64) -> Option<Group> {
    if groups.len() == 1 {
        return groups.pop();
    }
    let mut candidates = groups
        .into_iter()
        .filter(|g| g.done <= vt_done && vt_done <= g.started);
    let candidate = candidates.next()?;
    candidates.next().is_none().then_some(candidate)
}
#[derive(Default)]
pub struct Watcher {
    pub progress: Option<WorkflowProgress>,
    pub pending: bool,
    pub due: Option<Timestamp>,
    pub directory: Option<PathBuf>,
    pub new_run: Option<PathBuf>,
    files: Files,
    detected: Option<Timestamp>,
    running: bool,
    dormant: bool,
    dormant_signature: String,
    settled_signature: String,
    last_modified: Option<Timestamp>,
}
pub struct PollContext<'a> {
    pub vt: Option<&'a WorkflowProgress>,
    pub has_signal: bool,
    pub signature: &'a str,
    pub idle: bool,
    pub now: Timestamp,
}
impl Watcher {
    pub fn start(&mut self, now: Timestamp, signature: &str, idle: bool) {
        if self.dormant && idle && self.dormant_signature == signature {
            return;
        }
        if self.progress.as_ref().is_some_and(|p| p.settled) {
            if self.settled_signature == signature {
                return;
            }
            *self = Self::default();
        }
        if self.detected.is_none()
            || self.files.is_empty() && self.directory.is_none() && !self.running
        {
            self.detected = Some(now);
            self.pending = true;
        }
        if !self.running {
            self.due = Some(now);
        }
        self.running = true;
        self.dormant = false;
        self.dormant_signature.clear();
    }
    pub fn poll(
        &mut self,
        paths: &RuntimePaths,
        project: &Path,
        context: PollContext<'_>,
    ) -> io::Result<bool> {
        let PollContext {
            vt,
            has_signal,
            signature,
            idle,
            now,
        } = context;
        if self.due.is_none_or(|due| due > now) {
            return Ok(false);
        }
        self.due = None;
        let Some(detected) = self.detected else {
            return Ok(false);
        };
        let group = if let Some(directory) = &self.directory {
            scan(paths, directory, detected, &self.files)
                .ok()
                .map(|files| {
                    let (started, done, _, _) = counts(&files);
                    Group {
                        directory: directory.clone(),
                        files,
                        started,
                        done,
                    }
                })
        } else {
            discover(paths, project, detected)
                .ok()
                .and_then(|groups| select(groups, vt.map_or(0, |p| p.done)))
        };
        self.pending = false;
        if let Some(group) = group {
            self.new_run = group
                .files
                .iter()
                .filter(|(path, _)| !self.files.contains_key(*path))
                .max_by(|(a, sa), (b, sb)| sa.modified.cmp(&sb.modified).then_with(|| a.cmp(b)))
                .and_then(|(path, _)| path.parent().map(Path::to_path_buf));
            self.directory = Some(group.directory);
            self.files = group.files;
            let (started, done, _, modified) = counts(&self.files);
            self.last_modified = modified;
            let incomplete = has_signal
                && vt.is_some_and(|p| {
                    !p.settled
                        && (p.waiting_dynamic > 0
                            || p.running > 0
                            || p.pending > 0
                            || p.total > 0 && p.done < p.total)
                });
            let quiet = modified.and_then(|at| now.duration_since(at).ok());
            let settled_by = if started > 0
                && done == started
                && !incomplete
                && quiet.is_some_and(|age| age >= Duration::from_secs(10))
            {
                "journal"
            } else if started > done
                && (!has_signal || vt.is_some_and(|p| p.settled))
                && quiet.is_some_and(|age| age >= Duration::from_secs(300))
            {
                "timeout"
            } else {
                ""
            };
            self.progress = Some(WorkflowProgress {
                detected: true,
                total: started,
                done,
                settled: !settled_by.is_empty(),
                settled_by: settled_by.into(),
                ..Default::default()
            });
            if !settled_by.is_empty() {
                self.settled_signature = signature.into();
            }
        }
        if self.progress.as_ref().or(vt).is_some_and(|p| p.settled) {
            self.running = false;
            self.dormant = false;
            self.dormant_signature.clear();
        } else {
            let activity = self.last_modified.unwrap_or(detected);
            if idle
                && now
                    .duration_since(activity)
                    .is_ok_and(|age| age >= Duration::from_secs(60))
            {
                self.running = false;
                self.dormant = true;
                self.dormant_signature = signature.into();
                if self.progress.as_ref().is_some_and(|p| p.total > p.done) {
                    self.due = Some(
                        activity
                            .checked_add(Duration::from_secs(300))
                            .unwrap_or(now)
                            .max(now.checked_add(Duration::from_secs(1)).unwrap_or(now)),
                    );
                }
            } else {
                self.running = true;
                self.dormant = false;
                self.dormant_signature.clear();
                self.due = now.checked_add(Duration::from_secs(1));
            }
        }
        Ok(true)
    }
    pub fn disable(&mut self) {
        *self = Self::default();
    }
    pub fn finalize(&mut self) {
        self.due = None;
        self.running = false;
        self.dormant = false;
        self.dormant_signature.clear();
        self.pending = false;
        if let Some(progress) = self.progress.as_mut()
            && !progress.settled
        {
            progress.settled = true;
            progress.settled_by = "timeout".into();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn group(name: &str, started: i64, done: i64) -> Group {
        Group {
            directory: name.into(),
            files: Files::new(),
            started,
            done,
        }
    }
    #[test]
    fn multiple_sessions_are_not_associated_when_the_counter_match_is_ambiguous() {
        assert!(select(vec![group("a", 3, 1), group("b", 4, 2)], 2).is_none());
        assert_eq!(
            select(vec![group("a", 3, 1), group("b", 4, 4)], 2)
                .unwrap()
                .directory,
            PathBuf::from("a")
        );
        assert_eq!(
            select(vec![group("a", 3, 3)], 0).unwrap().directory,
            PathBuf::from("a")
        );
    }
    #[test]
    fn live_vt_blocks_quiet_completed_journal_then_disappearance_releases_settle() {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49326, installed.path()).unwrap();
        let project = root.path().join("projects/synthetic");
        let directory = project.join("session/subagents/workflows/wf_run");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("journal.jsonl"),b"{\"type\":\"started\",\"agentId\":\"a\"}\n{\"type\":\"result\",\"agentId\":\"a\",\"result\":\"SYNTHETIC_BODY_NEVER_RETAINED\"}\n").unwrap();
        let now = Timestamp::now();
        std::fs::File::options()
            .write(true)
            .open(directory.join("journal.jsonl"))
            .unwrap()
            .set_modified(
                now.checked_add(Duration::from_secs(1))
                    .unwrap()
                    .to_system_time_exact()
                    .unwrap(),
            )
            .unwrap();
        let mut watcher = Watcher::default();
        watcher.start(now, "running", false);
        let vt = WorkflowProgress {
            detected: true,
            total: 2,
            done: 1,
            running: 1,
            ..Default::default()
        };
        let later = now.checked_add(Duration::from_secs(11)).unwrap();
        watcher
            .poll(
                &paths,
                &project,
                PollContext {
                    vt: Some(&vt),
                    has_signal: true,
                    signature: "running",
                    idle: true,
                    now: later,
                },
            )
            .unwrap();
        assert_eq!(watcher.progress.as_ref().unwrap().done, 1);
        assert!(!watcher.progress.as_ref().unwrap().settled);
        assert_eq!(watcher.new_run.as_deref(), Some(directory.as_path()));
        let later = later.checked_add(Duration::from_secs(1)).unwrap();
        watcher
            .poll(
                &paths,
                &project,
                PollContext {
                    vt: None,
                    has_signal: false,
                    signature: "running",
                    idle: true,
                    now: later,
                },
            )
            .unwrap();
        assert!(watcher.progress.as_ref().unwrap().settled);
        assert_eq!(watcher.progress.as_ref().unwrap().settled_by, "journal");
        assert!(watcher.due.is_none());
        watcher.start(later, "running", true);
        assert!(watcher.due.is_none());
        watcher.start(later, "next-run", true);
        assert_eq!(watcher.due, Some(later));
        assert!(watcher.progress.is_none());
    }
}
