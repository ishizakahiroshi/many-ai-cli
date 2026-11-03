//! Public provider-boundary contracts for the wrapper startup adapter. The native
//! lifecycle fixtures are unit tests so constructing a core-only proof receipt
//! does not need a public test-only bypass of the registration boundary.
use many_ai_cli::{
    process::{ProcessPlan, wrapper_startup::STARTUP_JOB_ENV},
    wrapper::{
        launch_payload::PROTECTED_ENV_NAMES_ENV,
        startup::{StartupLifetime, strip_provider_environment},
    },
};
use std::{collections::BTreeMap, time::Duration};

#[test]
fn startup_job_metadata_is_removed_from_every_provider_override_spelling() {
    let mut plan = ProcessPlan {
        executable: "synthetic-provider".into(),
        args: vec![],
        cwd: "synthetic-root".into(),
        env: BTreeMap::from([
            (STARTUP_JOB_ENV.into(), Some("111".into())),
            (
                STARTUP_JOB_ENV.to_ascii_lowercase().into(),
                Some("222".into()),
            ),
            ("SYNTHETIC_ORDINARY_ENV".into(), Some("retained".into())),
            (
                many_ai_cli::wrapper::iris_owner::OWNER_ENV.into(),
                Some("synthetic-owner".into()),
            ),
            (
                many_ai_cli::wrapper::iris_owner::OWNER_ENV
                    .to_ascii_lowercase()
                    .into(),
                Some("synthetic-owner".into()),
            ),
        ]),
        stdin: vec![],
        timeout: Duration::ZERO,
        output_cap: 0,
        pipe_drain_timeout: Duration::from_secs(1),
    };
    strip_provider_environment(&mut plan);
    assert_eq!(
        plan.env.get(std::ffi::OsStr::new(STARTUP_JOB_ENV)),
        Some(&None)
    );
    assert_eq!(
        plan.env.get(std::ffi::OsStr::new(PROTECTED_ENV_NAMES_ENV)),
        Some(&None)
    );
    assert_eq!(plan.env.len(), 5);
    assert_eq!(
        plan.env.get(std::ffi::OsStr::new(
            many_ai_cli::wrapper::iris_owner::OWNER_ENV
        )),
        Some(&None)
    );
    assert_eq!(
        plan.env.get(std::ffi::OsStr::new(
            &many_ai_cli::wrapper::iris_owner::OWNER_ENV.to_ascii_lowercase()
        )),
        Some(&None)
    );
    assert_eq!(
        plan.env.get(std::ffi::OsStr::new("SYNTHETIC_ORDINARY_ENV")),
        Some(&Some("retained".into()))
    );
}

#[test]
fn standalone_wrapper_needs_no_startup_job_and_invalid_metadata_is_rejected() {
    assert!(StartupLifetime::adopt(None).is_ok());
    for value in ["", "0", "not-a-handle", "-1"] {
        assert!(StartupLifetime::adopt(Some(std::ffi::OsStr::new(value))).is_err());
    }
}

#[test]
fn owner_environment_native_helper() {
    if std::env::var_os("SYNTHETIC_OWNER_ENV_HELPER").is_none() {
        return;
    }
    let name = many_ai_cli::wrapper::iris_owner::OWNER_ENV;
    assert!(
        std::env::var_os(name).is_none(),
        "internal owner metadata reached child"
    );
    assert!(
        std::env::var_os(name.to_ascii_lowercase()).is_none(),
        "internal owner metadata reached child under another spelling"
    );
}

#[tokio::test]
async fn owner_metadata_is_absent_in_the_actual_native_child() {
    let temp = tempfile::tempdir().unwrap();
    let name = many_ai_cli::wrapper::iris_owner::OWNER_ENV;
    let mut plan = ProcessPlan {
        executable: std::env::current_exe().unwrap(),
        args: vec![
            "--exact".into(),
            "owner_environment_native_helper".into(),
            "--nocapture".into(),
        ],
        cwd: temp.path().to_path_buf(),
        env: BTreeMap::from([
            (name.into(), Some("synthetic-bootstrap".into())),
            (
                name.to_ascii_lowercase().into(),
                Some("synthetic-bootstrap".into()),
            ),
            ("SYNTHETIC_OWNER_ENV_HELPER".into(), Some("1".into())),
        ]),
        stdin: vec![],
        timeout: Duration::from_secs(5),
        output_cap: 8192,
        pipe_drain_timeout: Duration::from_secs(1),
    };
    strip_provider_environment(&mut plan);
    let output =
        many_ai_cli::process::run_capped(&plan, &many_ai_cli::process::Cancellation::default())
            .await
            .unwrap();
    assert!(
        matches!(
            output.outcome,
            many_ai_cli::process::ExitOutcome::Exited { code: Some(0), .. }
        ),
        "native provider environment boundary failed"
    );
}
