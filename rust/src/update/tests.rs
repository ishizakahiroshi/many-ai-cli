use super::{job::*, plan::*};
#[cfg(unix)]
use crate::proto::core::*;
use crate::{
    process::ExitOutcome,
    proto::provider::{Definition, LaunchDefinition, UpdateDefinition},
};
#[cfg(unix)]
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
fn definition() -> Definition {
    Definition {
        id: "synthetic".into(),
        launch: Some(LaunchDefinition {
            executable: "A".into(),
            ..Default::default()
        }),
        update: Some(UpdateDefinition {
            executable: "B".into(),
            args: vec!["update".into()],
            ..Default::default()
        }),
        ..Default::default()
    }
}
#[test]
fn updater_resolves_b_independently_for_preview_and_log() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan_update(&definition(), dir.path(), |s| Some(dir.path().join(s))).unwrap();
    assert_eq!(p.requested_executable, "B");
    assert_eq!(p.argv()[0], dir.path().join("B").to_str().unwrap());
    assert!(
        p.log_header()
            .contains(&format!("executable: {}", dir.path().join("B").display()))
    );
    assert_eq!(p.command.process.executable, dir.path().join("B"));
}
#[test]
fn updater_missing_b_never_falls_back_to_a() {
    let dir = tempfile::tempdir().unwrap();
    let result = plan_update(&definition(), dir.path(), |s| {
        if s == "A" {
            Some(dir.path().join("A"))
        } else {
            None
        }
    });
    assert!(matches!(result,Err(Unavailable::UpdateExecutableMissing(b))if b=="B"));
}
#[test]
fn updater_secondary_candidate_needs_explicit_update_command() {
    let dir = tempfile::tempdir().unwrap();
    let mut d = definition();
    d.launch.as_mut().unwrap().executable_candidates = vec!["secondary".into()];
    d.update.as_mut().unwrap().executable.clear();
    assert!(matches!(
        plan_update(&d, dir.path(), |s| if s == "secondary" {
            Some(dir.path().join(s))
        } else {
            None
        }),
        Err(Unavailable::NotConfigured)
    ));
    d.update.as_mut().unwrap().executable = "B".into();
    assert!(
        plan_update(&d, dir.path(), |s| if s != "A" {
            Some(dir.path().join(s))
        } else {
            None
        })
        .is_ok()
    );
}
#[test]
fn updater_disabled_login_and_empty_args_distinguish_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let mut d = definition();
    d.update.as_mut().unwrap().enabled = Some(false);
    assert!(matches!(
        plan_update(&d, dir.path(), |s| Some(dir.path().join(s))),
        Err(Unavailable::UpdateDisabled)
    ));
    d.update.as_mut().unwrap().login_may_be_required = true;
    assert!(matches!(
        plan_update(&d, dir.path(), |s| Some(dir.path().join(s))),
        Err(Unavailable::LoginMayBeRequired)
    ));
    d.update.as_mut().unwrap().enabled = Some(true);
    d.update.as_mut().unwrap().args.clear();
    assert!(matches!(
        plan_update(&d, dir.path(), |s| Some(dir.path().join(s))),
        Err(Unavailable::NotConfigured)
    ));
}
#[test]
fn updater_outcome_precedence_and_versions() {
    let ok = ExitOutcome::Exited {
        code: Some(0),
        signal: None,
    };
    let fail = ExitOutcome::Exited {
        code: Some(1),
        signal: None,
    };
    assert_eq!(
        classify("authentication required", &ok, false, Some("1"), Some("2")),
        Outcome::LoginRequired
    );
    assert_eq!(
        classify("EPERM", &fail, false, None, None),
        Outcome::FileInUse
    );
    assert_eq!(
        classify("EPERM", &ExitOutcome::TimedOut, false, None, None),
        Outcome::Failed
    );
    assert_eq!(
        classify("", &ok, false, Some("1\n"), Some("1")),
        Outcome::Latest
    );
    assert_eq!(
        classify("", &ok, false, Some("1"), Some("2")),
        Outcome::Updated
    );
    assert_eq!(classify("", &ok, false, None, Some("2")), Outcome::Unknown);
}

#[cfg(unix)]
#[derive(Default)]
struct Admission(Mutex<AdmissionState>);
#[cfg(unix)]
impl ProviderUpdateAdmission for Admission {
    fn begin_provider_spawn(&self, p: &str) -> Result<ProviderSpawnLease, ProviderAdmissionError> {
        self.0.lock().unwrap().begin_provider_spawn(p)
    }
    fn end_provider_spawn(&self, l: ProviderSpawnLease) {
        self.0.lock().unwrap().end_provider_spawn(&l);
    }
    fn begin_provider_update(
        &self,
        p: &str,
    ) -> Result<ProviderUpdateLease, ProviderAdmissionError> {
        self.0.lock().unwrap().begin_provider_update(p)
    }
    fn end_provider_update(&self, l: ProviderUpdateLease) {
        self.0.lock().unwrap().end_provider_update(&l);
    }
    fn provider_session_count(&self, p: &str) -> usize {
        self.0.lock().unwrap().provider_session_count(p)
    }
}
#[cfg(unix)]
fn script(dir: &std::path::Path, name: &str, text: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}
#[cfg(unix)]
#[tokio::test]
async fn updater_executes_b_and_retains_snapshot_when_definition_changes() {
    let dir = tempfile::tempdir().unwrap();
    let a = script(dir.path(), "A", "#!/bin/sh\nprintf launch_A\n");
    let b = script(dir.path(), "B", "#!/bin/sh\nprintf 'updater_B:%s' \"$1\"\n");
    let mut d = definition();
    let p = plan_update(&d, dir.path(), |s| {
        Some(if s == "A" { a.clone() } else { b.clone() })
    })
    .unwrap();
    d.update.as_mut().unwrap().executable = "A".into();
    let admission = Arc::new(Admission::default());
    let result = execute(p, admission.clone()).await.unwrap();
    assert_eq!(result.output.stdout, b"updater_B:update");
    assert_eq!(result.argv[0], b.to_str().unwrap());
    let log = String::from_utf8(result.log).unwrap();
    assert!(log.contains(&format!("executable: {}", b.display())));
    assert!(!log.contains("launch_A"));
    let lease = admission.begin_provider_spawn("synthetic").unwrap();
    admission.end_provider_spawn(lease);
}
#[cfg(unix)]
#[tokio::test]
async fn updater_timeout_and_failed_spawn_release_same_admission() {
    let dir = tempfile::tempdir().unwrap();
    let b = script(dir.path(), "B", "#!/bin/sh\nsleep 10\n");
    let mut p = plan_update(&definition(), dir.path(), |_| Some(b.clone())).unwrap();
    p.command.process.timeout = Duration::from_millis(40);
    let admission = Arc::new(Admission::default());
    let out = execute(p, admission.clone()).await.unwrap();
    assert_eq!(out.output.outcome, ExitOutcome::TimedOut);
    let mut p = plan_update(&definition(), dir.path(), |_| Some(b.clone())).unwrap();
    p.command.process.executable = dir.path().join("absent");
    assert!(matches!(
        execute(p, admission.clone()).await,
        Err(UpdateError::Io(_))
    ));
    let lease = admission.begin_provider_spawn("synthetic").unwrap();
    admission.end_provider_spawn(lease);
}
#[cfg(unix)]
#[tokio::test]
async fn updater_rejects_pending_spawn_before_running_process() {
    let dir = tempfile::tempdir().unwrap();
    let b = script(dir.path(), "B", "#!/bin/sh\nprintf should_not_run\n");
    let p = plan_update(&definition(), dir.path(), |_| Some(b.clone())).unwrap();
    let admission = Arc::new(Admission::default());
    let lease = admission.begin_provider_spawn("synthetic").unwrap();
    assert!(matches!(
        execute(p, admission.clone()).await,
        Err(UpdateError::Admission(
            ProviderAdmissionError::PendingSpawns { count: 1 }
        ))
    ));
    admission.end_provider_spawn(lease);
}
