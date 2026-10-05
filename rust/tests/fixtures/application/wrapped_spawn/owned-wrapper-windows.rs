//! Synthetic std-only native process. No provider, network, account or home I/O.
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn IsProcessInJob(
        process: *mut std::ffi::c_void,
        job: *mut std::ffi::c_void,
        result: *mut i32,
    ) -> i32;
}

fn main() -> io::Result<()> {
    let root = PathBuf::from(
        std::env::var_os("SYNTHETIC_WRAPPER_ROOT")
            .expect("explicit synthetic fixture root required"),
    );
    assert!(root.is_absolute() && root.is_dir());
    // Only arguments generated for this synthetic wrapper are recorded.
    fs::write(
        root.join("argv"),
        std::env::args().skip(1).collect::<Vec<_>>().join("\n"),
    )?;
    fs::write(
        root.join("environment"),
        [
            std::env::var("MANY_AI_CLI_HUB_PORT").unwrap_or_default(),
            std::env::var("MANY_AI_CLI_USAGE_PROBE").unwrap_or_default(),
            std::env::var("MANY_AI_CLI_SUBSCRIPTION_LOGIN").unwrap_or_default(),
            std::env::var("SYNTHETIC_PRECEDENCE").unwrap_or_default(),
        ]
        .join("\n"),
    )?;
    // Presence-only checks avoid ever copying the registration proof or handle.
    assert!(std::env::var_os("MANY_AI_CLI_INTERNAL_SPAWN_PROOF").is_some());
    assert!(std::env::var_os("MANY_AI_CLI_INTERNAL_STARTUP_JOB").is_some());
    let job = std::env::var("MANY_AI_CLI_INTERNAL_STARTUP_JOB")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut assigned = 0;
    // The helper checks its own inherited query-only Job handle, never a PID
    // supplied by another process or a discovered Job name.
    assert_ne!(
        unsafe {
            IsProcessInJob(
                GetCurrentProcess(),
                job as *mut std::ffi::c_void,
                &mut assigned,
            )
        },
        0
    );
    assert_eq!(assigned, 1);
    fs::write(root.join("job-assigned"), b"assigned-before-resume")?;
    println!("proof-present\nstdout-append");
    eprintln!("stderr-append");
    io::stdout().flush()?;
    io::stderr().flush()?;
    fs::write(root.join("pid"), std::process::id().to_string())?;
    if std::env::var("SYNTHETIC_EXIT_EARLY").ok().as_deref() == Some("1") {
        std::process::exit(23);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while !root.join("stop").exists() {
        if Instant::now() >= deadline {
            std::process::exit(91);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    std::process::exit(7);
}
