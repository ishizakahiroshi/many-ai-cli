use super::super::tests::fixture;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct FakeGist {
    calls: AtomicUsize,
    result: String,
}
impl BugReportIo for FakeGist {
    fn look_path(&self, _: &str) -> io::Result<String> {
        Ok("synthetic-gh".into())
    }
    fn create_secret_gist<'a>(
        &'a self,
        _: &'a str,
        markdown: &'a str,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<String>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert!(!markdown.contains("token=private"));
            Ok(self.result.clone())
        })
    }
}
fn owner(f: &super::super::tests::Fixture, io: Arc<FakeGist>) -> Arc<BugReport> {
    BugReport::new(BugReportDependencies {
        config: f.config.clone(),
        paths: f.paths.clone(),
        core: f.core.clone(),
        version: "fixture-v1".into(),
        platform: "windows".into(),
        arch: "amd64".into(),
        runtime_version: "rustc 1.90.0".into(),
        io,
    })
}
#[tokio::test]
async fn default_preview_never_opens_session_logs_or_saves_files() {
    let f = fixture();
    let gist = Arc::new(FakeGist {
        calls: AtomicUsize::new(0),
        result: String::new(),
    });
    let owner = owner(&f, gist.clone());
    let preview = owner
        .preview(PreviewRequest {
            session_id: Some(9999),
            ..Default::default()
        })
        .ok()
        .unwrap();
    assert_eq!(preview.warnings, Some(vec!["session_not_found".into()]));
    assert!(!preview.session_log_recorded);
    assert!(!preview.log_attachment_available);
    assert!(preview.log_markdown.is_empty());
    assert!(!f.paths.root().join("reports").exists());
    assert_eq!(gist.calls.load(Ordering::SeqCst), 0);
    let error = owner
        .preview(PreviewRequest {
            include_recent_log_lines: 200,
            ..Default::default()
        })
        .err()
        .unwrap();
    assert_eq!(error.status, 404);
    assert_eq!(
        owner
            .preview(PreviewRequest {
                include_recent_log_lines: 1,
                ..Default::default()
            })
            .err()
            .unwrap()
            .status,
        400
    );
}
#[tokio::test]
async fn preview_tokens_are_content_bound_single_use_expiring_and_bounded() {
    let f = fixture();
    let gist = Arc::new(FakeGist {
        calls: AtomicUsize::new(0),
        result: "https://gist.github.com/fixture/abcd".into(),
    });
    let owner = owner(&f, gist.clone());
    let token = owner.remember("visible log").unwrap();
    assert!(!owner.consume(&token, "changed log"));
    assert!(!owner.consume(&token, "visible log"));
    let token = owner.remember("visible log").unwrap();
    owner.previews.lock().unwrap().get_mut(&token).unwrap().1 =
        Instant::now() - Duration::from_secs(1);
    assert!(!owner.consume(&token, "visible log"));
    for _ in 0..70 {
        owner.remember("visible log").unwrap();
    }
    assert_eq!(owner.previews.lock().unwrap().len(), 64);
    let request = FinalizeRequest {
        symptom: "test".into(),
        include_session_log: true,
        log_markdown: "visible log".into(),
        log_preview_token: "unknown-token".into(),
        ..Default::default()
    };
    assert_eq!(
        owner
            .finalize(request, &Cancellation::default())
            .await
            .err()
            .unwrap()
            .code,
        "log_preview_required"
    );
    assert_eq!(gist.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn invalid_gist_url_saves_only_scrubbed_local_fallback_and_token_cannot_retry() {
    let f = fixture();
    let gist = Arc::new(FakeGist {
        calls: AtomicUsize::new(0),
        result: "https://attacker.example/abcd".into(),
    });
    let owner = owner(&f, gist.clone());
    let log = canonical("token=private");
    let token = owner.remember(&log).unwrap();
    let result = owner
        .finalize(
            FinalizeRequest {
                symptom: "symptom token=private".into(),
                include_session_log: true,
                log_markdown: log.clone(),
                log_preview_token: token.clone(),
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .await
        .ok()
        .unwrap();
    assert_eq!(result.warnings, vec!["gist_url_rejected"]);
    assert!(result.saved_path.starts_with('~'));
    assert!(!result.markdown.contains("token=private"));
    assert_eq!(gist.calls.load(Ordering::SeqCst), 1);
    assert!(!owner.consume(&token, &log));
    let root = Dir::open(&f.paths.root().join("reports")).unwrap();
    let entries = root.entries().unwrap();
    assert_eq!(entries.len(), 1);
    let saved = String::from_utf8(root.read(&entries[0], usize::MAX).unwrap()).unwrap();
    assert!(saved.contains("Scrubbed session log (local fallback)"));
    assert!(!saved.contains("token=private"));
    assert!(!saved.contains("attacker.example"));
}
#[tokio::test]
async fn finalized_log_gist_validated_and_fixed_issue_url_scrubbed() {
    let f = fixture();
    let gist = Arc::new(FakeGist {
        calls: AtomicUsize::new(0),
        result: "https://gist.github.com/fixture/abcd".into(),
    });
    let owner = owner(&f, gist.clone());
    let token = owner.remember("approved log").unwrap();
    let result = owner
        .finalize(
            FinalizeRequest {
                symptom: "fixture".into(),
                include_session_log: true,
                log_markdown: "approved log".into(),
                log_preview_token: token,
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .await
        .ok()
        .unwrap();
    assert!(
        result
            .markdown
            .contains("[log-attachment](https://gist.github.com/fixture/abcd)")
    );
    assert!(
        result
            .url
            .starts_with("https://github.com/ishizakahiroshi/many-ai-cli/issues/new?")
    );
    assert!(result.warnings.is_empty());
    assert!(!f.paths.root().join("reports").exists());
    assert_eq!(gist.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn log_tail_is_private_bounded_200_lines_and_prevents_escape() {
    let f = fixture();
    let owner = owner(
        &f,
        Arc::new(FakeGist {
            calls: AtomicUsize::new(0),
            result: String::new(),
        }),
    );
    let cfg = f.config.snapshot().unwrap().config;
    let logs = f
        .paths
        .clone()
        .with_log_dir(Path::new(&cfg.hub.log_dir))
        .unwrap()
        .resource(crate::config::Resource::Logs)
        .join("sessions");
    let dir = Dir::open_or_create_private(&logs).unwrap();
    let bytes = (0..250)
        .map(|i| format!("line-{i} token=private\n"))
        .collect::<String>();
    dir.replace("synthetic.jsonl", bytes.as_bytes(), 0o600)
        .unwrap();
    let (text, truncated) = owner
        .tail(&cfg, &logs.join("synthetic.jsonl").to_string_lossy())
        .unwrap();
    assert!(truncated);
    assert_eq!(text.lines().count(), 200);
    assert!(text.starts_with("line-50"));
    assert!(!text.contains("token=private"));
    let outside = f.paths.root().join("outside.jsonl");
    std::fs::write(&outside, "private").unwrap();
    assert!(owner.tail(&cfg, &outside.to_string_lossy()).is_err());
    assert!(
        owner
            .tail(&cfg, &logs.join("../outside.jsonl").to_string_lossy())
            .is_err()
    );
}
#[test]
fn gist_validation_rejects_credentials_ports_queries_fragments_and_markup() {
    let mut credentials = url::Url::parse("https://gist.github.com/a/b").unwrap();
    credentials.set_username("user").unwrap();
    assert!(validated_gist_url(credentials.as_str()).is_none());

    for url in [
        "http://gist.github.com/a/b",
        "https://gist.github.com/",
        "https://gist.github.com:443/a/b",
        "https://gist.github.com/a/b?token=x",
        "https://gist.github.com/a/b#x",
        "https://gist.github.com/a/(b)",
    ] {
        assert!(validated_gist_url(url).is_none(), "accepted {url}");
    }
    assert_eq!(
        validated_gist_url("https://gist.github.com/a/b").as_deref(),
        Some("https://gist.github.com/a/b")
    );
}
