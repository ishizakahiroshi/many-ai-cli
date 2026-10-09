use super::{ServiceRouter, cli_version::*, task_owner::HubTaskOwner, transport};
use crate::{
    application::spawn_policy::PathEnvironment,
    config::{Config, ConfigStore, RuntimePaths},
    process::{
        Cancellation,
        execpath::{ExecutableFs, Platform},
    },
    profile::store::ProviderRegistryStore,
    proto::{
        core::{CoreEffectFailure, CoreEffectSink, CoreEffects, CoreFuture},
        time::UNIX_EPOCH,
    },
};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};

struct Sink;
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        assert!(effects.0.is_empty());
        Box::pin(async { Ok(()) })
    }
}
struct Fs;
impl ExecutableFs for Fs {
    fn exists(&self, _: &str) -> bool {
        true
    }
    fn is_executable(&self, _: &str, _: Platform) -> bool {
        true
    }
    fn read(&self, _: &str) -> io::Result<Vec<u8>> {
        Ok(vec![])
    }
}
struct Environment;
impl PathEnvironment for Environment {
    fn sanitize(&self, environment: &[String]) -> io::Result<Vec<String>> {
        Ok(environment.to_vec())
    }
}
struct Executor {
    calls: AtomicUsize,
    started: Notify,
    gate: Semaphore,
}
impl CliVersionExecutor for Executor {
    fn execute(
        &self,
        _: CliVersionCommand,
    ) -> CoreFuture<'_, Result<CliVersionProcessOutput, CliVersionFailure>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            self.gate.acquire().await.unwrap().forget();
            Ok(CliVersionProcessOutput {
                output: b"synthetic v1\n".to_vec(),
                ..Default::default()
            })
        })
    }
}

#[tokio::test]
async fn served_versions_keep_owned_run_after_http_waiter_disappears_and_share_cache() {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), port, installed.path()).unwrap();
    let config = Arc::new(
        ConfigStore::new(
            paths.clone(),
            Config {
                token: "synthetic-version-token".into(),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let store = Arc::new(ProviderRegistryStore::new(&paths, config.clone()).unwrap());
    let executor = Arc::new(Executor {
        calls: AtomicUsize::new(0),
        started: Notify::new(),
        gate: Semaphore::new(0),
    });
    let versions = Arc::new(CliVersionHttp::new(CliVersionDependencies {
        executor: Some(executor.clone()),
        cwd: "/synthetic".into(),
        environment: vec!["PATH=/synthetic/bin".into()],
        platform: Platform::Unix,
        path_environment: Arc::new(Environment),
        executable_fs: Arc::new(Fs),
        modified_at: Arc::new(|_| None),
        clock: Arc::new(|| UNIX_EPOCH + Duration::from_secs(100)),
    }));
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let router = Arc::new(
        ServiceRouter::isolated_at_port(config, paths, port)
            .unwrap()
            .with_provider_routes(
                store,
                Arc::new(|_| true),
                Arc::new(|_, _, _| panic!("unexpected store warning")),
            )
            .with_cli_versions(versions.clone())
            .with_task_owner(tasks.handle()),
    );
    let cancel = Cancellation::default();
    let server = tokio::spawn(transport::serve(
        listener,
        router,
        Arc::new(Sink),
        cancel.clone(),
    ));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}/api/cli-versions");
    assert_eq!(
        client
            .post(&base)
            .body("{bad")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let authorized = format!("{base}?token=synthetic-version-token");
    assert_eq!(client.put(&authorized).send().await.unwrap().status(), 405);
    assert_eq!(
        client
            .post(&authorized)
            .body("{bad")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    let empty: serde_json::Value = client
        .get(&authorized)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty, serde_json::json!({"checked_at":"", "results":[]}));
    let first_client = client.clone();
    let first_url = authorized.clone();
    let first = tokio::spawn(async move {
        first_client
            .post(first_url)
            .json(&serde_json::json!({"providers":["codex"]}))
            .send()
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), executor.started.notified())
        .await
        .unwrap();
    first.abort();
    let _ = first.await;
    let joined_client = client.clone();
    let joined_url = authorized.clone();
    let joined = tokio::spawn(async move {
        joined_client
            .post(joined_url)
            .json(&serde_json::json!({"providers":["claude"]}))
            .send()
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while tasks.snapshot().requests.running < 2 {
            tokio::task::yield_now().await;
        }
        // Both requests have entered owned work. The follower is blocked on
        // the leader's shared flight rather than a second process operation.
        while versions.inflight_waiter_count_for_test() < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    executor.gate.add_permits(1);
    let response = joined.await.unwrap().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let result: serde_json::Value = response.json().await.unwrap();
    assert_eq!(result["results"][0]["provider"], "codex");
    assert_eq!(result["results"][0]["version_text"], "synthetic v1\n");
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let cached: serde_json::Value = client
        .get(&authorized)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cached, result);
    tasks.stop_requests();
    tokio::time::timeout(Duration::from_secs(3), tasks.drain_requests())
        .await
        .unwrap();
    cancel.cancel();
    server.await.unwrap().unwrap();
}
