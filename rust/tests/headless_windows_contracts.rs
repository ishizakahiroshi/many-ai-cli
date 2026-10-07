//! V01 native headless ownership. These fixtures execute real Windows children,
//! inherit both native pipes, and retain process HANDLEs before cancellation.
//! A private UI-restricted Job induces an actual nested Job assignment denial;
//! no production spawn hook or process-wide environment mutation is involved.
#![cfg(windows)]

use many_ai_cli::{
    orchestration::headless::{Spec, run},
    process::{Cancellation, ExitOutcome, ProcessPlan, SpawnOptions, run_capped_with_options},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    mem::{size_of, size_of_val},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_ACCESS_DENIED, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        JobObjects::*,
        SystemInformation::OSVERSIONINFOW,
        Threading::{
            GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
        },
    },
};

const STEP: Duration = Duration::from_secs(20);
const RUN_TIMEOUT: Duration = Duration::from_secs(5);
const RETURN_BOUND: Duration = Duration::from_secs(3);
const DRAIN: Duration = Duration::from_millis(250);
const POLL: Duration = Duration::from_millis(10);
const ROOT: &str = "MANY_AI_V01_ROOT";
const ROLE: &str = "MANY_AI_V01_ROLE";
const CASE: &str = "MANY_AI_V01_CASE";

struct Job(OwnedHandle);

impl Job {
    fn new(restrict_ui: bool) -> io::Result<Self> {
        // SAFETY: all successful native handles become owned immediately.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        job.set(JobObjectExtendedLimitInformation, &limits)?;
        if restrict_ui {
            // Only the synthetic tree loses clipboard-read permission. Microsoft
            // documents that UI-limited Jobs cannot form a nested hierarchy:
            // https://learn.microsoft.com/windows/win32/procthread/nested-jobs
            let limits = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: JOB_OBJECT_UILIMIT_READCLIPBOARD,
            };
            job.set(JobObjectBasicUIRestrictions, &limits)?;
        }
        Ok(job)
    }

    fn set<T>(&self, class: JOBOBJECTINFOCLASS, value: &T) -> io::Result<()> {
        if unsafe {
            SetInformationJobObject(
                self.0.as_raw_handle(),
                class,
                value as *const T as _,
                size_of::<T>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn assign(&self, process: HANDLE) -> io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn contains(&self, process: HANDLE) -> bool {
        in_job(process, self.0.as_raw_handle())
    }

    fn active_processes(&self) -> u32 {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        assert_ne!(
            unsafe {
                QueryInformationJobObject(
                    self.0.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    &mut accounting as *mut _ as _,
                    size_of_val(&accounting) as u32,
                    std::ptr::null_mut(),
                )
            },
            0,
            "query private fixture Job: {}",
            io::Error::last_os_error(),
        );
        accounting.ActiveProcesses
    }

    fn terminate(&self) {
        unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) };
    }
}

fn in_job(process: HANDLE, job: HANDLE) -> bool {
    let mut result = 0;
    assert_ne!(
        unsafe { IsProcessInJob(process, job, &mut result) },
        0,
        "query owned process Job membership: {}",
        io::Error::last_os_error(),
    );
    result != 0
}

/// The supervisor alone owns this Job. Fixture admission is gated on stdin so
/// no fixture can spawn descendants before assignment succeeds. No breakaway
/// flags or changes to a host/runner Job are permitted here.
struct Tree {
    child: Child,
    job: Job,
}

impl Tree {
    fn start(mut command: Command, restrict_ui: bool) -> Self {
        let job = Job::new(restrict_ui).expect("create private fixture Job");
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start owned fixture runner");
        let mut tree = Self { child, job };
        let inherited_job = in_job(tree.child.as_raw_handle(), std::ptr::null_mut());
        tree.job.assign(tree.child.as_raw_handle()).unwrap_or_else(|error| {
            panic!(
                "V01 fixture setup failed: private Job assignment; restricted_ui={restrict_ui}, inherited_job={inherited_job}, error={error}. Host Job restrictions are not bypassed."
            )
        });
        assert!(tree.job.contains(tree.child.as_raw_handle()));
        tree.child.stdin.take().unwrap().write_all(b"G").unwrap();
        tree
    }

    fn finished(&mut self) -> bool {
        self.child.try_wait().unwrap().is_some()
    }

    fn wait(&mut self, bound: Duration) {
        let until = Instant::now() + bound;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "owned fixture runner failed: {status}");
                return;
            }
            assert!(Instant::now() < until, "owned fixture runner did not exit");
            thread::sleep(POLL);
        }
    }

    fn process(&self, root: &Path, file: &str) -> Process {
        let pid: u32 = fs::read_to_string(root.join(file))
            .unwrap()
            .parse()
            .unwrap();
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                0,
                pid,
            )
        };
        assert!(
            !raw.is_null(),
            "open owned {file}: {}",
            io::Error::last_os_error()
        );
        let process = Process {
            pid,
            handle: unsafe { OwnedHandle::from_raw_handle(raw) },
        };
        assert!(
            self.job.contains(process.handle.as_raw_handle()),
            "{file} is outside fixture Job"
        );
        assert!(
            !process.exited(),
            "{file} exited before the supervisor held its handle"
        );
        process
    }

    fn await_file(&mut self, root: &Path, file: &str, bound: Duration) {
        let until = Instant::now() + bound;
        while !root.join(file).exists() {
            assert!(!self.finished(), "owned runner exited before {file}");
            assert!(
                Instant::now() < until,
                "owned fixture did not publish {file}"
            );
            thread::sleep(POLL);
        }
    }

    fn assert_empty_before_cleanup(&self) {
        let until = Instant::now() + RETURN_BOUND;
        while self.job.active_processes() != 0 {
            assert!(
                Instant::now() < until,
                "private Job still has a live owned process"
            );
            thread::sleep(POLL);
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // Failure cleanup only touches the retained fixture Job/direct handle.
        self.job.terminate();
        let _ = self.child.kill();
        unsafe { WaitForSingleObject(self.child.as_raw_handle(), 2_000) };
        let _ = self.child.try_wait();
    }
}

struct Process {
    pid: u32,
    handle: OwnedHandle,
}

impl Process {
    fn exited(&self) -> bool {
        match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => true,
            WAIT_TIMEOUT => false,
            value => panic!("owned process HANDLE wait failed: {value}"),
        }
    }

    fn wait(&self, bound: Duration) {
        assert_eq!(
            unsafe { WaitForSingleObject(self.handle.as_raw_handle(), bound.as_millis() as u32) },
            WAIT_OBJECT_0,
            "owned process {} did not signal exit",
            self.pid,
        );
    }

    fn terminate(&self) {
        assert_ne!(
            unsafe { TerminateProcess(self.handle.as_raw_handle(), 1) },
            0,
            "terminate retained owned descendant handle: {}",
            io::Error::last_os_error(),
        );
    }
}

fn environment(root: &Path, role: &str, case: &str) -> BTreeMap<OsString, Option<OsString>> {
    let mut values = BTreeMap::new();
    for key in ["SystemRoot", "WINDIR"] {
        values.insert(key.into(), std::env::var_os(key));
    }
    for key in [
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "TMPDIR",
    ] {
        values.insert(key.into(), Some(root.as_os_str().to_owned()));
    }
    values.insert(ROOT.into(), Some(root.as_os_str().to_owned()));
    values.insert(ROLE.into(), Some(role.into()));
    values.insert(CASE.into(), Some(case.into()));
    values
}

fn fixture_command(root: &Path, role: &str, case: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "native_headless_fixture", "--nocapture"])
        .current_dir(root)
        .env_clear();
    for (key, value) in environment(root, role, case) {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    command
}

fn publish(root: &Path, name: &str, bytes: impl AsRef<[u8]>) {
    let temporary = root.join(format!("{name}.tmp"));
    fs::write(&temporary, bytes).unwrap();
    fs::rename(temporary, root.join(name)).unwrap();
}

fn admit() {
    let mut byte = [0];
    io::stdin()
        .read_exact(&mut byte)
        .expect("supervisor admission");
    assert_eq!(byte, [b'G']);
}

#[test]
fn native_headless_fixture() {
    let Some(role) = std::env::var_os(ROLE) else {
        return;
    };
    let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
    let case = std::env::var(CASE).unwrap();
    match role.to_str().unwrap() {
        "rust-runner" => {
            admit();
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            let mut report = runtime.block_on(rust_runner(&root, &case));
            // Tokio Windows pipe reads use blocking IO. Return of run alone
            // cannot establish that those reads and the runtime have stopped.
            let shutdown = Instant::now();
            drop(runtime);
            assert!(
                shutdown.elapsed() < RETURN_BOUND,
                "owned runner runtime did not drain promptly"
            );
            report["runtime_shutdown_ms"] = json!(shutdown.elapsed().as_millis());
            publish(
                &root,
                "rust-result.json",
                serde_json::to_vec(&report).unwrap(),
            );
        }
        "direct" => {
            publish(&root, "direct.pid", std::process::id().to_string());
            let grandchild = fixture_command(&root, "grandchild", &case)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            // The runner/supervisor Jobs retain containment. Keeping the
            // grandchild alive after this handle closes is the tested behavior.
            drop(grandchild);
            while !root.join("exit-direct").exists() {
                thread::sleep(POLL);
            }
        }
        "grandchild" => {
            publish(&root, "grandchild.pid", std::process::id().to_string());
            writeln!(io::stdout(), "V01_STDOUT_READY").unwrap();
            writeln!(io::stderr(), "V01_STDERR_READY").unwrap();
            io::stdout().flush().unwrap();
            io::stderr().flush().unwrap();
            publish(&root, "grandchild-ready", b"both native pipes written");
            while !root.join("probe-retained-pipes").exists() {
                thread::sleep(POLL);
            }
            writeln!(io::stdout(), "V01_STDOUT_RETAINED").unwrap();
            writeln!(io::stderr(), "V01_STDERR_RETAINED").unwrap();
            io::stdout().flush().unwrap();
            io::stderr().flush().unwrap();
            publish(
                &root,
                "retained-pipes-written",
                b"both native pipes written again",
            );
            loop {
                thread::sleep(Duration::from_secs(1));
            }
        }
        other => panic!("unknown fixture role {other}"),
    }
    // Do not mix libtest PASS output into the actual headless pipe fixture.
    std::process::exit(0);
}

async fn rust_runner(root: &Path, case: &str) -> Value {
    let spec = Spec {
        process: ProcessPlan {
            executable: std::env::current_exe().unwrap(),
            args: ["--exact", "native_headless_fixture", "--nocapture"]
                .map(Into::into)
                .to_vec(),
            cwd: root.to_owned(),
            env: environment(root, "direct", case),
            stdin: vec![],
            timeout: if case == "deadline" {
                RUN_TIMEOUT
            } else {
                Duration::ZERO
            },
            output_cap: 16 * 1024,
            pipe_drain_timeout: DRAIN,
        },
        prompt: String::new(),
        prompt_via: "stdin".into(),
        format: "text".into(),
        raw_logs: None,
        event_capacity: 64,
    };
    let start = Instant::now();
    let cancel = Cancellation::default();
    if case == "failed-attach" {
        // This explicit native probe is distinct from the product's subsequent
        // call. The unchanged headless error path is asserted separately below.
        let probe = Job::new(false).unwrap();
        let error = probe.assign(unsafe { GetCurrentProcess() }).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
        let error = run(spec, &cancel, |_| {}).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("os error 5"));
        assert!(start.elapsed() < RETURN_BOUND);
        assert!(!root.join("direct.pid").exists());
        assert!(!root.join("grandchild.pid").exists());
        return json!({
            "case":case, "run_return_ms":start.elapsed().as_millis(),
            "native_attach_probe_error":ERROR_ACCESS_DENIED,
            "headless_error_kind":"PermissionDenied", "provider_executed":false,
        });
    }
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let captured = bytes.clone();
    let run_cancel = cancel.clone();
    let mut run_task = tokio::spawn(async move {
        run(spec, &run_cancel, move |chunk| {
            captured.lock().unwrap().extend_from_slice(chunk)
        })
        .await
    });
    let ready_until = Instant::now() + STEP;
    loop {
        assert!(
            !run_task.is_finished(),
            "headless ended before owned descendant readiness"
        );
        assert!(
            Instant::now() < ready_until,
            "owned descendant readiness timed out"
        );
        let ready = {
            let output = bytes.lock().unwrap();
            let text = String::from_utf8_lossy(&output);
            root.join("supervisor-ready").exists()
                && text.contains("V01_STDOUT_READY")
                && text.contains("V01_STDERR_READY")
        };
        if ready {
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    let ready_ms = start.elapsed().as_millis();
    let trigger = Instant::now();
    match case {
        "cancel" => cancel.cancel(),
        "normal" => publish(root, "exit-direct", b"owned direct child may exit"),
        "deadline" => assert!(
            start.elapsed() < RUN_TIMEOUT,
            "readiness must precede product deadline"
        ),
        other => panic!("unknown Rust runner case {other}"),
    }
    let result = tokio::time::timeout(RUN_TIMEOUT + RETURN_BOUND, &mut run_task)
        .await
        .expect("headless did not return within its deadline and drain bound")
        .unwrap()
        .unwrap();
    let elapsed = start.elapsed();
    match case {
        "cancel" => {
            assert!(result.canceled && !result.timed_out);
            assert_eq!(result.state, "error");
            assert!(trigger.elapsed() < RETURN_BOUND);
        }
        "normal" => {
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.state, "completed");
            assert!(result.process_output.pipes_forced_closed);
            assert!(trigger.elapsed() < RETURN_BOUND);
        }
        "deadline" => {
            assert!(result.timed_out && !result.canceled);
            assert_eq!(result.state, "error");
            assert!(elapsed >= RUN_TIMEOUT && elapsed < RUN_TIMEOUT + RETURN_BOUND);
        }
        _ => unreachable!(),
    }
    assert_eq!(result.dropped_chunks, 0);
    assert!(String::from_utf8_lossy(&result.process_output.stdout).contains("V01_STDOUT_READY"));
    assert!(String::from_utf8_lossy(&result.process_output.stderr).contains("V01_STDERR_READY"));
    json!({
        "case":case, "ready_ms":ready_ms, "run_return_ms":elapsed.as_millis(),
        "after_trigger_ms":trigger.elapsed().as_millis(),
        "after_deadline_ms":if case == "deadline" { elapsed.saturating_sub(RUN_TIMEOUT).as_millis() } else { 0 },
        "deadline_ms":if case == "deadline" { RUN_TIMEOUT.as_millis() } else { 0 },
        "pipe_drain_ms":DRAIN.as_millis(), "pipes_forced_closed":result.process_output.pipes_forced_closed,
        "timed_out":result.timed_out, "canceled":result.canceled, "exit_code":result.exit_code,
        "stdout_ready":true, "stderr_ready":true,
    })
}

fn windows_version() -> Value {
    // RtlGetVersion reports the native build without application-manifest
    // version virtualization. This linked call exists only in this test binary.
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
    }
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    assert_eq!(unsafe { RtlGetVersion(&mut version) }, 0);
    json!({"major":version.dwMajorVersion, "minor":version.dwMinorVersion, "build":version.dwBuildNumber})
}

fn emit_receipt(report: Value) {
    // Raw stdout keeps the bounded permanent regression receipt in ordinary
    // successful CI logs, independently of libtest's print-macro capture.
    writeln!(io::stdout(), "V01_EVIDENCE {}", report).unwrap();
}

fn rust_case(case: &str) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let mut tree = Tree::start(
        fixture_command(root, "rust-runner", case),
        case == "failed-attach",
    );
    let processes = if case == "failed-attach" {
        None
    } else {
        tree.await_file(root, "grandchild-ready", STEP);
        let direct = tree.process(root, "direct.pid");
        let grandchild = tree.process(root, "grandchild.pid");
        assert_ne!(direct.pid, grandchild.pid);
        publish(
            root,
            "supervisor-ready",
            b"both owned process handles retained",
        );
        Some((direct, grandchild))
    };
    tree.wait(STEP);
    if let Some((direct, grandchild)) = &processes {
        direct.wait(RETURN_BOUND);
        grandchild.wait(RETURN_BOUND);
    }
    tree.assert_empty_before_cleanup();
    let mut report: Value =
        serde_json::from_slice(&fs::read(root.join("rust-result.json")).unwrap()).unwrap();
    report["os"] = windows_version();
    report["runtime_and_runner_exited"] = json!(true);
    report["owned_job_empty_before_harness_cleanup"] = json!(true);
    if let Some((direct, grandchild)) = processes {
        report["direct_pid"] = json!(direct.pid);
        report["grandchild_pid"] = json!(grandchild.pid);
        report["both_owned_handles_signaled"] = json!(true);
    }
    emit_receipt(report);
}

#[test]
fn headless_failed_native_job_attachment_prevents_provider_execution() {
    rust_case("failed-attach");
}

#[test]
fn headless_cancellation_reaps_native_grandchild_retaining_both_pipes() {
    rust_case("cancel");
}

#[test]
fn headless_deadline_reaps_native_grandchild_retaining_both_pipes() {
    rust_case("deadline");
}

#[test]
fn headless_direct_exit_bounds_drain_and_reaps_native_grandchild() {
    rust_case("normal");
}

#[derive(Deserialize)]
struct OracleManifest {
    schema: u32,
    go_source_sha: String,
    go_version: String,
    overlay_test: String,
    package: String,
    entry_test: String,
    build_tag: String,
    packages: Vec<String>,
    embed_patterns: BTreeMap<String, Vec<String>>,
    files: BTreeMap<String, String>,
}

fn oracle_manifest() -> OracleManifest {
    let manifest: OracleManifest = serde_json::from_str(include_str!(
        "fixtures/core/headless/v01_windows_oracle_sources.json"
    ))
    .unwrap();
    assert_eq!(manifest.schema, 1);
    assert_eq!(
        manifest.go_source_sha,
        "d8fbf8598c3effd4e2f837e43ad6f0488c461de3"
    );
    assert_eq!(manifest.go_version, "go version go1.26.8 windows/amd64");
    assert_eq!(manifest.package, "./internal/headless");
    assert_eq!(manifest.entry_test, "TestV01WindowsOracle");
    assert_eq!(manifest.build_tag, "many_ai_v01_oracle");
    assert_eq!(
        manifest.overlay_test,
        "internal/headless/v01_windows_oracle_test.go"
    );
    assert_eq!(
        manifest.embed_patterns,
        BTreeMap::from([(
            "internal/provider".into(),
            vec!["schema.json".into(), "manifests/*.json".into()]
        )])
    );
    for name in manifest.files.keys().chain(manifest.packages.iter()) {
        assert!(
            Path::new(name)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
            "oracle manifest contains a non-relative source path"
        );
    }
    manifest
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tool_output(plan: ProcessPlan, env_clear: bool) -> Vec<u8> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let output = runtime
        .block_on(run_capped_with_options(
            &plan,
            &Cancellation::default(),
            SpawnOptions {
                env_clear,
                no_window: true,
                ..Default::default()
            },
        ))
        .expect("owned read/build tool process");
    assert_eq!(
        output.outcome,
        ExitOutcome::Exited {
            code: Some(0),
            signal: None
        },
        "oracle preparation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout_truncated && !output.stderr_truncated && !output.pipes_forced_closed);
    output.stdout
}

fn preparation_environment(root: &Path) -> BTreeMap<OsString, Option<OsString>> {
    let mut env = environment(root, "oracle-build", "build");
    for key in ["PATH", "PATHEXT", "SystemDrive"] {
        if let Some(value) = std::env::var_os(key) {
            env.insert(key.into(), Some(value));
        }
    }
    env
}

fn materialize_committed_sources(
    repo: &Path,
    destination: &Path,
    manifest: &OracleManifest,
) -> String {
    let mut env = preparation_environment(destination.parent().unwrap());
    for (key, value) in [
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_GLOBAL", "NUL"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_OPTIONAL_LOCKS", "0"),
    ] {
        env.insert(key.into(), Some(value.into()));
    }
    let git = |args: Vec<OsString>, stdin: Vec<u8>| {
        // Built-in object readers do not run hooks, filters or textconv.
        // --no-lazy-fetch makes a missing local object a setup failure, never
        // permission to contact a promisor remote from this regression.
        let mut all = [
            "--no-replace-objects",
            "--no-lazy-fetch",
            "--no-optional-locks",
            "--no-pager",
        ]
        .map(Into::into)
        .to_vec();
        all.extend(args);
        tool_output(
            ProcessPlan {
                executable: "git".into(),
                args: all,
                cwd: repo.into(),
                env: env.clone(),
                stdin,
                timeout: Duration::from_secs(30),
                output_cap: 8 * 1024 * 1024,
                pipe_drain_timeout: Duration::from_secs(2),
            },
            true,
        )
    };
    let commit = String::from_utf8(git(
        vec!["rev-parse".into(), "--verify".into(), "HEAD".into()],
        vec![],
    ))
    .unwrap();
    let commit = commit.trim().to_owned();
    assert_eq!(commit.len(), 40);
    assert!(commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let mut args = vec![
        "ls-tree".into(),
        "-r".into(),
        "-z".into(),
        "--full-tree".into(),
        commit.clone().into(),
        "--".into(),
        "go.mod".into(),
        "go.sum".into(),
    ];
    args.extend(manifest.packages.iter().map(Into::into));
    let tree = git(args, vec![]);
    let mut blobs = BTreeMap::new();
    for entry in tree
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let text = std::str::from_utf8(entry).unwrap();
        let (meta, name) = text.split_once('\t').expect("Git ls-tree record");
        let path = Path::new(name);
        let package = path.parent().unwrap().to_str().unwrap().replace('\\', "/");
        let filename = path.file_name().unwrap().to_str().unwrap();
        let go_source = filename.ends_with(".go")
            && manifest.packages.contains(&package)
            && (package == "internal/headless" || !filename.ends_with("_test.go"));
        let embedded = name == "internal/provider/schema.json"
            || (package == "internal/provider/manifests" && filename.ends_with(".json"));
        if !go_source && !embedded && name != "go.mod" && name != "go.sum" {
            continue;
        }
        let fields: Vec<_> = meta.split(' ').collect();
        assert_eq!(fields.len(), 3);
        assert!(
            fields[0] == "100644" || fields[0] == "100755",
            "oracle source must be a regular committed file: {name}"
        );
        assert_eq!(fields[1], "blob");
        assert_eq!(fields[2].len(), 40);
        assert!(fields[2].bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(
            blobs
                .insert(name.to_owned(), fields[2].to_owned())
                .is_none()
        );
    }
    assert_eq!(
        blobs.keys().collect::<BTreeSet<_>>(),
        manifest.files.keys().collect::<BTreeSet<_>>(),
        "committed Go source/embed membership differs from the fixed oracle"
    );
    let request: String = blobs.values().map(|blob| format!("{blob}\n")).collect();
    let response = git(
        vec!["cat-file".into(), "--batch".into()],
        request.into_bytes(),
    );
    let mut remaining = response.as_slice();
    for (name, blob) in blobs {
        let newline = remaining
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("Git blob header");
        let header = std::str::from_utf8(&remaining[..newline]).unwrap();
        let fields: Vec<_> = header.split(' ').collect();
        assert_eq!(fields.len(), 3, "missing committed blob for {name}");
        assert_eq!(fields[0], blob);
        assert_eq!(fields[1], "blob");
        let length: usize = fields[2].parse().unwrap();
        remaining = &remaining[newline + 1..];
        assert!(
            remaining.len() > length,
            "truncated committed blob for {name}"
        );
        let bytes = &remaining[..length];
        assert_eq!(
            sha256(bytes),
            manifest.files[&name],
            "fixed Go blob changed: {name}"
        );
        if name.ends_with(".go") {
            assert!(
                !String::from_utf8_lossy(bytes).contains(&manifest.build_tag),
                "fixture tag must not change fixed production source selection"
            );
        }
        assert_eq!(remaining[length], b'\n');
        remaining = &remaining[length + 1..];
        let target = destination.join(name);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }
    assert!(remaining.is_empty(), "unexpected trailing Git batch data");
    commit
}

struct BuiltOracle {
    executable: PathBuf,
    manifest: OracleManifest,
    input_commit: String,
    helper_sha256: String,
}

fn build_oracle(root: &Path) -> BuiltOracle {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let manifest = oracle_manifest();
    let go: PathBuf = std::env::var_os("MANY_AI_GO_BINARY")
        .unwrap_or_else(|| "go".into())
        .into();
    // Resolve prepared caches before replacing HOME/USERPROFILE. This query
    // inherits the caller's cache settings and GOENV; it cannot fetch a toolchain
    // or dependencies. The subsequent compile uses only its measured paths.
    let cache_env = [
        ("GOTOOLCHAIN", "local"),
        ("GOWORK", "off"),
        ("GOTELEMETRY", "off"),
        ("GOPROXY", "off"),
        ("GOSUMDB", "off"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), Some(value.into())))
    .collect();
    let caches: BTreeMap<String, String> = serde_json::from_slice(&tool_output(
        ProcessPlan {
            executable: go.clone(),
            args: ["env", "-json", "GOCACHE", "GOMODCACHE", "GOPATH"]
                .map(Into::into)
                .to_vec(),
            cwd: repo.into(),
            env: cache_env,
            stdin: vec![],
            timeout: Duration::from_secs(30),
            output_cap: 16 * 1024,
            pipe_drain_timeout: Duration::from_secs(2),
        },
        false,
    ))
    .unwrap();
    for key in ["GOCACHE", "GOMODCACHE"] {
        assert!(
            Path::new(&caches[key]).is_absolute(),
            "prepared {key} must be an absolute cache directory"
        );
    }
    let source = root.join("verified-go-source");
    fs::create_dir(&source).unwrap();
    let input_commit = materialize_committed_sources(repo, &source, &manifest);
    assert!(!source.join(&manifest.overlay_test).exists());
    // These are the helper bytes compiled into this Rust test binary, not a
    // later working-tree read. Only the fixed production inputs come from Git.
    let helper_bytes = include_bytes!("fixtures/core/headless/v01_windows_oracle_test.go");
    let helper_sha256 = sha256(helper_bytes);
    let helper = root.join("v01_windows_oracle_test.go");
    fs::write(&helper, helper_bytes).unwrap();
    let overlay = root.join("overlay.json");
    fs::write(
        &overlay,
        serde_json::to_vec(&json!({"Replace":{
            source.join(&manifest.overlay_test).to_str().unwrap(): helper.to_str().unwrap()
        }}))
        .unwrap(),
    )
    .unwrap();
    let executable = root.join("headless-v01.exe");
    let mut env = preparation_environment(root);
    for key in ["GOCACHE", "GOMODCACHE", "GOPATH"] {
        env.insert(key.into(), Some(caches[key].clone().into()));
    }
    for (key, value) in [
        ("GOTOOLCHAIN", "local"),
        ("GOWORK", "off"),
        ("GOENV", "off"),
        ("GOTELEMETRY", "off"),
        ("GOPROXY", "off"),
        ("GOSUMDB", "off"),
        ("CGO_ENABLED", "0"),
    ] {
        env.insert(key.into(), Some(value.into()));
    }
    let invoke = |args: Vec<OsString>| {
        tool_output(
            ProcessPlan {
                executable: go.clone(),
                args,
                cwd: source.clone(),
                env: env.clone(),
                stdin: vec![],
                timeout: Duration::from_secs(180),
                output_cap: 2 * 1024 * 1024,
                pipe_drain_timeout: Duration::from_secs(2),
            },
            true,
        )
    };
    assert_eq!(
        String::from_utf8(invoke(vec!["version".into()]))
            .unwrap()
            .trim(),
        manifest.go_version
    );
    invoke(vec![
        "test".into(),
        "-c".into(),
        "-mod=readonly".into(),
        "-buildvcs=false".into(),
        format!("-tags={}", manifest.build_tag).into(),
        format!("-overlay={}", overlay.display()).into(),
        "-o".into(),
        executable.as_os_str().to_owned(),
        manifest.package.clone().into(),
    ]);
    assert!(executable.is_file());
    BuiltOracle {
        executable,
        manifest,
        input_commit,
        helper_sha256,
    }
}

fn go_case(oracle: &BuiltOracle, root: &Path, case: &str) {
    let manifest = &oracle.manifest;
    fs::create_dir(root).unwrap();
    let mut command = Command::new(&oracle.executable);
    command
        .args([
            format!("-test.run=^{}$", manifest.entry_test),
            "-test.v".into(),
        ])
        .current_dir(root)
        .env_clear();
    for (key, value) in environment(root, "go-runner", case) {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    command
        .env("MANY_AI_V01_FIXTURE", std::env::current_exe().unwrap())
        .env(
            "MANY_AI_V01_TIMEOUT_MS",
            RUN_TIMEOUT.as_millis().to_string(),
        );
    let mut tree = Tree::start(command, true);
    tree.await_file(root, "go-ready.json", STEP);
    let ready: Value =
        serde_json::from_slice(&fs::read(root.join("go-ready.json")).unwrap()).unwrap();
    assert_eq!(ready["go_source_sha"], manifest.go_source_sha);
    assert_eq!(ready["go_version"], "go1.26.8");
    assert_eq!(ready["case"], case);
    assert_eq!(ready["native_attach_probe_error"], ERROR_ACCESS_DENIED);
    assert_eq!(
        ready["probe_scope"],
        "diagnostic_second_attach_on_live_run_child"
    );
    assert_eq!(ready["original_run_attach_error_directly_observed"], false);
    assert_eq!(
        ready["current_job_ui_restrictions"],
        JOB_OBJECT_UILIMIT_READCLIPBOARD
    );
    assert_eq!(
        ready["deadline_ms"],
        if case == "deadline" {
            RUN_TIMEOUT.as_millis() as u64
        } else {
            0
        }
    );
    assert_eq!(ready["stdout_ready"], true);
    assert_eq!(ready["stderr_ready"], true);
    let direct = tree.process(root, "direct.pid");
    let grandchild = tree.process(root, "grandchild.pid");
    assert_eq!(ready["direct_pid"], direct.pid);
    assert_eq!(ready["grandchild_pid"], grandchild.pid);
    if case == "deadline" {
        assert!(ready["run_start_elapsed_ms"].as_u64().unwrap() < RUN_TIMEOUT.as_millis() as u64);
    }
    let triggered = Instant::now();
    if case == "cancel" {
        publish(root, "cancel-run", b"cancel actual Go Run");
    }
    direct.wait(RUN_TIMEOUT + RETURN_BOUND);
    assert!(
        !grandchild.exited(),
        "Go canceled its grandchild despite failed attachment"
    );
    assert!(
        !tree.finished(),
        "Go Run returned before retained-pipe reproduction"
    );
    publish(
        root,
        "probe-retained-pipes",
        b"write both pipes after direct HANDLE signaled",
    );
    tree.await_file(root, "go-stdout-retained", RETURN_BOUND);
    tree.await_file(root, "go-stderr-retained", RETURN_BOUND);
    // Positive writes received on BOTH inherited streams after the direct
    // process exited establish the cause. An elapsed test timeout alone is
    // never accepted as reproduction or as successful product cancellation.
    let observation = Instant::now();
    while observation.elapsed() < DRAIN {
        assert!(!grandchild.exited());
        assert!(!tree.finished());
        assert!(!root.join("go-run-returned").exists());
        assert!(!root.join("go-result.json").exists());
        thread::sleep(POLL);
    }
    let blocked_observation_ms = observation.elapsed().as_millis();
    let before_cleanup_ms = triggered.elapsed().as_millis();
    publish(
        root,
        "harness-cleanup",
        b"supervisor will terminate retained grandchild handle",
    );
    grandchild.terminate();
    grandchild.wait(RETURN_BOUND);
    tree.wait(RETURN_BOUND);
    tree.assert_empty_before_cleanup();
    let result: Value =
        serde_json::from_slice(&fs::read(root.join("go-result.json")).unwrap()).unwrap();
    assert_eq!(result["go_source_sha"], manifest.go_source_sha);
    assert_eq!(result["go_version"], "go1.26.8");
    assert_eq!(result["case"], case);
    assert_eq!(result["state"], "error");
    assert_eq!(result["timed_out"], case == "deadline");
    assert_eq!(result["canceled"], case == "cancel");
    assert_eq!(result["cleanup_release_observed"], true);
    assert_eq!(result["error"], "");
    assert_eq!(result["direct_exited"], true);
    assert_eq!(result["grandchild_exited"], true);
    let return_ms = result["run_return_ms"].as_u64().unwrap();
    if case == "deadline" {
        assert!(return_ms >= RUN_TIMEOUT.as_millis() as u64);
    }
    emit_receipt(json!({
        "case":format!("go-{case}-failed-attachment-retained-pipes"),
        "go_source_sha":manifest.go_source_sha, "go_version":manifest.go_version,
        "verified_input_source_commit":oracle.input_commit, "overlay_helper_sha256":oracle.helper_sha256,
        "os":windows_version(), "ready":ready, "result":result,
        "direct_handle_signaled_before_probe":true, "grandchild_alive_after_direct_exit":true,
        "stdout_retained_probe_received":true, "stderr_retained_probe_received":true,
        "run_pending_before_harness_cleanup":true, "before_cleanup_ms":before_cleanup_ms,
        "blocked_observation_ms":blocked_observation_ms,
        "after_deadline_return_ms":if case == "deadline" { return_ms - RUN_TIMEOUT.as_millis() as u64 } else { 0 },
        "cleanup_owner":"supervisor retained grandchild HANDLE; not Go product cleanup",
        "both_owned_handles_signaled":true, "runtime_and_runner_exited":true,
        "owned_job_empty_before_harness_job_cleanup":true,
        "baseline_cancellation_accepted":false,
    }));
}

#[test]
fn fixed_go_failed_job_attachment_retains_grandchild_pipes_until_owned_harness_cleanup() {
    let temporary = tempfile::tempdir().unwrap();
    let oracle = build_oracle(temporary.path());
    for case in ["cancel", "deadline"] {
        go_case(&oracle, &temporary.path().join(case), case);
    }
}
