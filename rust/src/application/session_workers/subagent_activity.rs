//! Newest modification time of a provider's sub-agent log folder, used only as
//! evidence that the turn is still making progress (the "no response" badge).
//!
//! Claude Code writes nothing to the parent transcript while a sub-agent runs;
//! the output goes to `<transcript without .jsonl>/subagents/agent-<id>.jsonl`.
//! Watching only the parent therefore counts a healthy sub-agent run as a stall.
//! Metadata only: file contents and names are neither read nor kept, and a
//! missing or unreadable folder is the normal state, so it stays silent.
//! Independent of `workflow.subagent_tree_enabled` (the tree reader is not used).
use crate::{
    application::session_observations::subagents::{
        artifact_metadata, directory_metadata, entries,
    },
    config::RuntimePaths,
    proto::time::Timestamp,
};
use std::path::{Path, PathBuf};

/// How often a running session's folder is looked at.
pub(super) const PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// The single place that decides which providers keep sub-agent logs, and where.
/// Today only claude does; add another provider's layout here when one appears.
pub(super) fn dir_for_transcript(provider: &str, transcript: &Path) -> Option<PathBuf> {
    if provider != "claude" || transcript.extension().is_none_or(|ext| ext != "jsonl") {
        return None;
    }
    let session_dir = transcript.with_extension("");
    session_dir.file_name()?;
    Some(session_dir.join("subagents"))
}

pub(super) fn latest_mtime(
    paths: &RuntimePaths,
    provider: &str,
    transcript: &Path,
) -> Option<Timestamp> {
    let dir = dir_for_transcript(provider, transcript)?;
    let mut latest: Option<Timestamp> = None;
    for entry in entries(paths, &dir).ok()? {
        let path = dir.join(&entry.name);
        let metadata = if entry.is_dir {
            directory_metadata(paths, &path)
        } else {
            artifact_metadata(paths, &path)
        };
        let Some(at) = metadata
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|m| Timestamp::from_system_time(m).ok())
        else {
            continue;
        };
        if latest.is_none_or(|current| at > current) {
            latest = Some(at);
        }
    }
    latest
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Duration};

    fn at(seconds: i64) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + Duration::from_secs(seconds as u64)
    }
    fn touch(path: &Path, seconds: i64) {
        fs::write(path, b"synthetic").unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at(seconds))
            .unwrap();
    }
    fn paths(root: &Path) -> RuntimePaths {
        RuntimePaths::trial(root, 49691, &root.join("../installed-subagent-fixture")).unwrap()
    }

    #[test]
    fn directory_is_derived_from_the_claude_transcript_name_only() {
        let transcript = Path::new("projects")
            .join("proj")
            .join("synthetic-session.jsonl");
        assert_eq!(
            dir_for_transcript("claude", &transcript),
            Some(
                Path::new("projects")
                    .join("proj")
                    .join("synthetic-session")
                    .join("subagents")
            )
        );
        assert_eq!(dir_for_transcript("codex", &transcript), None);
        assert_eq!(
            dir_for_transcript("claude", Path::new("projects/proj/synthetic-session")),
            None
        );
    }

    #[test]
    fn newest_entry_wins_without_reading_contents() {
        let root = tempfile::tempdir().unwrap();
        let transcript = root.path().join("synthetic-session.jsonl");
        touch(&transcript, 1_000);
        let dir = root.path().join("synthetic-session").join("subagents");
        fs::create_dir_all(&dir).unwrap();
        touch(&dir.join("agent-a.jsonl"), 2_000);
        touch(&dir.join("agent-b.jsonl"), 3_000);
        touch(&dir.join("agent-a.meta.json"), 2_500);
        let latest = latest_mtime(&paths(root.path()), "claude", &transcript);
        assert_eq!(latest, Some(Timestamp::from_unix(3_000, 0).unwrap()));
    }

    #[test]
    fn missing_folder_or_other_provider_yields_nothing() {
        let root = tempfile::tempdir().unwrap();
        let transcript = root.path().join("synthetic-session.jsonl");
        touch(&transcript, 1_000);
        assert_eq!(
            latest_mtime(&paths(root.path()), "claude", &transcript),
            None
        );
        let dir = root.path().join("synthetic-session").join("subagents");
        fs::create_dir_all(&dir).unwrap();
        touch(&dir.join("agent-a.jsonl"), 2_000);
        assert_eq!(
            latest_mtime(&paths(root.path()), "codex", &transcript),
            None
        );
    }

    mod poll {
        use super::super::super::*;
        use crate::{
            config::{Config, ConfigStore},
            hub::task_owner::HubTaskOwner,
            terminal::{
                events::CoreEventBus,
                journal::{JournalOptions, SessionJournal},
                session::EngineOptions,
            },
        };
        use std::{fs, time::SystemTime};

        struct Io;
        impl WrapperTransport for Io {
            fn send<'a>(
                &'a self,
                _: SessionBinding,
                _: proto::Message,
            ) -> CoreFuture<'a, Result<(), SessionError>> {
                Box::pin(async { Ok(()) })
            }
        }
        impl CoreEffectSink for Io {
            fn apply<'a>(
                &'a self,
                effects: CoreEffects,
            ) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
                Box::pin(async move {
                    drop(effects);
                    Ok(())
                })
            }
        }
        impl WrappedSessionSpawner for Io {
            fn spawn_and_wait<'a>(
                &'a self,
                _: WrappedSpawnSpec,
                _: Duration,
                _: &'a HttpWaitCancellation,
            ) -> CoreFuture<'a, SpawnWaitOutcome> {
                Box::pin(async { SpawnWaitOutcome::Failed("synthetic fixture".into()) })
            }
        }
        const OLD: &str = "2000-01-01T00:00:00Z";
        const SESSION: &str = "12345678-1234-1234-1234-123456789abc";

        #[tokio::test]
        async fn running_sessions_count_subagent_logs_even_with_the_tree_setting_off() {
            let root = tempfile::tempdir().unwrap();
            let trial = root.path().join("trial");
            fs::create_dir(&trial).unwrap();
            let paths = RuntimePaths::trial(&trial, 49692, &root.path().join("installed")).unwrap();
            let mut config = Config::defaults(&paths);
            config.workflow.subagent_tree_enabled = false;
            let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
            let io = Arc::new(Io);
            let effects: Arc<dyn CoreEffectSink> = io.clone();
            let core = Arc::new(SessionEngine::new(
                EngineOptions {
                    idle_after: Duration::from_secs(3600),
                    ..Default::default()
                },
                Arc::new(SessionJournal::new(
                    paths.clone(),
                    None,
                    JournalOptions::default(),
                )),
                io.clone(),
                effects.clone(),
                io.clone(),
                CoreEventBus::new(16).unwrap(),
            ));
            let claude_dir = trial.join("claude-home");
            let registered = core
                .register(
                    RegisterRequest {
                        message: proto::Message {
                            provider: "claude".into(),
                            cwd: "synthetic-project".into(),
                            claude_dir: claude_dir.to_string_lossy().into_owned(),
                            agent_session_id: SESSION.into(),
                            pid: 21,
                            cols: 80,
                            rows: 24,
                            ..Default::default()
                        },
                        spawn_proof: None,
                    },
                    WrapperConnectionId(1),
                    Timestamp::now(),
                )
                .await
                .unwrap();
            let binding = registered.binding;
            io.apply(registered.after_registered).await.unwrap();
            let workers = SessionWorkers::new(
                Arc::new(ConfigStore::new(paths.clone(), config).unwrap()),
                paths.clone(),
                Arc::new(FilesService::new(trial.clone(), paths.clone())),
                owner.handle(),
                Arc::new(|_, _| {}),
            );
            workers
                .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
                .unwrap();
            let project = claude_dir.join("projects").join("synthetic-project");
            fs::create_dir_all(&project).unwrap();
            let transcript = project.join(format!("{SESSION}.jsonl"));
            fs::write(
                &transcript,
                r#"{"type":"user","message":{"role":"user","content":"synthetic"},"timestamp":"2026-10-14T00:00:00Z","cwd":"synthetic-project"}"#
                    .to_owned()
                    + "\n",
            )
            .unwrap();
            core.observe_output(
                binding,
                OutputChunk {
                    bytes: b"synthetic".to_vec(),
                    total_pty_bytes: 0,
                },
                Timestamp::now(),
            )
            .unwrap();
            let grew = || {
                core.details(binding.session)
                    .unwrap()
                    .snapshot
                    .transcript_grew_at
            };
            let make_old = || {
                // Pretend the clock is stale; the transcript offset is already published.
                core.apply_observation(
                    binding,
                    SessionObservation::Transcript {
                        path: transcript.clone(),
                        agent_session_id: SESSION.into(),
                        safe_offset: 1,
                        grew_at: OLD.into(),
                    },
                    Timestamp::now(),
                )
                .unwrap();
                assert_eq!(grew(), OLD);
            };
            let reopen = |cursors: &mut BTreeMap<LiveSessionId, Cursor>| {
                cursors
                    .get_mut(&binding.session)
                    .unwrap()
                    .subagent_checked_at = None;
            };
            let mut cursors = BTreeMap::new();
            // No sub-agent folder yet: the parent transcript alone decides.
            workers.poll(&mut cursors).await.unwrap();
            assert!(cursors.contains_key(&binding.session));
            make_old();
            reopen(&mut cursors);
            workers.poll(&mut cursors).await.unwrap();
            assert_eq!(grew(), OLD);
            // A sub-agent log appears (tree setting is off): the clock moves.
            let agents = project.join(SESSION).join("subagents");
            fs::create_dir_all(&agents).unwrap();
            fs::write(agents.join("agent-1.jsonl"), b"synthetic").unwrap();
            reopen(&mut cursors);
            workers.poll(&mut cursors).await.unwrap();
            assert_ne!(grew(), OLD);
            // Same folder, nothing newer: the clock stays where it was.
            make_old();
            reopen(&mut cursors);
            workers.poll(&mut cursors).await.unwrap();
            assert_eq!(grew(), OLD);
            // A newer write is ignored until the ten second probe is due again.
            fs::File::options()
                .write(true)
                .open(agents.join("agent-1.jsonl"))
                .unwrap()
                .set_modified(SystemTime::now() + Duration::from_secs(5))
                .unwrap();
            cursors
                .get_mut(&binding.session)
                .unwrap()
                .subagent_checked_at = Some(Timestamp::now());
            workers.poll(&mut cursors).await.unwrap();
            assert_eq!(grew(), OLD);
            reopen(&mut cursors);
            workers.poll(&mut cursors).await.unwrap();
            assert_ne!(grew(), OLD);
        }
    }
}
