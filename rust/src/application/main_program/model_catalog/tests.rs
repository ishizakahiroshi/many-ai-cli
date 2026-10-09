use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[test]
fn body_limit_is_json_prefix_eof_instead_of_oversize_rejection() {
    let cap = 256 * 1024;
    let mut complete = br#"{"anthropic":[{"id":"synthetic"}]}"#.to_vec();
    complete.resize(cap, b' ');
    complete.extend_from_slice(b"ignored invalid tail");
    let mut prefix = Vec::new();
    assert!(append_prefix(&mut prefix, &complete, cap));
    assert_eq!(prefix.len(), cap);
    assert_eq!(
        parsers::defaults(&prefix).unwrap()["anthropic"][0].id,
        "synthetic"
    );
    let mut incomplete = br#"{"anthropic":[{"id":""#.to_vec();
    incomplete.resize(cap + 16, b'x');
    incomplete.extend_from_slice(br#""}]}"#);
    let mut prefix = Vec::new();
    for chunk in incomplete.chunks(97) {
        if append_prefix(&mut prefix, chunk, cap) {
            break;
        }
    }
    assert_eq!(prefix.len(), cap);
    assert!(parsers::defaults(&prefix).is_err());
}
struct Fake {
    defaults: AtomicUsize,
    native: AtomicUsize,
    nvidia: AtomicUsize,
    failed: AtomicBool,
    key: Mutex<String>,
}
struct GatedIo {
    delegate: Arc<Fake>,
    pause: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
    requests: AtomicUsize,
}
impl GatedIo {
    fn new(delegate: Arc<Fake>) -> Arc<Self> {
        Arc::new(Self {
            delegate,
            pause: AtomicBool::new(true),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Semaphore::new(0),
            requests: AtomicUsize::new(0),
        })
    }
    async fn models(&self) -> io::Result<Vec<Model>> {
        let sequence = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
        if self.pause.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
        }
        Ok(vec![model(&format!("observed-{sequence}"))])
    }
}
impl CatalogIo for GatedIo {
    fn invalidate_local(&self) -> CoreFuture<'_, io::Result<()>> {
        self.delegate.invalidate_local()
    }
    fn defaults<'a>(
        &'a self,
        source: &'a str,
    ) -> CoreFuture<'a, io::Result<BTreeMap<String, Vec<Model>>>> {
        self.delegate.defaults(source)
    }
    fn native<'a>(
        &'a self,
        provider: &'a str,
        force: bool,
    ) -> CoreFuture<'a, io::Result<Vec<Model>>> {
        if provider == "cursor-agent" {
            Box::pin(self.models())
        } else {
            self.delegate.native(provider, force)
        }
    }
    fn nvidia<'a>(&'a self, _: &'a str) -> CoreFuture<'a, io::Result<Vec<Model>>> {
        Box::pin(self.models())
    }
    fn nvidia_key(&self) -> io::Result<String> {
        Ok(String::new())
    }
    fn local<'a>(
        &'a self,
        config: &'a Config,
        kind: super::super::model_cache::LocalCatalog,
        force: bool,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<ObservedModels>> {
        self.delegate.local(config, kind, force, cancel)
    }
}
#[tokio::test]
async fn concurrent_catalog_requests_share_real_gated_native_fetch_without_whole_request_lock() {
    let (_root, config, delegate, _owner) = fixture();
    let io = GatedIo::new(delegate.clone());
    let owner = Arc::new(ModelsCatalog::new(config, io.clone()));
    let first = {
        let owner = owner.clone();
        tokio::spawn(async move {
            owner
                .response(false, &Cancellation::default())
                .await
                .unwrap()
        })
    };
    io.entered.notified().await;
    let cancel = Cancellation::default();
    let second = owner.response(false, &cancel);
    tokio::pin!(second);
    assert!(futures_util::poll!(&mut second).is_pending());
    assert_eq!(delegate.defaults.load(Ordering::SeqCst), 1);
    assert_eq!(io.requests.load(Ordering::SeqCst), 1);
    assert!(owner.cursor.state.lock().unwrap().flight.is_some());
    io.release.add_permits(1);
    let first = first.await.unwrap();
    let second = second.await.unwrap();
    assert_eq!(first.groups[3].models, second.groups[3].models);
    assert_eq!(io.requests.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn native_fresh_hit_does_not_wait_for_refresh_and_detached_old_generation_cannot_overwrite_new()
 {
    let (_root, _config, delegate, _owner) = fixture();
    let io = GatedIo::new(delegate);
    io.pause.store(false, Ordering::SeqCst);
    let cache = Arc::new(Cache::new(600, 180, FlightMode::GenerationDetach));
    let initial = cache
        .get(Vec::new(), false, false, io.native("cursor-agent", false))
        .await;
    assert_eq!(initial.value[0].id, "observed-1");
    io.pause.store(true, Ordering::SeqCst);
    let old = {
        let cache = cache.clone();
        let io = io.clone();
        tokio::spawn(async move {
            cache
                .get(Vec::new(), true, false, io.native("cursor-agent", true))
                .await
        })
    };
    io.entered.notified().await;
    // Fresh cached reads do not consume or wait for the actual refresh future.
    let hit = cache
        .get(Vec::new(), false, false, async {
            panic!("fresh hit must not fetch")
        })
        .await;
    assert_eq!(hit.value[0].id, "observed-1");
    cache.invalidate();
    io.pause.store(false, Ordering::SeqCst);
    let latest = cache
        .get(Vec::new(), true, false, io.native("cursor-agent", true))
        .await;
    assert_eq!(latest.value[0].id, "observed-3");
    io.release.add_permits(1);
    assert_eq!(old.await.unwrap().value[0].id, "observed-2");
    assert_eq!(
        cache
            .get(Vec::new(), false, false, async {
                panic!("current generation cached")
            })
            .await
            .value[0]
            .id,
        "observed-3"
    );
}
#[tokio::test]
async fn nvidia_changed_generation_waits_then_fetches_own_key_and_dropped_fetch_owner_unstrands_waiter()
 {
    let (_root, _config, delegate, _owner) = fixture();
    let io = GatedIo::new(delegate);
    let cache = Arc::new(Cache::new(600, 60, FlightMode::GenerationWait));
    let first = {
        let cache = cache.clone();
        let io = io.clone();
        tokio::spawn(async move {
            cache
                .get(
                    b"old-key-fingerprint".to_vec(),
                    false,
                    false,
                    io.nvidia("synthetic-old"),
                )
                .await
        })
    };
    io.entered.notified().await;
    cache.invalidate();
    let second = cache.get(
        b"new-key-fingerprint".to_vec(),
        true,
        false,
        io.nvidia("synthetic-new"),
    );
    tokio::pin!(second);
    assert!(futures_util::poll!(&mut second).is_pending());
    assert_eq!(io.requests.load(Ordering::SeqCst), 1);
    io.pause.store(false, Ordering::SeqCst);
    io.release.add_permits(1);
    first.await.unwrap();
    assert_eq!(second.await.value[0].id, "observed-2");
    cache.invalidate();
    io.pause.store(true, Ordering::SeqCst);
    let abandoned = {
        let cache = cache.clone();
        let io = io.clone();
        tokio::spawn(async move {
            cache
                .get(
                    b"next-key".to_vec(),
                    false,
                    false,
                    io.nvidia("synthetic-next"),
                )
                .await
        })
    };
    io.entered.notified().await;
    let takeover = cache.get(
        b"next-key".to_vec(),
        false,
        false,
        io.nvidia("synthetic-next"),
    );
    tokio::pin!(takeover);
    assert!(futures_util::poll!(&mut takeover).is_pending());
    abandoned.abort();
    assert!(abandoned.await.unwrap_err().is_cancelled());
    io.pause.store(false, Ordering::SeqCst);
    assert_eq!(takeover.await.value[0].id, "observed-4");
}
fn model(id: &str) -> Model {
    Model {
        id: id.into(),
        label: id.into(),
        ..Default::default()
    }
}
impl CatalogIo for Fake {
    fn invalidate_local(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn defaults<'a>(
        &'a self,
        _: &'a str,
    ) -> CoreFuture<'a, io::Result<BTreeMap<String, Vec<Model>>>> {
        Box::pin(async move {
            self.defaults.fetch_add(1, Ordering::SeqCst);
            if self.failed.load(Ordering::SeqCst) {
                return Err(io::Error::other("synthetic failure"));
            }
            Ok([
                ("anthropic", "a"),
                ("openai", "b"),
                ("copilot", "c"),
                ("grok", "remote-grok"),
            ]
            .into_iter()
            .map(|(key, id)| (key.into(), vec![model(id)]))
            .collect())
        })
    }
    fn native<'a>(&'a self, provider: &'a str, _: bool) -> CoreFuture<'a, io::Result<Vec<Model>>> {
        Box::pin(async move {
            self.native.fetch_add(1, Ordering::SeqCst);
            if provider == "grok" {
                Err(io::Error::other("synthetic absent native"))
            } else {
                Ok(vec![model(provider)])
            }
        })
    }
    fn nvidia<'a>(&'a self, _: &'a str) -> CoreFuture<'a, io::Result<Vec<Model>>> {
        Box::pin(async move {
            self.nvidia.fetch_add(1, Ordering::SeqCst);
            Ok(vec![model("nvidia/synthetic/model")])
        })
    }
    fn nvidia_key(&self) -> io::Result<String> {
        Ok(self.key.lock().unwrap().clone())
    }
    fn local<'a>(
        &'a self,
        _: &'a Config,
        kind: super::super::model_cache::LocalCatalog,
        _: bool,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<ObservedModels>> {
        Box::pin(async move {
            let models = match kind {
                super::super::model_cache::LocalCatalog::Ollama => vec![
                    Model {
                        remote_host: "synthetic-cloud".into(),
                        ..model("cloud-alias")
                    },
                    model("local"),
                ],
                super::super::model_cache::LocalCatalog::LmStudio => vec![model("lm")],
            };
            Ok(ObservedModels {
                models,
                at: Timestamp::now(),
                failed: false,
            })
        })
    }
}
fn fixture() -> (
    tempfile::TempDir,
    Arc<ConfigStore>,
    Arc<Fake>,
    ModelsCatalog,
) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49236, &root.path().join("installed")).unwrap();
    let mut config = Config::default();
    config.nvidia_nim.enabled = true;
    config.local_models = vec![
        crate::config::LocalModel {
            id: "local".into(),
            label: "Configured label".into(),
        },
        crate::config::LocalModel {
            id: "manual".into(),
            label: String::new(),
        },
    ];
    let config = Arc::new(ConfigStore::new(paths, config).unwrap());
    let fake = Arc::new(Fake {
        defaults: AtomicUsize::new(0),
        native: AtomicUsize::new(0),
        nvidia: AtomicUsize::new(0),
        failed: AtomicBool::new(false),
        key: Mutex::new("synthetic-key".into()),
    });
    let owner = ModelsCatalog::new(config.clone(), fake.clone());
    (root, config, fake, owner)
}
#[tokio::test(start_paused = true)]
async fn full_catalog_has_source_order_labels_fallback_routes_and_keyed_ttls() {
    let (_root, _config, fake, owner) = fixture();
    let cancel = Cancellation::default();
    let result = owner.response(false, &cancel).await.unwrap();
    assert_eq!(
        result
            .groups
            .iter()
            .map(|group| group.label.as_str())
            .collect::<Vec<_>>(),
        [
            "Anthropic",
            "OpenAI",
            "GitHub Copilot",
            "Cursor Agent",
            "Grok Build",
            "OpenCode",
            "NVIDIA NIM",
            "Ollama Cloud",
            "Ollama Local",
            "LM Studio"
        ]
    );
    assert_eq!(result.groups[6].route, "nvidia-nim");
    assert!(result.groups[6].hosted && result.groups[6].trial);
    assert_eq!(
        result.groups[8]
            .models
            .iter()
            .map(|model| (model.id.as_str(), model.label.as_str()))
            .collect::<Vec<_>>(),
        [("local", "Configured label"), ("manual", "manual")]
    );
    assert!(result.sources["grok"].starts_with("grok models (fallback: "));
    assert!(!result.cached_at.is_empty());
    owner.response(false, &cancel).await.unwrap();
    assert_eq!(fake.defaults.load(Ordering::SeqCst), 1);
    assert_eq!(fake.native.load(Ordering::SeqCst), 3);
    *fake.key.lock().unwrap() = "synthetic-other-key".into();
    owner.response(false, &cancel).await.unwrap();
    assert_eq!(fake.nvidia.load(Ordering::SeqCst), 2);
    tokio::time::advance(Duration::from_secs(181)).await;
    owner.response(false, &cancel).await.unwrap();
    assert_eq!(fake.native.load(Ordering::SeqCst), 4);
    owner.response(true, &cancel).await.unwrap();
    assert_eq!(fake.defaults.load(Ordering::SeqCst), 2);
    assert_eq!(fake.native.load(Ordering::SeqCst), 7);
}
#[tokio::test(start_paused = true)]
async fn remote_failure_keeps_last_actual_value_until_force_discards_it() {
    let (_root, _config, fake, owner) = fixture();
    let cancel = Cancellation::default();
    owner.response(false, &cancel).await.unwrap();
    fake.failed.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(86401)).await;
    let stale = owner.response(false, &cancel).await.unwrap();
    assert_eq!(stale.groups[0].models[0].id, "a");
    owner.response(false, &cancel).await.unwrap();
    assert_eq!(fake.defaults.load(Ordering::SeqCst), 2);
    let forced = owner.response(true, &cancel).await.unwrap();
    assert!(forced.groups[0].models.is_empty());
    assert!(forced.groups[1].models.is_empty());
    assert!(!forced.groups.iter().any(|group| group.provider == "grok"));
    assert_eq!(fake.defaults.load(Ordering::SeqCst), 3);
}
#[test]
fn native_and_nvidia_parsers_retain_source_order_and_reject_unsafe_ids() {
    assert_eq!(
        parsers::native(
            b"\x1b[32m* auto - Automatic\x1b[0m\nopus-4.1\nopus-4.1\nHeader text\n",
            "cursor-agent"
        )
        .unwrap()
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>(),
        ["auto", "opus-4.1"]
    );
    assert_eq!(
        parsers::native(b"grok-4-fast\nclaude\n", "grok").unwrap()[0].label,
        "Grok 4-fast"
    );
    let parsed = parsers::native(
        br#"opencode/first
{"id":"first","name":"First Model","status":"active"}
opencode/deleted
{"id":"deleted","status":"deprecated"}
opencode/last
"#,
        "opencode",
    )
    .unwrap();
    assert_eq!(
        parsed
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["opencode/first", "opencode/last"]
    );
    assert!(parsers::nvidia(br#"{"data":[{"id":"a/model"},{"id":"a/model"}]}"#).is_err());
    assert!(parsers::nvidia(br#"{"data":[{"id":"a/../model"}]}"#).is_err());
    assert_eq!(
        parsers::nvidia(br#"{"data":[{"id":"a/model"}]}"#).unwrap()[0].id,
        "nvidia/a/model"
    );
    assert!(parsers::defaults(br#"{"anthropic":[{"id":7}]}"#).is_err());
}
