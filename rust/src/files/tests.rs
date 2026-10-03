use super::*;
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    time::{Duration, UNIX_EPOCH},
};
struct Workspace(Vec<(String, String)>);
impl WorkspaceReads for Workspace {
    fn memo_mentions(&self) -> Vec<(String, String)> {
        self.0.clone()
    }
    fn recorded_handoff_note(&self, _: &Path, _: &Path) -> bool {
        false
    }
}
fn setup() -> (tempfile::TempDir, FilesService, scope::Scope) {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("project 日本語");
    fs::create_dir(&cwd).unwrap();
    fs::create_dir(cwd.join(".git")).unwrap();
    let runtime = dir.path().join("runtime");
    fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49123, &dir.path().join("installed")).unwrap();
    let service = FilesService::new(cwd.clone(), paths.clone());
    let scope = scope::Scope::new(cwd, paths);
    (dir, service, scope)
}
fn request(route: &str, body: Value) -> Request {
    Request {
        method: "POST".into(),
        path: route.into(),
        body: serde_json::to_vec(&body).unwrap(),
        ..Default::default()
    }
}
fn read_request(route: &str, path: &Path) -> Request {
    Request {
        method: "GET".into(),
        path: route.into(),
        query: url::form_urlencoded::Serializer::new(String::new())
            .append_pair("path", &path.to_string_lossy())
            .finish(),
        ..Default::default()
    }
}
fn response(result: Result<Response>) -> Response {
    result.unwrap_or_else(|e| e)
}
fn value(r: &Response) -> Value {
    serde_json::from_slice(&r.body).unwrap()
}
#[test]
fn direct_loopback_reads_outside_roots_but_remote_requires_memo_and_is_readonly() {
    let (dir, service, s) = setup();
    let path = dir.path().join("外部 notes.md");
    fs::write(&path, b"hello").unwrap();
    let r = read_request("/api/files-content", &path);
    let ws = Workspace(vec![]);
    let local = response(service.read_handle(&r, &s, None, false, &ws));
    assert_eq!(local.status, 200);
    assert_eq!(value(&local)["readOnly"], true);
    assert_eq!(
        response(service.read_handle(&r, &s, None, true, &ws)).status,
        403
    );
    let ws = Workspace(vec![(
        dir.path().to_string_lossy().into(),
        "see 外部 notes.md".into(),
    )]);
    let remote = response(service.read_handle(&r, &s, None, true, &ws));
    assert_eq!(remote.status, 200);
    assert_eq!(value(&remote)["readOnly"], true);
    let save = response(mutation::handle(
        &request("/api/files-save", json!({"path":path,"content":"bad"})),
        &s,
    ));
    assert_eq!(save.status, 403);
    assert_eq!(fs::read(path).unwrap(), b"hello");
}
#[test]
fn secrets_denied_only_outside_roots_and_before_mentions() {
    let (dir, service, s) = setup();
    let outside = dir.path().join("secrets.json");
    fs::write(&outside, b"synthetic").unwrap();
    let ws = Workspace(vec![(
        String::new(),
        outside.to_string_lossy().into_owned(),
    )]);
    for remote in [false, true] {
        assert_eq!(
            response(service.read_handle(
                &read_request("/api/files-content", &outside),
                &s,
                None,
                remote,
                &ws
            ))
            .status,
            403
        );
    }
    let inside = s.cwd.join(".env");
    fs::write(&inside, b"synthetic=true").unwrap();
    assert_eq!(
        response(service.read_handle(
            &read_request("/api/files-content", &inside),
            &s,
            None,
            true,
            &ws
        ))
        .status,
        200
    );
}
#[test]
fn remote_mentioned_binary_download_denied_local_stream_allowed() {
    let (dir, service, s) = setup();
    let path = dir.path().join("payload.custombin");
    fs::write(&path, b"not-secret").unwrap();
    let ws = Workspace(vec![(String::new(), path.to_string_lossy().into())]);
    let r = read_request("/api/files-download", &path);
    assert_eq!(
        response(service.read_handle(&r, &s, None, true, &ws)).status,
        403
    );
    let local = response(service.read_handle(&r, &s, None, false, &ws));
    assert_eq!(local.status, 200);
    assert!(local.file.is_some());
    assert_eq!(
        local.headers["Content-Disposition"],
        "attachment; filename=\"payload.custombin\"; filename*=UTF-8''payload.custombin"
    );
}
#[test]
fn save_nanosecond_conflict_preserves_file_and_reports_current_mtime() {
    let (_dir, _service, s) = setup();
    let path = s.cwd.join("memo 日本語.md");
    fs::write(&path, b"old").unwrap();
    let stamp = UNIX_EPOCH + Duration::new(1_700_000_000, 900);
    let f = fs::OpenOptions::new().write(true).open(&path).unwrap();
    f.set_times(fs::FileTimes::new().set_modified(stamp))
        .unwrap();
    let stale = time::format(UNIX_EPOCH + Duration::new(1_700_000_000, 800)).unwrap();
    let r = response(mutation::handle(
        &request(
            "/api/files-save",
            json!({"path":path,"content":"new","baseMtime":stale}),
        ),
        &s,
    ));
    assert_eq!(r.status, 409);
    assert_eq!(value(&r)["mtime"], time::format_utc(stamp).unwrap());
    assert_eq!(fs::read(&path).unwrap(), b"old");
    let r = response(mutation::handle(
        &request(
            "/api/files-save",
            json!({"path":path,"content":"new","baseMtime":time::format(stamp).unwrap()}),
        ),
        &s,
    ));
    assert_eq!(r.status, 200);
    assert_eq!(fs::read(path).unwrap(), b"new");
}
#[test]
fn create_casefold_unknown_fields_null_and_first_value_match_go() {
    let (_dir, _service, s) = setup();
    let mut r = request(
        "/api/files-create",
        json!({"DIR":s.cwd,"NAME":"空 白.md","ignored":1}),
    );
    r.body.extend_from_slice(b" {ignored trailing}");
    let r = response(mutation::handle(&r, &s));
    assert_eq!(r.status, 200);
    assert!(s.cwd.join("空 白.md").is_file());
    let r = response(mutation::handle(
        &request("/api/files-create", json!({"dir":s.cwd,"name":"空 白.md"})),
        &s,
    ));
    assert_eq!(r.status, 409);
    assert_eq!(value(&r)["error"], "already_exists");
    let mut r = request(
        "/api/files-save",
        json!({"path":s.cwd.join("空 白.md"),"content":"x"}),
    );
    r.body.extend_from_slice(b" {}");
    assert_eq!(response(mutation::handle(&r, &s)).status, 400);
}
#[test]
fn denied_vcs_mutations_and_root_delete_never_modify_targets() {
    let (_dir, _service, s) = setup();
    fs::create_dir(s.cwd.join(".git/hooks")).unwrap();
    let path = s.cwd.join(".git/hooks/pre-commit.md");
    fs::write(&path, b"original").unwrap();
    for (route, body) in [
        ("/api/files-save", json!({"path":path,"content":"bad"})),
        (
            "/api/files-create",
            json!({"dir":s.cwd.join(".git"),"name":"config.md"}),
        ),
        ("/api/files-mkdir", json!({"dir":s.cwd,"name":".git"})),
        ("/api/files-rename", json!({"src":path,"newName":"new.md"})),
        ("/api/files-delete-dir", json!({"src":s.cwd.join(".git")})),
    ] {
        assert_eq!(
            response(mutation::handle(&request(route, body), &s)).status,
            403,
            "{route}"
        );
    }
    assert_eq!(
        response(mutation::handle(
            &request("/api/files-delete-dir", json!({"src":s.cwd})),
            &s
        ))
        .status,
        409
    );
    assert_eq!(fs::read(path).unwrap(), b"original");
}
#[test]
fn multi_move_preflight_is_all_or_nothing_then_unicode_moves_succeed() {
    let (_dir, _service, s) = setup();
    let a = s.cwd.join("a.md");
    let b = s.cwd.join("b 日本語.md");
    let dst = s.cwd.join("dst dir");
    fs::write(&a, b"a").unwrap();
    fs::write(&b, b"b").unwrap();
    fs::create_dir(&dst).unwrap();
    fs::write(dst.join("a.md"), b"existing").unwrap();
    let body = json!({"srcs":[a,b],"dstDir":dst});
    let r = response(mutation::handle(
        &request("/api/files-move", body.clone()),
        &s,
    ));
    assert_eq!(r.status, 400);
    assert_eq!(value(&r)["error"], "move_preflight_failed");
    assert!(a.exists() && b.exists());
    fs::remove_file(dst.join("a.md")).unwrap();
    let r = response(mutation::handle(&request("/api/files-move", body), &s));
    assert_eq!(r.status, 200);
    assert!(!a.exists() && !b.exists());
    assert_eq!(fs::read(dst.join("b 日本語.md")).unwrap(), b"b");
}
#[test]
fn preview_bounds_summary_and_stream_range() {
    let (_dir, service, s) = setup();
    let p = s.cwd.join("long.md");
    fs::write(&p, vec![b'x'; content::CONTENT_MAX + 1]).unwrap();
    let ws = Workspace(vec![]);
    let r =
        response(service.read_handle(&read_request("/api/files-content", &p), &s, None, true, &ws));
    assert_eq!(
        value(&r)["content"].as_str().unwrap().len(),
        content::CONTENT_MAX
    );
    assert_eq!(value(&r)["truncated"], true);
    let mut r = read_request("/api/files-download", &p);
    r.headers.push(("Range".into(), "bytes=1-9".into()));
    let r = response(service.read_handle(&r, &s, None, false, &ws));
    assert_eq!(r.status, 206);
    assert_eq!(r.file.unwrap().ranges[0].len, 9);
    assert_eq!(
        content::summary("---\ndescription: \"Unicode 日本語\"\n---\n# Title\nignored"),
        "Unicode 日本語"
    );
    assert_eq!(
        content::summary("# Title\n> meta\n\nfirst paragraph\nsecond line\n\nno"),
        "first paragraph second line"
    );
}
#[cfg(unix)]
#[test]
fn capability_rejects_symlink_interleavings_and_replacement_cannot_redirect() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (dir, _service, s) = setup();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.md"), b"unchanged").unwrap();
    let owned = s.cwd.join("owned");
    fs::create_dir(&owned).unwrap();
    let cap = safe_fs::Dir::open(&owned).unwrap();
    fs::rename(&owned, s.cwd.join("moved")).unwrap();
    symlink(&outside, &owned).unwrap();
    cap.create_new("created.md", b"private", 0o600).unwrap();
    assert!(!outside.join("created.md").exists());
    assert!(s.cwd.join("moved/created.md").exists());
    assert_eq!(
        fs::metadata(s.cwd.join("moved/created.md"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(safe_fs::Dir::open(&owned).is_err());
    symlink(outside.join("secret.md"), s.cwd.join("moved/link.md")).unwrap();
    assert!(cap.open_file("link.md", true).is_err());
    assert!(cap.create_new("link.md", b"bad", 0o600).is_err());
    cap.replace("link.md", b"safe replacement", 0o600).unwrap();
    assert_eq!(fs::read(outside.join("secret.md")).unwrap(), b"unchanged");
    assert_eq!(
        fs::read(s.cwd.join("moved/link.md")).unwrap(),
        b"safe replacement"
    );
}
#[cfg(unix)]
#[test]
fn recursive_remove_never_follows_symlink_and_atomic_rename_never_overwrites() {
    use std::os::unix::fs::symlink;
    let (dir, _service, s) = setup();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"safe").unwrap();
    let cap = safe_fs::Dir::open(&s.cwd).unwrap();
    let child = cap.child_dir("delete", true).unwrap();
    symlink(&outside, child.path().join("link")).unwrap();
    child.create_new("file.md", b"owned", 0o600).unwrap();
    drop(child);
    cap.remove_tree("delete").unwrap();
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"safe");
    cap.create_new("a.md", b"a", 0o600).unwrap();
    cap.create_new("b.md", b"b", 0o600).unwrap();
    assert_eq!(
        cap.rename_to("a.md", &cap, "b.md").unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(cap.read("b.md", 2).unwrap(), b"b");
}
#[test]
fn attachment_filename_and_injection_preserve_unicode_safe_basename() {
    let name = attachments::sanitized_filename("../../日本語 screenshot.PNG", b"ignored");
    assert_eq!(name, "日本語_screenshot.png");
    assert!(name.len() <= 180);
    assert_eq!(
        attachments::inject("codex", Path::new("C:\\owned files\\画像.png")).unwrap(),
        "@C:/owned files/画像.png "
    );
    assert!(attachments::inject("claude", Path::new("/owned/line\nfile")).is_err());
}
#[test]
fn maintenance_disabled_is_noop_and_size_cap_removes_oldest() {
    let (_dir, service, _s) = setup();
    let root = safe_fs::Dir::open(service.paths.root()).unwrap();
    let attach = root.child_dir("attachments", true).unwrap();
    let session = attach.child_dir("1", true).unwrap();
    session.create_new("old.md", b"1234", 0o600).unwrap();
    session.create_new("new.md", b"5678", 0o600).unwrap();
    let old = UNIX_EPOCH + Duration::from_secs(100);
    fs::File::options()
        .write(true)
        .open(session.path().join("old.md"))
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
    assert_eq!(
        service.clean_attachments(0, 0, SystemTime::now()).unwrap(),
        0
    );
    assert_eq!(
        service.clean_attachments(0, 4, SystemTime::now()).unwrap(),
        1
    );
    assert!(!session.path().join("old.md").exists());
    assert!(session.path().join("new.md").exists());
}
use std::time::SystemTime;
#[test]
fn home_git_ceiling_and_gitfile_do_not_widen_file_scope() {
    let base = tempfile::tempdir().unwrap();
    let home = base.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::create_dir(home.join(".git")).unwrap();
    let project = home.join("project");
    fs::create_dir(&project).unwrap();
    let deep = project.join("deep");
    fs::create_dir(&deep).unwrap();
    assert_eq!(scope::find_git_root_with_home(&deep, Some(&home)), deep);
    fs::write(project.join(".git"), "gitdir: synthetic").unwrap();
    assert_eq!(scope::find_git_root_with_home(&deep, Some(&home)), deep);
    fs::remove_file(project.join(".git")).unwrap();
    fs::create_dir(project.join(".git")).unwrap();
    assert_eq!(scope::find_git_root_with_home(&deep, Some(&home)), project);
    assert_eq!(scope::find_git_root_with_home(&home, Some(&home)), home);
}
#[test]
fn multipart_ranges_stream_with_exact_length_and_conditionals() {
    let (_dir, service, s) = setup();
    let p = s.cwd.join("range.txt");
    fs::write(&p, b"0123456789").unwrap();
    let stamp = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(stamp))
        .unwrap();
    let mut r = read_request("/api/files-download", &p);
    r.headers.push(("Range".into(), "bytes=0-1,8-9".into()));
    let ws = Workspace(vec![]);
    let response = response(service.read_handle(&r, &s, None, false, &ws));
    assert_eq!(response.status, 206);
    let stream = response.file.unwrap();
    assert_eq!(stream.ranges.len(), 2);
    let total: u64 = stream
        .ranges
        .iter()
        .map(|p| p.prefix.len() as u64 + p.len)
        .sum::<u64>()
        + stream.suffix.len() as u64;
    assert_eq!(response.headers["Content-Length"], total.to_string());
    let mut r = read_request("/api/files-download", &p);
    r.headers
        .push(("If-Modified-Since".into(), httpdate::fmt_http_date(stamp)));
    let response = super::tests::response(service.read_handle(&r, &s, None, false, &ws));
    assert_eq!(response.status, 304);
    assert!(response.file.is_none());
    r.headers.push(("If-Match".into(), "\"unknown\"".into()));
    assert_eq!(
        super::tests::response(service.read_handle(&r, &s, None, false, &ws)).status,
        412
    );
}
#[test]
fn file_timestamp_errors_never_fabricate_success_and_explicit_empty_base_is_invalid() {
    assert!(time::format(UNIX_EPOCH + Duration::from_secs(253402300800)).is_err());
    let (_dir, _service, s) = setup();
    let p = s.cwd.join("a.md");
    fs::write(&p, b"original").unwrap();
    let request = request(
        "/api/files-save",
        json!({"path":p,"content":"bad","baseMtime":""}),
    );
    assert_eq!(response(mutation::handle(&request, &s)).status, 400);
    assert_eq!(fs::read(p).unwrap(), b"original");
}
#[test]
fn list_never_summarizes_secrets_and_skips_symlinks_heavy_subtrees() {
    let (_dir, service, s) = setup();
    let docs = s.cwd.join("docs/local");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join(".env"), "SECRET=synthetic").unwrap();
    fs::write(docs.join("notes.md"), "# Title\n\nhello world").unwrap();
    fs::create_dir(docs.join("node_modules")).unwrap();
    fs::write(docs.join("node_modules/large.txt"), "not listed").unwrap();
    let r = Request {
        method: "GET".into(),
        path: "/api/files-list".into(),
        ..Default::default()
    };
    let r = response(service.read_handle(&r, &s, None, false, &Workspace(vec![])));
    let rows = value(&r)["items"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter().find(|v| v["name"] == ".env").unwrap()["summary"],
        ""
    );
    assert_eq!(
        rows.iter().find(|v| v["name"] == "notes.md").unwrap()["summary"],
        "hello world"
    );
}

#[test]
fn save_timestamp_duplicates_validate_each_occurrence_and_null_keeps_prior() {
    let (_dir, _service, s) = setup();
    let p = s.cwd.join("a.md");
    fs::write(&p, b"old").unwrap();
    let stamp = fs::metadata(&p).unwrap().modified().unwrap();
    let prefix = format!(
        "{{\"path\":{},\"content\":\"new\",",
        serde_json::to_string(&p).unwrap()
    );
    let mut request = Request {
        method: "POST".into(),
        path: "/api/files-save".into(),
        body: format!(
            "{prefix}\"baseMtime\":\"\",\"baseMtime\":{}}}",
            serde_json::to_string(&time::format(stamp).unwrap()).unwrap()
        )
        .into_bytes(),
        ..Default::default()
    };
    assert_eq!(response(mutation::handle(&request, &s)).status, 400);
    request.body = format!(
        "{prefix}\"baseMtime\":{},\"baseMtime\":null}}",
        serde_json::to_string(&time::format(stamp).unwrap()).unwrap()
    )
    .into_bytes();
    assert_eq!(response(mutation::handle(&request, &s)).status, 200);
}

#[cfg(unix)]
#[test]
fn purge_preserves_active_session_and_never_follows_foreign_symlink() {
    use std::os::unix::fs::symlink;
    let (dir, service, _) = setup();
    let root = safe_fs::Dir::open(service.paths.root()).unwrap();
    let attachments = root.child_dir("attachments", true).unwrap();
    attachments
        .child_dir("7", true)
        .unwrap()
        .create_new("live.txt", b"live", 0o600)
        .unwrap();
    attachments
        .child_dir("8", true)
        .unwrap()
        .create_new("old.txt", b"old", 0o600)
        .unwrap();
    let outside = dir.path().join("foreign");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"safe").unwrap();
    symlink(&outside, attachments.path().join("9")).unwrap();
    let r = response(service.purge_attachment_ids(&[LiveSessionId(7)]));
    assert_eq!(value(&r)["folders"], 1);
    assert!(attachments.path().join("7/live.txt").exists());
    assert!(!attachments.path().join("8").exists());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"safe");
}

#[cfg(unix)]
#[test]
fn saved_attachment_is_private_local_timestamp_prefixed_and_bounded() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, service, _) = setup();
    let value = service
        .save_attachment(7, "codex", "CON.txt", b"synthetic", None)
        .unwrap();
    let path = Path::new(value["saved_path"].as_str().unwrap());
    assert!(path.starts_with(service.paths.resource(crate::config::Resource::Attachments)));
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("_CON.txt")
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read(path).unwrap(), b"synthetic");
    assert!(
        service
            .save_attachment(
                7,
                "claude",
                "large.bin",
                &vec![0; attachments::UPLOAD_MAX + 1],
                None
            )
            .is_err()
    );
}

#[test]
fn private_append_does_not_overwrite_and_concurrent_handles_append_at_eof() {
    use std::io::Write;
    #[cfg(unix)]
    use std::io::{Seek, SeekFrom};
    let root = tempfile::tempdir().unwrap();
    let dir = std::sync::Arc::new(safe_fs::Dir::open(root.path()).unwrap());
    let mut first = dir.open_append("history.jsonl").unwrap();
    first.write_all(b"original\n").unwrap();
    // Rewinding an append-only descriptor must not authorize an overwrite.
    #[cfg(unix)]
    first.seek(SeekFrom::Start(0)).unwrap();
    first.write_all(b"retained\n").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(5));
    let mut threads = Vec::new();
    for worker in 0..4 {
        let mut file = dir.open_append("history.jsonl").unwrap();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            for index in 0..100 {
                let line = format!("{worker}:{index:03}:{}\n", "x".repeat(64));
                // One write is the atomic unit; the API does not promise that
                // several separate writes form a transaction across handles.
                assert_eq!(file.write(line.as_bytes()).unwrap(), line.len());
            }
        }));
    }
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    let contents = fs::read_to_string(root.path().join("history.jsonl")).unwrap();
    assert!(contents.starts_with("original\nretained\n"));
    let lines: std::collections::BTreeSet<_> = contents.lines().skip(2).collect();
    assert_eq!(lines.len(), 400);
    assert_eq!(contents.lines().count(), 402);
    assert!(dir.open_append("../escape").is_err());
    fs::create_dir(root.path().join("directory")).unwrap();
    assert!(dir.open_append("directory").is_err());
}

#[cfg(unix)]
#[test]
fn private_append_rejects_links_fifo_and_survives_parent_replacement() {
    use std::{
        io::Write,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let root = tempfile::tempdir().unwrap();
    let owned = root.path().join("owned");
    let foreign = root.path().join("foreign");
    fs::create_dir(&owned).unwrap();
    fs::create_dir(&foreign).unwrap();
    let dir = safe_fs::Dir::open(&owned).unwrap();
    fs::write(foreign.join("keep"), b"unchanged").unwrap();
    symlink(foreign.join("keep"), owned.join("linked")).unwrap();
    assert!(dir.open_append("linked").is_err());
    let fifo = std::ffi::CString::new(owned.join("pipe").to_str().unwrap()).unwrap();
    // SAFETY: this test creates a FIFO only inside its owned temporary root.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(dir.open_append("pipe").is_err());
    fs::write(owned.join("old.log"), b"prefix").unwrap();
    fs::set_permissions(owned.join("old.log"), fs::Permissions::from_mode(0o666)).unwrap();
    let mut existing = dir.open_append("old.log").unwrap();
    assert_eq!(
        existing.metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    existing.write_all(b" suffix").unwrap();
    let renamed = root.path().join("renamed");
    fs::rename(&owned, &renamed).unwrap();
    symlink(&foreign, &owned).unwrap();
    let mut file = dir.open_append("new.log").unwrap();
    file.write_all(b"owned inode").unwrap();
    assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(fs::read(renamed.join("new.log")).unwrap(), b"owned inode");
    assert!(!foreign.join("new.log").exists());
    assert_eq!(fs::read(foreign.join("keep")).unwrap(), b"unchanged");
    assert_eq!(fs::read(renamed.join("old.log")).unwrap(), b"prefix suffix");
}

#[cfg(unix)]
#[test]
fn private_directory_creation_restricts_only_final_and_never_follows_replacement() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let ancestor = root.path().join("existing");
    fs::create_dir(&ancestor).unwrap();
    fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o755)).unwrap();
    let leaf = ancestor.join("private").join("logs");
    let dir = safe_fs::Dir::open_or_create_private(&leaf).unwrap();
    assert_eq!(
        fs::metadata(&ancestor).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(
        dir.own_metadata().unwrap().permissions().mode() & 0o777,
        0o700
    );
    fs::set_permissions(&leaf, fs::Permissions::from_mode(0o755)).unwrap();
    let repaired = safe_fs::Dir::open_or_create_private(&leaf).unwrap();
    assert_eq!(
        repaired.own_metadata().unwrap().permissions().mode() & 0o777,
        0o700
    );
    let foreign = root.path().join("foreign");
    fs::create_dir(&foreign).unwrap();
    fs::set_permissions(&foreign, fs::Permissions::from_mode(0o755)).unwrap();
    symlink(&foreign, ancestor.join("link")).unwrap();
    assert!(safe_fs::Dir::open_or_create_private(&ancestor.join("link")).is_err());
    assert!(safe_fs::Dir::open_or_create_private(&ancestor.join("link/new")).is_err());
    assert_eq!(
        fs::metadata(&foreign).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!foreign.join("new").exists());
    let moved = ancestor.join("moved");
    fs::rename(&leaf, &moved).unwrap();
    symlink(&foreign, &leaf).unwrap();
    fs::set_permissions(&moved, fs::Permissions::from_mode(0o755)).unwrap();
    dir.restrict_private().unwrap();
    assert_eq!(
        fs::metadata(&moved).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&foreign).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(safe_fs::Dir::open_or_create_private(Path::new("/")).is_err());
}

#[test]
fn attachment_history_uses_injected_journal_once() {
    use crate::proto::core::{
        HistoryEvent, PersistenceEffect, PersistenceEffectSink, SessionError,
    };
    use std::sync::{Arc, Mutex};
    struct Journal(Mutex<Vec<HistoryEvent>>);
    impl PersistenceEffectSink for Journal {
        fn apply(&self, effect: PersistenceEffect) -> std::result::Result<(), SessionError> {
            let PersistenceEffect::Event { session, event } = effect else {
                panic!("attachment emits one event")
            };
            assert_eq!(session, LiveSessionId(7));
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }
    let (_dir, service, _) = setup();
    let journal = Arc::new(Journal(Mutex::new(vec![])));
    let service = service.with_history(journal.clone());
    let saved = service
        .save_attachment(7, "codex", "fixture.txt", b"synthetic attachment", None)
        .unwrap();
    let events = journal.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0["type"], "attach");
    assert_eq!(events[0].0["path"], saved["saved_path"]);
    assert_eq!(events[0].0["filename"], "fixture.txt");
    assert!(!events[0].0.contains_key("data"));
}
