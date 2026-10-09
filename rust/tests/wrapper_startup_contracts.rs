//! Public provider-boundary contracts for the wrapper startup adapter. The native
//! lifecycle fixtures are unit tests so constructing a core-only proof receipt
//! does not need a public test-only bypass of the registration boundary.
use many_ai_cli::{
    process::{ProcessPlan, wrapper_startup::STARTUP_JOB_ENV},
    wrapper::startup::{StartupLifetime, strip_provider_environment},
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
    assert_eq!(plan.env.len(), 2);
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
