use super::*;
use crate::config::{self, UserPrefs};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default, Deserialize)]
#[serde(default)]
struct Payload {
    hex: String,
    pad_to: usize,
    directory: bool,
    write_only: bool,
}
impl Payload {
    fn bytes(&self) -> Vec<u8> {
        let mut bytes: Vec<_> = self
            .hex
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        bytes.resize(bytes.len().max(self.pad_to), 0);
        bytes
    }
}
#[derive(Deserialize)]
struct Case {
    name: String,
    path: String,
    method: String,
    body: Payload,
    initial: Value,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    media: BTreeMap<String, Payload>,
    #[serde(default)]
    fail_save: bool,
    #[serde(default)]
    missing_root: bool,
}
struct Fixture {
    _temp: tempfile::TempDir,
    paths: RuntimePaths,
    store: Arc<ConfigStore>,
}
fn fixture(initial: Value, missing_root: bool) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::production(temp.path()).unwrap();
    if !missing_root {
        std::fs::create_dir(paths.root()).unwrap();
    }
    let raw = initial.to_string().replace(
        "<root>",
        &paths.root().to_string_lossy().replace('\\', "\\\\"),
    );
    let config = Config {
        user_prefs: config::decode_user_prefs_http_json(raw.as_bytes()).unwrap(),
        ..Default::default()
    };
    let store = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
    Fixture {
        _temp: temp,
        paths,
        store,
    }
}
fn request(method: &str, path: &str, bytes: &[u8]) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        body: bytes.into(),
        ..Default::default()
    }
}
fn call(f: &Fixture, method: &str, path: &str, bytes: &[u8]) -> Response {
    handle_authenticated(&request(method, path, bytes), &f.store, &f.paths, |_| {}).unwrap()
}
fn digest(bytes: &[u8]) -> Value {
    json!({"len":bytes.len(),"sha256":Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>()})
}
fn normalize(value: Value, paths: &RuntimePaths) -> Value {
    match value {
        Value::String(text) => {
            for (resource, fixed) in [
                (Resource::Avatar, "user_avatar.bin"),
                (Resource::NotifySound, "notify_sound_custom.bin"),
            ] {
                if text == paths.resource(resource).to_string_lossy() {
                    return format!("<root>{}{fixed}", std::path::MAIN_SEPARATOR).into();
                }
            }
            text.replace(paths.root().to_string_lossy().as_ref(), "<root>")
                .into()
        }
        Value::Object(map) => map
            .into_iter()
            .map(|(k, v)| (k, normalize(v, paths)))
            .collect(),
        Value::Array(values) => values.into_iter().map(|v| normalize(v, paths)).collect(),
        other => other,
    }
}
fn project(prefs: &UserPrefs, paths: &RuntimePaths) -> Value {
    normalize(
        json!({"avatar":prefs.avatar,"notify_sound":prefs.public_json()["notify_sound"]}),
        paths,
    )
}
fn file_observation(path: &Path) -> Value {
    if path.is_dir() {
        return "directory".into();
    }
    std::fs::read(path).map_or(Value::Null, |bytes| digest(&bytes))
}

#[test]
fn pinned_go_media_corpus_matches_body_headers_and_failure_state() {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/services/preference-media/cases.json"
    ))
    .unwrap();
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/services/preference-media/go_observations.json"
    ))
    .unwrap();
    let expected = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 170);
    assert_eq!(cases.len(), expected.len());
    for (case, expected) in cases.iter().zip(expected) {
        let f = fixture(case.initial.clone(), case.missing_root);
        for (key, payload) in &case.media {
            let path = f.paths.resource(if key == "avatar" {
                Resource::Avatar
            } else {
                Resource::NotifySound
            });
            if payload.directory {
                std::fs::create_dir(path).unwrap();
            } else {
                std::fs::write(&path, payload.bytes()).unwrap();
                #[cfg(unix)]
                if payload.write_only {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o200))
                        .unwrap();
                }
                #[cfg(windows)]
                let _ = payload.write_only;
            }
        }
        if case.fail_save {
            std::fs::create_dir(f.paths.resource(Resource::Config)).unwrap();
        }
        let mut request = request(&case.method, &case.path, &case.body.bytes());
        request
            .headers
            .push(("Content-Type".into(), case.content_type.clone()));
        let warnings = AtomicUsize::new(0);
        let response = handle_authenticated(&request, &f.store, &f.paths, |message| {
            assert_eq!(message, "save config failed");
            warnings.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        let body = if response.headers["Content-Type"] == "application/json" {
            let mut body: Value = serde_json::from_slice(&response.body).unwrap();
            if response.status == 500 {
                let prefix = body["detail"]
                    .as_str()
                    .unwrap()
                    .split(':')
                    .next()
                    .unwrap()
                    .to_string();
                body["detail"] = format!("{prefix}: <owned filesystem error>").into();
            }
            body
        } else if response.body.len() < 1024 {
            json!({"base64":STANDARD.encode(&response.body)})
        } else {
            digest(&response.body)
        };
        let mut headers = BTreeMap::new();
        for name in ["Content-Type", "Cache-Control", "Content-Disposition"] {
            headers.insert(
                name,
                response.headers.get(name).cloned().unwrap_or_default(),
            );
        }
        assert_eq!(
            response.headers["X-Content-Type-Options"], "nosniff",
            "{}",
            case.name
        );
        #[cfg(unix)]
        for (key, payload) in &case.media {
            if payload.write_only {
                use std::os::unix::fs::PermissionsExt;
                let path = f.paths.resource(if key == "avatar" {
                    Resource::Avatar
                } else {
                    Resource::NotifySound
                });
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        let runtime = project(&f.store.snapshot().unwrap().config.user_prefs, &f.paths);
        let saved = std::fs::read_to_string(f.paths.resource(Resource::Config))
            .ok()
            .map(|text| {
                project(
                    &Config::from_yaml(&text, &f.paths).unwrap().user_prefs,
                    &f.paths,
                )
            });
        let detail_prefix = if response.status == 500 {
            body["detail"].as_str().unwrap().split(':').next().unwrap()
        } else {
            ""
        };
        let observed = json!({"name":case.name,"status":response.status,"body":body,"headers":headers,"runtime":runtime,"saved":saved,"files":{"avatar":file_observation(&f.paths.resource(Resource::Avatar)),"sound":file_observation(&f.paths.resource(Resource::NotifySound))},"source_detail_prefix":detail_prefix});
        assert_eq!(&observed, expected, "source corpus case {}", case.name);
        assert_eq!(
            warnings.load(Ordering::SeqCst),
            usize::from(response.status == 500 && observed["body"]["error"] == "save_failed"),
            "{}",
            case.name
        );
        if response.status == 200 && case.method == "GET" {
            let key = if case.path == AVATAR_PATH {
                "avatar"
            } else {
                "sound"
            };
            assert_eq!(
                response.body,
                case.media[key].bytes(),
                "exact binary bytes {}",
                case.name
            );
        }
        if response.status == 200 && case.method != "GET" {
            assert_eq!(response.body, b"{\"ok\":true}\n");
        }
    }
}

#[test]
fn source_manifest_matches_actual_pinned_go_declarations() {
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/services/preference-media/go_observations.json"
    ))
    .unwrap();
    assert_eq!(
        manifest["baseline"],
        "21d0bc7935a2c4696fb89ccff2e324157a528c2d"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for (path, digest) in manifest["source_sha256"].as_object().unwrap() {
        let bytes = std::fs::read_to_string(root.join(path))
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(
            Sha256::digest(bytes.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            digest.as_str().unwrap(),
            "baseline source changed: {path}"
        );
    }
}

#[test]
fn failed_save_keeps_binary_and_publication_then_successful_retry_persists() {
    let f = fixture(
        json!({"avatar":"https://example.invalid/old","notify_sound":{"enabled":true,"type":"custom"}}),
        false,
    );
    let cfg = f.paths.resource(Resource::Config);
    std::fs::create_dir(&cfg).unwrap();
    let png = b"\x89PNG\r\n\x1a\nsynthetic";
    assert_eq!(call(&f, "PUT", AVATAR_UPLOAD_PATH, png).status, 500);
    assert_eq!(f.store.snapshot().unwrap().revision, 1);
    assert_eq!(call(&f, "GET", AVATAR_PATH, b"").body, png);
    std::fs::remove_dir(&cfg).unwrap();
    assert_eq!(call(&f, "PUT", SOUND_PATH, b"ID3synthetic").status, 200);
    let saved = Config::from_yaml(&std::fs::read_to_string(cfg).unwrap(), &f.paths).unwrap();
    assert_eq!(
        saved.user_prefs.avatar,
        f.paths.resource(Resource::Avatar).to_string_lossy()
    );
    assert!(saved.user_prefs.notify_sound.enabled);
    assert_eq!(saved.user_prefs.notify_sound.r#type, "custom");
    assert_eq!(saved.user_prefs.notify_sound.custom_mime, "audio/mpeg");
    assert_eq!(
        call(
            &f,
            "DELETE",
            AVATAR_UPLOAD_PATH,
            &vec![0; AVATAR_MAX_BYTES + 1]
        )
        .status,
        200
    );
    assert_eq!(call(&f, "GET", AVATAR_PATH, b"").status, 404);
    assert_eq!(
        std::fs::read(f.paths.resource(Resource::Avatar)).unwrap(),
        png
    );
    assert_eq!(
        std::fs::read(f.paths.resource(Resource::NotifySound)).unwrap(),
        b"ID3synthetic"
    );
}

#[test]
fn existing_file_is_truncated_without_replacing_its_mode() {
    let f = fixture(json!({}), false);
    let path = f.paths.resource(Resource::Avatar);
    std::fs::write(&path, vec![0; 256]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    assert_eq!(call(&f, "PUT", AVATAR_UPLOAD_PATH, b"GIF89a").status, 200);
    assert_eq!(std::fs::read(&path).unwrap(), b"GIF89a");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    #[cfg(unix)]
    let sound = f.paths.resource(Resource::NotifySound);
    assert_eq!(call(&f, "PUT", SOUND_PATH, b"ID3").status, 200);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&sound).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn exact_routes_only_and_binary_get_does_not_create_root() {
    let f = fixture(json!({}), true);
    for path in [
        "/api/avatar/",
        "/api/user-prefs/avatar/",
        "/api/user-prefs/notify-sound-custom/",
        "/api/user-prefs",
    ] {
        assert!(!is_route(path));
        assert!(
            handle_authenticated(
                &request("PUT", path, b"ID3"),
                &f.store,
                &f.paths,
                |_| panic!("unexpected warning")
            )
            .is_none()
        );
    }
    assert_eq!(call(&f, "GET", AVATAR_PATH, b"").status, 404);
    assert_eq!(call(&f, "GET", SOUND_PATH, b"").status, 404);
    assert!(!f.paths.root().exists());
    assert_eq!(f.store.snapshot().unwrap().revision, 0);
}

#[test]
fn explicit_trial_root_keeps_both_media_and_config_in_selected_tree() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49962, installed.path()).unwrap();
    let store = ConfigStore::new(paths.clone(), Config::default()).unwrap();
    for (path, bytes) in [
        (AVATAR_UPLOAD_PATH, b"GIF87a".as_slice()),
        (SOUND_PATH, b"ID3".as_slice()),
    ] {
        assert_eq!(
            handle_authenticated(&request("PUT", path, bytes), &store, &paths, |_| {})
                .unwrap()
                .status,
            200
        );
    }
    assert!(paths.resource(Resource::Avatar).is_file());
    assert!(paths.resource(Resource::NotifySound).is_file());
    assert!(paths.resource(Resource::Config).is_file());
    assert_eq!(std::fs::read_dir(installed.path()).unwrap().count(), 0);
}

#[test]
fn concurrent_avatar_sound_and_unrelated_preference_writes_keep_each_change() {
    let f = fixture(json!({"display_name":"initial"}), false);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let threads: Vec<_> = (0..3)
        .map(|kind| {
            let store = f.store.clone();
            let paths = f.paths.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for number in 0..16 {
                    if kind < 2 {
                        let (path, prefix) = if kind == 0 {
                            (AVATAR_UPLOAD_PATH, "GIF89a")
                        } else {
                            (SOUND_PATH, "ID3")
                        };
                        let bytes = format!("{prefix}synthetic-{number}");
                        assert_eq!(
                            handle_authenticated(
                                &request("PUT", path, bytes.as_bytes()),
                                &store,
                                &paths,
                                |_| {}
                            )
                            .unwrap()
                            .status,
                            200
                        );
                    } else {
                        loop {
                            let mut snapshot = store.snapshot().unwrap();
                            snapshot.config.user_prefs.display_name = format!("synthetic-{number}");
                            match store.persist(snapshot.revision, snapshot.config) {
                                Ok(_) => break,
                                Err(ConfigError::Conflict { .. }) => continue,
                                Err(error) => panic!("synthetic config save failed: {error}"),
                            }
                        }
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let snapshot = f.store.snapshot().unwrap();
    assert_eq!(snapshot.revision, 48);
    assert_eq!(snapshot.config.user_prefs.display_name, "synthetic-15");
    assert_eq!(
        snapshot.config.user_prefs.notify_sound.custom_mime,
        "audio/mpeg"
    );
    assert_eq!(
        call(&f, "GET", AVATAR_PATH, b"").body,
        b"GIF89asynthetic-15"
    );
    assert_eq!(call(&f, "GET", SOUND_PATH, b"").body, b"ID3synthetic-15");
    let saved = Config::from_yaml(
        &std::fs::read_to_string(f.paths.resource(Resource::Config)).unwrap(),
        &f.paths,
    )
    .unwrap();
    assert_eq!(saved.user_prefs, snapshot.config.user_prefs);
}

#[test]
fn existing_other_local_file_is_never_an_avatar_source() {
    let f = fixture(json!({}), false);
    let mut snapshot = f.store.snapshot().unwrap();
    snapshot.config.token = "synthetic-private-value".into();
    snapshot.config.user_prefs.avatar = f
        .paths
        .resource(Resource::Config)
        .to_string_lossy()
        .into_owned();
    f.store.persist(snapshot.revision, snapshot.config).unwrap();
    let response = call(&f, "GET", AVATAR_PATH, b"");
    assert_eq!(response.status, 404);
    assert_eq!(response.body, b"404 page not found\n");
}

#[test]
fn simultaneous_first_uploads_to_same_media_file_succeed() {
    for (path, bytes) in [
        (AVATAR_UPLOAD_PATH, b"GIF89asynthetic".as_slice()),
        (SOUND_PATH, b"ID3synthetic".as_slice()),
    ] {
        for _ in 0..2 {
            let f = fixture(json!({}), false);
            let barrier = Arc::new(std::sync::Barrier::new(16));
            let threads: Vec<_> = (0..16)
                .map(|_| {
                    let store = f.store.clone();
                    let paths = f.paths.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        handle_authenticated(&request("PUT", path, bytes), &store, &paths, |_| {})
                            .unwrap()
                    })
                })
                .collect();
            for thread in threads {
                let response = thread.join().unwrap();
                assert_eq!(
                    response.status,
                    200,
                    "{}",
                    String::from_utf8_lossy(&response.body)
                );
            }
            assert_eq!(f.store.snapshot().unwrap().revision, 16);
            let get = if path == AVATAR_UPLOAD_PATH {
                AVATAR_PATH
            } else {
                SOUND_PATH
            };
            assert_eq!(call(&f, "GET", get, b"").body, bytes);
        }
    }
}

#[cfg(windows)]
fn fixture_junction(source: &Path, target: &Path) -> io::Result<()> {
    use std::{
        fs::File,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle},
        },
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, OPEN_EXISTING,
        },
        System::IO::DeviceIoControl,
    };
    let absolute = std::path::absolute(source)?;
    let display: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    // Rust canonical RuntimePaths can carry a Win32 verbatim prefix. The NT
    // namespace uses \??\ directly; never build the invalid \??\\\?\ form.
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let native = display
        .strip_prefix(verbatim.as_slice())
        .unwrap_or(&display);
    let substitute = "\\??\\"
        .encode_utf16()
        .chain(native.iter().copied())
        .collect::<Vec<_>>();
    let size = 16 + (substitute.len() + display.len() + 2) * 2;
    if size > 16 * 1024 || display.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid junction target",
        ));
    }
    let mut buffer = vec![0_u8; size];
    buffer[..4].copy_from_slice(&0xA0000003_u32.to_le_bytes());
    for (offset, value) in [
        (4, size - 8),
        (10, substitute.len() * 2),
        (12, (substitute.len() + 1) * 2),
        (14, display.len() * 2),
    ] {
        buffer[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
    }
    let units = substitute
        .into_iter()
        .chain(Some(0))
        .chain(display)
        .chain(Some(0));
    for (index, unit) in units.enumerate() {
        buffer[16 + index * 2..18 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    std::fs::create_dir(target)?;
    let result = (|| {
        let target = target
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: the target buffer is terminated. Returned handle is transferred
        // once into File. No subprocess, shell, registry mutation or credential.
        let raw = unsafe {
            CreateFileW(
                target.as_ptr(),
                GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_handle(raw) };
        let mut returned = 0;
        if unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                0x000900A4,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir(target);
    }
    result
}

#[cfg(windows)]
#[test]
fn windows_reparse_upload_target_and_ancestor_rejected_before_write() {
    let f = fixture(json!({}), false);
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel.bin");
    std::fs::write(&sentinel, b"outside-sentinel").unwrap();
    // A junction occupying the media basename is an unprivileged native
    // reparse fixture. The owned writer must reject it without truncation.
    let avatar = f.paths.resource(Resource::Avatar);
    fixture_junction(outside.path(), &avatar).unwrap();
    assert_eq!(call(&f, "PUT", AVATAR_UPLOAD_PATH, b"GIF89a").status, 500);
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside-sentinel");
    assert!(!outside.path().join("user_avatar.bin").exists());
    assert_eq!(f.store.snapshot().unwrap().revision, 0);
    std::fs::remove_dir(&avatar).unwrap();
    // Reject a junction ancestor as well; acquisition must not follow it.
    let ancestor = f._temp.path().join("linked-directory");
    fixture_junction(outside.path(), &ancestor).unwrap();
    assert!(Dir::open(&ancestor).is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside-sentinel");
    std::fs::remove_dir(&ancestor).unwrap();
}
