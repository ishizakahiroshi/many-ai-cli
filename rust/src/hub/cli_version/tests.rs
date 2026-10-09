use super::*;
use crate::{
    profile::registry::default_adapters,
    proto::{
        provider::{LaunchDefinition, Layers},
        time::parse_rfc3339,
    },
};
use base64::Engine;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, Semaphore};

#[derive(Default)]
struct FakeFs {
    files: BTreeMap<String, Vec<u8>>,
}
impl ExecutableFs for FakeFs {
    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
    fn is_executable(&self, path: &str, _: Platform) -> bool {
        self.files.contains_key(path)
    }
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}
struct IdentityEnvironment;
impl PathEnvironment for IdentityEnvironment {
    fn sanitize(&self, environment: &[String]) -> io::Result<Vec<String>> {
        Ok(environment.to_vec())
    }
}
#[derive(Default)]
struct FakeExecutor {
    outputs: BTreeMap<String, CliVersionProcessOutput>,
    calls: Mutex<Vec<Value>>,
    active: AtomicUsize,
    maximum: AtomicUsize,
    started: Notify,
    gate: Option<Semaphore>,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl CliVersionExecutor for FakeExecutor {
    fn execute(
        &self,
        command: CliVersionCommand,
    ) -> CoreFuture<'_, Result<CliVersionProcessOutput, CliVersionFailure>> {
        Box::pin(async move {
            assert!(
                command
                    .environment
                    .iter()
                    .all(|entry| !entry.starts_with("HOME="))
            );
            assert_eq!(command.timeout, CHECK_TIMEOUT);
            assert_eq!(command.output_cap, OUTPUT_CAP);
            self.calls.lock().unwrap().push(json!({"executable":command.executable,"args":command.args,"timeout_ms":command.timeout.as_millis(),"output_cap":command.output_cap}));
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.started.notify_waiters();
            if let Some(gate) = &self.gate {
                gate.acquire().await.unwrap().forget();
            }
            Ok(self
                .outputs
                .get(&command.executable)
                .cloned()
                .unwrap_or_else(|| CliVersionProcessOutput {
                    output: b"synthetic version\n".to_vec(),
                    ..Default::default()
                }))
        })
    }
}
fn definition(id: &str) -> Definition {
    Definition {
        schema_version: 1,
        id: id.into(),
        display_name: format!("Synthetic {id}"),
        launch: Some(LaunchDefinition {
            executable: id.into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn registry(definitions: Vec<Definition>) -> Registry {
    let result = Registry::build(
        Layers {
            embedded: Some(definitions),
            ..Default::default()
        },
        &default_adapters(),
    );
    assert!(
        result
            .diagnostics()
            .iter()
            .all(|d| d.severity.as_str() != "error"),
        "invalid synthetic definition"
    );
    result
}
fn dependencies(
    executor: Option<Arc<dyn CliVersionExecutor>>,
    fs: FakeFs,
    modified: BTreeMap<String, Timestamp>,
) -> CliVersionDependencies {
    CliVersionDependencies {
        executor,
        cwd: "/synthetic".into(),
        environment: vec!["PATH=/synthetic/bin".into()],
        platform: Platform::Unix,
        path_environment: Arc::new(IdentityEnvironment),
        executable_fs: Arc::new(fs),
        modified_at: Arc::new(move |exe| modified.get(exe).copied()),
        clock: Arc::new(|| parse_rfc3339("2026-10-04T02:03:04.567890000+09:00").unwrap()),
    }
}
fn request(method: &str, body: &str) -> Request {
    Request {
        method: method.into(),
        path: PATH.into(),
        body: body.as_bytes().to_vec(),
        ..Default::default()
    }
}
#[tokio::test]
async fn pinned_go_caller_observations() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/services/cli-version/cases.json"
    ))
    .unwrap();
    let observations: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/services/cli-version/go_observations.json"
    ))
    .unwrap();
    assert_eq!(
        observations["baseline"],
        "21d0bc7935a2c4696fb89ccff2e324157a528c2d"
    );
    let observations = observations["cases"].as_array().unwrap();
    assert_eq!(cases.len(), observations.len());
    for (case, expected) in cases.iter().zip(observations) {
        let name = case["name"].as_str().unwrap();
        assert_eq!(case["name"], expected["name"]);
        let mut outputs = BTreeMap::new();
        for (path, output) in case["processes"].as_object().unwrap() {
            let start_failed = output["start_failed"].as_bool().unwrap_or(false);
            outputs.insert(
                path.clone(),
                CliVersionProcessOutput {
                    output: output["output"]
                        .as_str()
                        .map(|s| base64::engine::general_purpose::STANDARD.decode(s).unwrap())
                        .unwrap_or_default(),
                    exit_code: if start_failed {
                        0
                    } else {
                        output["exit_code"].as_i64().unwrap_or(0) as i32
                    },
                    timed_out: !start_failed && output["timed_out"].as_bool().unwrap_or(false),
                    start_failed,
                },
            );
        }
        let executor = Arc::new(FakeExecutor {
            outputs,
            ..Default::default()
        });
        let fs = FakeFs {
            files: case["lookup"]
                .as_object()
                .unwrap()
                .values()
                .map(|p| (p.as_str().unwrap().into(), vec![]))
                .collect(),
        };
        let modified = case["modified"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(p, t)| (p.clone(), parse_rfc3339(t.as_str().unwrap()).unwrap()))
            .collect();
        let service = CliVersionHttp::new(dependencies(Some(executor.clone()), fs, modified));
        match case["kind"].as_str().unwrap() {
            "one" => {
                let def: Definition = serde_json::from_value(case["definition"].clone()).unwrap();
                let result = service.check_one(&def.id, &def).await.unwrap();
                assert_eq!(
                    serde_json::to_value(result).unwrap(),
                    expected["result"],
                    "{name}"
                );
            }
            "line" => assert_eq!(
                first_non_empty_line(case["text"].as_str().unwrap()),
                expected["line"],
                "{name}"
            ),
            "normalize" => {
                let ids: Vec<String> = serde_json::from_value(case["ids"].clone()).unwrap();
                let result = normalize_provider_ids(&ids);
                // Go nil and empty slices differ internally; POST consumes only length/order.
                assert_eq!(
                    json!(result),
                    if expected["ids"].is_null() {
                        json!([])
                    } else {
                        expected["ids"].clone()
                    },
                    "{name}"
                );
            }
            _ => {
                let registry =
                    registry(serde_json::from_value(case["definitions"].clone()).unwrap());
                let body = match case["body_mode"].as_str() {
                    Some("limit") => format!("{}{{}}", " ".repeat(1048576)),
                    Some("trailing") => format!("{{}}{}", " ".repeat(1048577)),
                    _ => case["body"].as_str().unwrap().to_owned(),
                };
                let mut input = request(case["method"].as_str().unwrap(), &body);
                if let Some(length) = case["content_length"].as_u64() {
                    input
                        .headers
                        .push(("Content-Length".into(), length.to_string()));
                }
                if case["chunked"].as_bool().unwrap_or(false) {
                    input
                        .headers
                        .push(("Transfer-Encoding".into(), "chunked".into()));
                }
                let response = service
                    .handle_authenticated(
                        &input,
                        (!case["registry_missing"].as_bool().unwrap_or(false)).then_some(&registry),
                    )
                    .await;
                assert_eq!(
                    response.status,
                    expected["status"].as_u64().unwrap() as u16,
                    "{name}"
                );
                assert_eq!(
                    serde_json::from_slice::<Value>(&response.body).unwrap(),
                    expected["body"],
                    "{name}"
                );
                assert_eq!(
                    response
                        .headers
                        .get("Cache-Control")
                        .map(String::as_str)
                        .unwrap_or(""),
                    expected["cache_control"].as_str().unwrap(),
                    "{name}"
                );
                assert_eq!(
                    serde_json::to_value(service.last_result()).unwrap(),
                    expected["cached"],
                    "{name}"
                );
            }
        }
        let mut calls = executor.calls.lock().unwrap().clone();
        calls.sort_by(|a, b| a["executable"].as_str().cmp(&b["executable"].as_str()));
        assert_eq!(json!(calls), expected["calls"], "{name}");
    }
}
fn controlled(count: usize) -> (Arc<CliVersionHttp>, Arc<FakeExecutor>, Arc<Registry>) {
    let definitions: Vec<_> = (0..count)
        .map(|i| definition(&format!("synthetic-{i:02}")))
        .collect();
    let fs = FakeFs {
        files: definitions
            .iter()
            .map(|d| (format!("/synthetic/bin/{}", d.id), vec![]))
            .collect(),
    };
    let executor = Arc::new(FakeExecutor {
        gate: Some(Semaphore::new(0)),
        ..Default::default()
    });
    let service = Arc::new(CliVersionHttp::new(dependencies(
        Some(executor.clone()),
        fs,
        BTreeMap::new(),
    )));
    (service, executor, Arc::new(registry(definitions)))
}
async fn wait_calls(executor: &FakeExecutor, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let notified = executor.started.notified();
            if executor.calls.lock().unwrap().len() >= count {
                return;
            }
            notified.await;
        }
    })
    .await
    .unwrap();
}
async fn wait_follower(service: &CliVersionHttp) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if service
                .state
                .lock()
                .unwrap()
                .inflight
                .as_ref()
                .is_some_and(|f| f.receiver_count() > 0)
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn default_selection_and_eight_way_fanout_preserve_request_order() {
    let (service, executor, registry) = controlled(19);
    let owner = service.clone();
    let snapshot = registry.clone();
    let task = tokio::spawn(async move { owner.run(&snapshot, vec![]).await });
    wait_calls(&executor, 8).await;
    assert_eq!(executor.active.load(Ordering::SeqCst), 8);
    assert_eq!(executor.calls.lock().unwrap().len(), 8);
    executor.gate.as_ref().unwrap().add_permits(19);
    let response = task.await.unwrap().unwrap();
    assert_eq!(
        response
            .results
            .iter()
            .map(|r| r.provider.clone())
            .collect::<Vec<_>>(),
        all_provider_ids(&registry)
    );
    assert_eq!(executor.maximum.load(Ordering::SeqCst), 8);
    assert_eq!(executor.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn single_flight_ignores_joiner_selection_and_get_keeps_last_until_completion() {
    let (service, executor, registry) = controlled(2);
    executor.gate.as_ref().unwrap().add_permits(1);
    let old = service
        .run(&registry, vec!["synthetic-00".into()])
        .await
        .unwrap();
    let first_service = service.clone();
    let first_registry = registry.clone();
    let first = tokio::spawn(async move {
        first_service
            .handle_authenticated(
                &request("POST", r#"{"providers":["synthetic-01"]}"#),
                Some(&first_registry),
            )
            .await
    });
    wait_calls(&executor, 2).await;
    let get = service
        .handle_authenticated(&request("GET", ""), None)
        .await;
    assert_eq!(
        serde_json::from_slice::<CliVersionResponse>(&get.body).unwrap(),
        old
    );
    let second_service = service.clone();
    let second_registry = registry.clone();
    let second = tokio::spawn(async move {
        second_service
            .handle_authenticated(
                &request("POST", r#"{"providers":["synthetic-00"]}"#),
                Some(&second_registry),
            )
            .await
    });
    wait_follower(&service).await;
    executor.gate.as_ref().unwrap().add_permits(1);
    let a = first.await.unwrap();
    let b = second.await.unwrap();
    assert_eq!(a.body, b.body);
    assert_eq!(executor.calls.lock().unwrap().len(), 2);
    assert_eq!(
        service.last_result().unwrap().results[0].provider,
        "synthetic-01"
    );
    // Returned values do not provide mutable access to the shared cached result.
    let mut clone = service.last_result().unwrap();
    clone.results.clear();
    assert_eq!(service.last_result().unwrap().results.len(), 1);
}
#[tokio::test]
async fn dropped_owner_wakes_joiners_retains_cache_and_allows_retry() {
    let (service, executor, registry) = controlled(1);
    executor.gate.as_ref().unwrap().add_permits(1);
    let old = service.run(&registry, vec![]).await.unwrap();
    let owner = service.clone();
    let snapshot = registry.clone();
    let first = tokio::spawn(async move { owner.run(&snapshot, vec![]).await });
    wait_calls(&executor, 2).await;
    let owner = service.clone();
    let snapshot = registry.clone();
    let follower = tokio::spawn(async move { owner.run(&snapshot, vec!["missing".into()]).await });
    wait_follower(&service).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert_eq!(
        follower.await.unwrap(),
        Err(CliVersionFailure::OwnerDropped)
    );
    assert_eq!(executor.active.load(Ordering::SeqCst), 0);
    assert_eq!(service.last_result(), Some(old));
    executor.gate.as_ref().unwrap().add_permits(1);
    assert!(service.run(&registry, vec![]).await.is_ok());
    assert_eq!(executor.calls.lock().unwrap().len(), 3);
}
#[tokio::test]
async fn unavailable_executor_is_explicit_and_cache_get_never_requires_it() {
    let fs = FakeFs {
        files: [("/synthetic/bin/alpha".into(), vec![])]
            .into_iter()
            .collect(),
    };
    let service = CliVersionHttp::new(dependencies(None, fs, BTreeMap::new()));
    let registry = registry(vec![definition("alpha")]);
    let response = service
        .handle_authenticated(&request("POST", ""), Some(&registry))
        .await;
    assert_eq!(response.status, 503);
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap()["error"],
        "cli_version_runner_unavailable"
    );
    assert!(service.last_result().is_none());
    let response = service
        .handle_authenticated(&request("GET", ""), None)
        .await;
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap(),
        json!({"checked_at":"","results":[]})
    );
    assert_eq!(
        service
            .handle_authenticated(&request("POST", "{bad"), None)
            .await
            .status,
        400
    );
}
#[tokio::test]
async fn windows_shim_uses_shared_resolver_and_stats_actual_executable() {
    let shim = r"C:\synthetic\bin\alpha.cmd";
    let exe = r"C:\synthetic\bin\actual.exe";
    let fs = FakeFs {
        files: [
            (shim.into(), br#""%~dp0\actual.exe" %*"#.to_vec()),
            (exe.into(), vec![]),
        ]
        .into_iter()
        .collect(),
    };
    let executor = Arc::new(FakeExecutor::default());
    let mut deps = dependencies(
        Some(executor.clone()),
        fs,
        [(
            exe.into(),
            parse_rfc3339("2026-09-30T01:02:03.999Z").unwrap(),
        )]
        .into_iter()
        .collect(),
    );
    deps.platform = Platform::Windows;
    deps.cwd = r"C:\synthetic".into();
    deps.environment = vec![r"Path=C:\synthetic\bin".into(), "PATHEXT=.CMD;.EXE".into()];
    let service = CliVersionHttp::new(deps);
    let result = service
        .check_one("alpha", &definition("alpha"))
        .await
        .unwrap();
    assert_eq!(result.executable, exe);
    assert_eq!(result.executable_modified_at, "2026-09-30T01:02:03Z");
    assert_eq!(
        executor.calls.lock().unwrap()[0]["args"],
        json!(["--version"])
    );
}
#[tokio::test]
async fn path_context_is_injected_and_sanitized_without_mutating_ambient_environment() {
    struct Sanitizer;
    impl PathEnvironment for Sanitizer {
        fn sanitize(&self, environment: &[String]) -> io::Result<Vec<String>> {
            assert_eq!(
                environment,
                [
                    "PATH=relative::/stale".to_string(),
                    "SYNTHETIC=kept".to_string()
                ]
            );
            Ok(vec![
                "PATH=relative::/synthetic/bin".into(),
                "SYNTHETIC=kept".into(),
            ])
        }
    }
    struct Executor;
    impl CliVersionExecutor for Executor {
        fn execute(
            &self,
            command: CliVersionCommand,
        ) -> CoreFuture<'_, Result<CliVersionProcessOutput, CliVersionFailure>> {
            assert_eq!(
                command.environment,
                vec!["PATH=relative::/synthetic/bin", "SYNTHETIC=kept"]
            );
            assert_eq!(command.cwd, PathBuf::from("/synthetic"));
            assert_eq!(command.args, vec!["--version"]);
            Box::pin(async {
                Ok(CliVersionProcessOutput {
                    output: b"synthetic\n".to_vec(),
                    ..Default::default()
                })
            })
        }
    }
    let fs = FakeFs {
        files: [
            ("/synthetic/bin/alpha".into(), vec![]),
            ("/synthetic/relative/alpha".into(), vec![]),
        ]
        .into_iter()
        .collect(),
    };
    let mut deps = dependencies(Some(Arc::new(Executor)), fs, BTreeMap::new());
    deps.path_environment = Arc::new(Sanitizer);
    deps.environment = vec!["PATH=relative::/stale".into(), "SYNTHETIC=kept".into()];
    let result = CliVersionHttp::new(deps)
        .check_one("alpha", &definition("alpha"))
        .await
        .unwrap();
    assert_eq!(result.executable, "/synthetic/bin/alpha");
}
