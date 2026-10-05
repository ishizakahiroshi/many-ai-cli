//! Fixed-path avatar and custom notification-sound media handlers.
//!
//! The Hub owns authentication/Host/Origin/PIN checks. These handlers use only
//! the canonical ConfigStore and RuntimePaths and never fetch avatar URLs.
//! Source binary mutations write the media, publish preferences, then save;
//! a failed config save deliberately retains the binary and published value.
use super::http::{Request, Response, require_method};
use crate::{
    config::{Config, ConfigError, ConfigStore, Resource, RuntimePaths},
    files::safe_fs::Dir,
    proto::unicode::simple_lower,
    routine::memo::sniff_image,
};
use std::{
    io::{self, Write},
    path::Path,
};

pub const AVATAR_PATH: &str = "/api/avatar";
pub const AVATAR_UPLOAD_PATH: &str = "/api/user-prefs/avatar";
pub const SOUND_PATH: &str = "/api/user-prefs/notify-sound-custom";
pub const AVATAR_MAX_BYTES: usize = 5 * 1024 * 1024;
pub const SOUND_MAX_BYTES: usize = 2 * 1024 * 1024;

pub fn is_route(path: &str) -> bool {
    matches!(path, AVATAR_PATH | AVATAR_UPLOAD_PATH | SOUND_PATH)
}

/// The warning callback receives only the fixed diagnostic "save config failed".
/// It runs outside ConfigStore's lock and must not expose binary payload bytes.
pub fn handle_authenticated(
    request: &Request,
    store: &ConfigStore,
    paths: &RuntimePaths,
    warning: impl Fn(&str),
) -> Option<Response> {
    let methods: &[&str] = match request.path.as_str() {
        AVATAR_PATH => &["GET"],
        AVATAR_UPLOAD_PATH => &["PUT", "DELETE"],
        SOUND_PATH => &["GET", "PUT"],
        _ => return None,
    };
    if let Err(response) = require_method(request, methods) {
        return Some(response);
    }
    Some(match (request.path.as_str(), request.method.as_str()) {
        (AVATAR_PATH, _) => avatar_get(store, paths),
        (AVATAR_UPLOAD_PATH, "DELETE") => {
            // Source never reads the DELETE body or removes user_avatar.bin.
            publish_media(store, |cfg| cfg.user_prefs.avatar.clear(), warning)
        }
        (AVATAR_UPLOAD_PATH, _) => put(request, store, paths, true, warning),
        (SOUND_PATH, "GET") => sound_get(store, paths),
        (SOUND_PATH, _) => put(request, store, paths, false, warning),
        _ => unreachable!("recognized media route"),
    })
}

fn avatar_get(store: &ConfigStore, paths: &RuntimePaths) -> Response {
    let snapshot = match store.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => return save_error(error),
    };
    let avatar = &snapshot.config.user_prefs.avatar;
    if avatar.is_empty() || avatar.starts_with("http://") || avatar.starts_with("https://") {
        return avatar_not_found();
    }
    let path = paths.resource(Resource::Avatar);
    if candidate_key(avatar) != candidate_key(&path.to_string_lossy()) {
        return avatar_not_found();
    }
    let bytes = match read_media(&path) {
        Ok(bytes) => bytes,
        Err(_) => return avatar_not_found(),
    };
    let mut response = Response::bytes(
        200,
        image_mime(&bytes).unwrap_or("application/octet-stream"),
        bytes,
    );
    response
        .headers
        .insert("Cache-Control".into(), "max-age=3600".into());
    response
}
fn avatar_not_found() -> Response {
    Response::bytes(
        404,
        "text/plain; charset=utf-8",
        b"404 page not found\n".to_vec(),
    )
}
fn sound_get(store: &ConfigStore, paths: &RuntimePaths) -> Response {
    let bytes = match read_media(&paths.resource(Resource::NotifySound)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Response::error(404, "not_found", "not found");
        }
        Err(error) => return io_error("read_error", "read error", error),
    };
    let snapshot = match store.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => return save_error(error),
    };
    // GET reads the fixed file even when custom_file is empty/another path.
    // Preserve a historical allowed MIME's exact case/parameters/whitespace.
    let mime = &snapshot.config.user_prefs.notify_sound.custom_mime;
    let mime = if audio_mime_allowed(mime) {
        mime.as_str()
    } else {
        "application/octet-stream"
    };
    let mut response = Response::bytes(200, mime, bytes);
    response.headers.insert(
        "Content-Disposition".into(),
        "inline; filename=\"notify-sound\"".into(),
    );
    response
}
fn put(
    request: &Request,
    store: &ConfigStore,
    paths: &RuntimePaths,
    avatar: bool,
    warning: impl Fn(&str),
) -> Response {
    let limit = if avatar {
        AVATAR_MAX_BYTES
    } else {
        SOUND_MAX_BYTES
    };
    if request.body.len() > limit {
        return Response::error(
            400,
            "bad_request",
            "read body error: http: request body too large",
        );
    }
    let mime = if avatar {
        image_mime(&request.body)
    } else {
        audio_mime(&request.body)
    };
    let Some(mime) = mime else {
        return Response::error(
            415,
            "bad_request",
            if avatar {
                "image/png, image/jpeg, image/gif, or image/webp required"
            } else {
                "audio/* MIME type required"
            },
        );
    };
    let path = paths.resource(if avatar {
        Resource::Avatar
    } else {
        Resource::NotifySound
    });
    if let Err(error) = write_media(&path, &request.body) {
        return io_error("write_error", "write error", error);
    }
    let value = path.to_string_lossy().into_owned();
    publish_media(
        store,
        |cfg| {
            if avatar {
                cfg.user_prefs.avatar = value.clone();
            } else {
                cfg.user_prefs.notify_sound.custom_file = value.clone();
                cfg.user_prefs.notify_sound.custom_mime = mime.into();
            }
        },
        warning,
    )
}
fn publish_media(
    store: &ConfigStore,
    change: impl Fn(&mut Config),
    warning: impl Fn(&str),
) -> Response {
    loop {
        let mut snapshot = match store.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                warning("save config failed");
                return save_error(error);
            }
        };
        change(&mut snapshot.config);
        match store.publish_then_persist_legacy_with(snapshot.revision, snapshot.config, |_| {}) {
            Ok(_) => return Response::json(200, &serde_json::json!({"ok": true})),
            Err(ConfigError::Conflict { .. }) => continue,
            Err(error) => {
                warning("save config failed");
                return save_error(error);
            }
        }
    }
}
fn save_error(error: ConfigError) -> Response {
    Response::error(500, "save_failed", &format!("save failed: {error}"))
}
fn io_error(code: &str, prefix: &str, error: io::Error) -> Response {
    Response::error(500, code, &format!("{prefix}: {error}"))
}
fn read_media(path: &Path) -> io::Result<Vec<u8>> {
    let dir = Dir::open(path.parent().expect("fixed media parent"))?;
    // Go GET uses os.ReadFile without applying its upload-size limit.
    dir.read(path.file_name().unwrap().to_str().unwrap(), usize::MAX)
}
fn write_media(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = Dir::open(path.parent().expect("fixed media parent"))?;
    let name = path.file_name().unwrap().to_str().unwrap();
    let mut file = dir.open_write_or_create(name, 0o600)?;
    file.set_len(0)?;
    file.write_all(bytes)
}
fn candidate_key(path: &str) -> String {
    let clean = clean_native_path(path, cfg!(windows));
    if cfg!(windows) {
        simple_lower(&clean)
    } else {
        clean
    }
}
fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    // Existing helper has the exact four accepted net/http/sniff.go signatures.
    sniff_image(bytes).map(|kind| match kind {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => unreachable!("fixed image sniff helper"),
    })
}
fn audio_mime_allowed(raw: &str) -> bool {
    let mime = simple_lower(raw.trim());
    mime.split_once(';')
        .map_or(mime.as_str(), |(base, _)| base.trim())
        .starts_with("audio/")
}
fn audio_mime(bytes: &[u8]) -> Option<&'static str> {
    // The only audio/* outputs in Go 1.26.8 net/http/sniff.go. Prefix signatures
    // are all within its 512-byte window. OggS is application/ogg (rejected),
    // despite the pinned handler's comment claiming OGG acceptance. MP3 frame
    // sync without ID3, AAC, FLAC, MP4 and WebM are likewise not accepted.
    // Provenance and source-observer hashes: fixtures/services/preference-media.
    if bytes.len() >= 12 && &bytes[..4] == b"FORM" && &bytes[8..12] == b"AIFF" {
        Some("audio/aiff")
    } else if bytes.starts_with(b"ID3") {
        Some("audio/mpeg")
    } else if bytes.starts_with(b"MThd\0\0\0\x06") {
        Some("audio/midi")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Some("audio/wave")
    } else {
        None
    }
}

/// Lexical native filepath.Clean semantics; this never resolves a symlink or
/// normalizes the supplied path by accessing the filesystem.
pub(crate) fn clean_native_path(path: &str, windows: bool) -> String {
    let source = if windows {
        path.replace('/', "\\")
    } else {
        path.into()
    };
    let sep = if windows { '\\' } else { '/' };
    let mut prefix = String::new();
    let mut rest = source.as_str();
    if windows && source.as_bytes().get(1) == Some(&b':') {
        prefix = source[..2].into();
        rest = &source[2..];
    } else if windows && source.starts_with("\\\\") {
        let mut ends = source[2..].match_indices('\\').map(|(i, _)| i + 2);
        if let Some(server_end) = ends.next() {
            let volume_end = ends.next().unwrap_or(source.len());
            if server_end > 2 {
                prefix = source[..volume_end].into();
                rest = &source[volume_end..];
            }
        }
    }
    let rooted = rest.starts_with(sep);
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split(sep) {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            ".." if rooted => {}
            part => parts.push(part),
        }
    }
    let separator = sep.to_string();
    let mut out = prefix;
    if rooted {
        out.push(sep);
    }
    out.push_str(&parts.join(&separator));
    if out.is_empty() || (windows && out.ends_with(':')) {
        out.push('.');
    }
    out
}

#[cfg(test)]
mod tests;
