use super::*;
use crate::{
    config::{Config, Resource},
    hub::{
        http::Request,
        slash_routes::{COMMANDS_PATH, SOURCES_PATH, SlashHttp},
        task_owner::HubTaskOwner,
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};
struct FakeIo {
    body: Mutex<Option<Vec<u8>>>,
    skills: Mutex<Vec<SlashCmd>>,
    reads: AtomicUsize,
}
impl SlashIo for FakeIo {
    fn read<'a>(&'a self, _: &'a str) -> CoreFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.body
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| io::Error::other("synthetic source outage"))
        })
    }
    fn skills(&self, _: &str, _: &SearchContext) -> Vec<SlashCmd> {
        self.skills.lock().unwrap().clone()
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _tasks: HubTaskOwner,
    paths: RuntimePaths,
    config: Arc<ConfigStore>,
    io: Arc<FakeIo>,
    owner: Arc<SlashCommands>,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let trial = root.path().join("trial");
    std::fs::create_dir(&trial).unwrap();
    let paths = RuntimePaths::trial(&trial, 49679, &root.path().join("installed")).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap());
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let io = Arc::new(FakeIo {
        body: Mutex::new(Some(b"| /remote | source command |".to_vec())),
        skills: Mutex::default(),
        reads: AtomicUsize::new(0),
    });
    let owner = SlashCommands::with_io(
        config.clone(),
        paths.clone(),
        Weak::new(),
        tasks.handle(),
        io.clone(),
        Arc::new(|_, _| {}),
    );
    Fixture {
        _root: root,
        _tasks: tasks,
        paths,
        config,
        io,
        owner,
    }
}
fn at() -> Timestamp {
    Timestamp::from_unix(1791158400, 0).unwrap()
}
#[test]
fn markdown_corpus_matches_pinned_go_priority_greedy_plain_rows_and_ascii_whitespace() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("markdown-go.json")).unwrap();
    assert_eq!(cases.len(), 29);
    for case in cases {
        let commands = markdown::parse(case["input"].as_str().unwrap());
        let actual = if commands.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::to_value(&commands).unwrap()
        };
        assert_eq!(actual, case["cmds"], "input {:?}", case["input"]);
    }
}
#[tokio::test]
async fn ttl_force_refresh_and_outage_preserve_original_fetch_time_and_fresh_skill_precedence() {
    let f = fixture();
    let first = f
        .owner
        .commands("claude", SearchContext::default(), false, at())
        .await
        .unwrap();
    assert_eq!(first.fetched_at, "2026-10-05T00:00:00Z");
    *f.io.body.lock().unwrap() = None;
    let cached = f
        .owner
        .commands(
            "claude",
            SearchContext::default(),
            false,
            at() + Duration::from_secs(24 * 3600 - 1),
        )
        .await
        .unwrap();
    assert_eq!(cached.cmds, first.cmds);
    assert_eq!(f.io.reads.load(Ordering::SeqCst), 1);
    *f.io.skills.lock().unwrap() = vec![
        SlashCmd {
            cmd: "/remote".into(),
            desc: "Skill. override".into(),
            kind: "skill".into(),
            ..Default::default()
        },
        SlashCmd {
            cmd: "/new".into(),
            desc: "Skill".into(),
            ..Default::default()
        },
    ];
    let expired = f
        .owner
        .commands(
            "claude",
            SearchContext::default(),
            false,
            at() + Duration::from_secs(24 * 3600 + 1),
        )
        .await
        .unwrap();
    assert_eq!(expired.fetched_at, first.fetched_at);
    assert_eq!(expired.cmds.as_ref().unwrap()[0].kind, "skill");
    assert_eq!(expired.cmds.as_ref().unwrap().len(), 2);
    assert_eq!(f.io.reads.load(Ordering::SeqCst), 2);
    // Fallback is not cached: every expired read retries the remote source.
    f.owner
        .commands(
            "claude",
            SearchContext::default(),
            false,
            at() + Duration::from_secs(24 * 3600 + 1),
        )
        .await
        .unwrap();
    assert_eq!(f.io.reads.load(Ordering::SeqCst), 3);
    f.owner
        .commands("claude", SearchContext::default(), true, at())
        .await
        .unwrap();
    assert_eq!(f.io.reads.load(Ordering::SeqCst), 4);
}
#[tokio::test]
async fn first_outage_with_skills_caches_skills_but_empty_success_and_empty_fallback_stay_json_null()
 {
    let f = fixture();
    *f.io.body.lock().unwrap() = Some(vec![]);
    assert!(
        f.owner
            .commands("copilot", SearchContext::default(), false, at())
            .await
            .unwrap()
            .cmds
            .is_none()
    );
    *f.io.body.lock().unwrap() = None;
    assert!(
        f.owner
            .commands("copilot", SearchContext::default(), true, at())
            .await
            .unwrap()
            .cmds
            .is_none()
    );
    *f.io.skills.lock().unwrap() = vec![SlashCmd {
        cmd: "$local".into(),
        desc: "Skill".into(),
        ..Default::default()
    }];
    let response = f
        .owner
        .commands("codex", SearchContext::default(), false, at())
        .await
        .unwrap();
    assert_eq!(response.cmds.unwrap()[0].cmd, "$local");
    let count = f.io.reads.load(Ordering::SeqCst);
    f.owner
        .commands(
            "codex",
            SearchContext::default(),
            false,
            at() + Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(f.io.reads.load(Ordering::SeqCst), count);
}
#[tokio::test]
async fn failed_save_retains_published_sources_and_invalidates_all_changed_provider_contexts_only()
{
    let f = fixture();
    for home in ["one", "two"] {
        f.owner
            .commands(
                "claude",
                SearchContext {
                    home_dir: home.into(),
                    ..Default::default()
                },
                false,
                at(),
            )
            .await
            .unwrap();
    }
    f.owner
        .commands("codex", SearchContext::default(), false, at())
        .await
        .unwrap();
    std::fs::create_dir(f.paths.resource(Resource::Config)).unwrap();
    let source = "https://raw.githubusercontent.com/synthetic/commands.md";
    assert!(matches!(
        f.owner.patch_sources(&SourcePatch {
            claude: Some(format!(" {source} ")),
            ..Default::default()
        }),
        Err(SourceFailure::Save)
    ));
    assert_eq!(
        f.config.snapshot().unwrap().config.slash_cmd_sources.claude,
        source
    );
    assert!(
        f.owner
            .cache
            .lock()
            .unwrap()
            .keys()
            .all(|key| !key.starts_with("claude|"))
    );
    assert!(f.owner.cache.lock().unwrap().contains_key("codex|||"));
}
#[tokio::test]
async fn source_http_patch_preserves_omitted_and_null_fields_and_rejects_before_publication() {
    let f = fixture();
    let http = SlashHttp::new(f.owner.clone());
    let original = f.config.snapshot().unwrap().config.slash_cmd_sources;
    let request=Request {method:"POST".into(),path:SOURCES_PATH.into(),body:br#"{"CLAUDE":"https://raw.githubusercontent.com/synthetic/a.md","codex":null} trailing"#.to_vec(),..Default::default()};
    assert_eq!(
        http.handle_authenticated(&request, at())
            .await
            .unwrap()
            .status,
        200
    );
    let actual = f.config.snapshot().unwrap().config.slash_cmd_sources;
    assert_eq!(actual.codex, original.codex);
    assert_eq!(actual.opencode, original.opencode);
    let invalid = Request {
        body: br#"{"claude":"http://raw.githubusercontent.com/no"}"#.to_vec(),
        ..request
    };
    assert_eq!(
        http.handle_authenticated(&invalid, at())
            .await
            .unwrap()
            .status,
        400
    );
    assert_eq!(
        f.config.snapshot().unwrap().config.slash_cmd_sources,
        actual
    );
    let bad_method = Request {
        method: "DELETE".into(),
        path: COMMANDS_PATH.into(),
        query: "provider=invalid".into(),
        ..Default::default()
    };
    assert_eq!(
        http.handle_authenticated(&bad_method, at())
            .await
            .unwrap()
            .status,
        405
    );
}
#[tokio::test]
async fn native_local_source_cap_path_scope_and_skill_frontmatter_follow_explicit_user_context() {
    let f = fixture();
    let native =
        NativeSlashIo::new(f.paths.clone(), vec![], Some(f.paths.root().to_owned())).unwrap();
    let source = f.paths.root().join("commands.md");
    std::fs::write(&source, vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
    assert!(
        native
            .read(source.to_str().unwrap())
            .await
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    std::fs::write(&source, "| /local | Local |\n").unwrap();
    assert!(native.read(source.to_str().unwrap()).await.is_ok());
    assert!(
        validate_source(
            f._root.path().join("outside.md").to_str().unwrap(),
            &f.paths
        )
        .is_err()
    );
    let vendor = f.paths.root().join("owned-vendor");
    let skill = vendor.join("skills/personal");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\r\nname: personal\r\ndescription: '**User** skill.'\r\n---\r\nbody",
    )
    .unwrap();
    let disabled = vendor.join("skills/private");
    std::fs::create_dir_all(&disabled).unwrap();
    std::fs::write(
        disabled.join("SKILL.md"),
        "---\nname: secret\nuser-invokable: false\n---\n",
    )
    .unwrap();
    let ignored = vendor.join("skills/node_modules/pkg");
    std::fs::create_dir_all(&ignored).unwrap();
    std::fs::write(ignored.join("SKILL.md"), "---\nname: ignored\n---\n").unwrap();
    let context = SearchContext {
        codex_home: vendor.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let skills = native.skills("codex", &context);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].cmd, "$personal");
    assert_eq!(skills[0].desc, "Skill. User skill.");
    assert!(
        !serde_json::to_string(&skills)
            .unwrap()
            .contains("owned-vendor")
    );
    assert!(native.skills("codex", &SearchContext::default()).is_empty());
}
#[test]
fn source_url_allowlist_credentials_and_private_dns_predicates_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("trial")).unwrap();
    let paths = RuntimePaths::trial(
        &root.path().join("trial"),
        49680,
        &root.path().join("installed"),
    )
    .unwrap();
    assert!(validate_source("https://raw.githubusercontent.com/example/file.md", &paths).is_ok());
    for source in [
        "http://raw.githubusercontent.com/example",
        "https://127.0.0.1/x",
        "https://example.com/x",
        "https://@raw.githubusercontent.com/x",
        "relative.md",
    ] {
        assert!(validate_source(source, &paths).is_err(), "{source}");
    }
}

#[tokio::test]
async fn native_trial_remote_refused_and_held_local_reads_are_bounded() {
    let f = fixture();
    let io = sources::NativeSlashIo::new(f.paths.clone(), vec![], None).unwrap();
    let mut credentials = url::Url::parse("https://raw.githubusercontent.com/x").unwrap();
    credentials.set_username("user").unwrap();
    assert!(sources::validate_source(credentials.as_str(), &f.paths).is_err());
    let remote = "https://raw.githubusercontent.com/example/synthetic/main/commands.md";
    sources::validate_source(remote, &f.paths).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), io.read(remote))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    let folder = f.paths.root().join("private-source");
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("commands.md");
    std::fs::write(&path, b"| /private | fixture only |").unwrap();
    assert_eq!(
        io.read(path.to_str().unwrap()).await.unwrap(),
        b"| /private | fixture only |"
    );
    std::fs::write(&path, vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
    assert!(
        io.read(path.to_str().unwrap())
            .await
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    assert!(io.read(folder.to_str().unwrap()).await.is_err());
    let outside = f._root.path().join("outside-source");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("commands.md"), b"outside private content").unwrap();
    let alias = f.paths.root().join("source-alias");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &alias).unwrap();
    #[cfg(windows)]
    {
        let status = std::process::Command::new("pwsh")
            .args(["-NoProfile","-NonInteractive","-Command","New-Item -ItemType Junction -Path $env:SLASH_FIXTURE_LINK -Target $env:SLASH_FIXTURE_TARGET -ErrorAction Stop | Out-Null"])
            .env("SLASH_FIXTURE_LINK",&alias).env("SLASH_FIXTURE_TARGET",&outside)
            .status().unwrap();
        assert!(status.success());
    }
    assert!(
        io.read(alias.join("commands.md").to_str().unwrap())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(outside.join("commands.md")).unwrap(),
        b"outside private content"
    );
}
