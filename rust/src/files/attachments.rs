use super::{FilesService, Result, content, err, safe_fs::Dir, time};
use crate::{
    config::paths::Resource,
    hub::http::{Request, Response},
    proto::core::{HistoryEvent, LiveSessionId, SessionCore, SessionStorage},
};
use serde_json::json;
use std::{
    io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
pub const UPLOAD_MAX: usize = 10 * 1024 * 1024;
pub const MULTIPART_MAX: usize = UPLOAD_MAX + 1024 * 1024;
impl FilesService {
    pub(super) fn attach(
        &self,
        r: &Request,
        core: &dyn SessionCore,
        storage: Option<&dyn SessionStorage>,
    ) -> Result<Response> {
        self.require_attachment_history()?;
        let (session, filename, data) = multipart(r)?;
        let snap = core
            .snapshot(LiveSessionId(session))
            .filter(|s| !s.provider.is_empty())
            .ok_or_else(|| err(404, "not_found", "session not found"))?;
        let saved = self.save_attachment(session, &snap.provider, &filename, &data, storage)?;
        Ok(Response::json(200, &saved))
    }
    /// Authenticated UI WS attachment caller. The Hub intentionally sends no
    /// attachment reply; the immutable history event feeds its normal history.
    pub fn record_ws_attachment(
        &self,
        message: &crate::proto::Message,
        core: &dyn SessionCore,
        storage: Option<&dyn SessionStorage>,
    ) -> Result<()> {
        self.require_attachment_history()?;
        use base64::{
            Engine, alphabet,
            engine::{GeneralPurpose, GeneralPurposeConfig},
        };
        if message.image_data.is_empty() {
            return Err(err(400, "bad_request", "missing image_data"));
        }
        if message.image_data.len() > 4 * 1024 * 1024 {
            return Err(err(
                400,
                "bad_request",
                "attachment message exceeds WebSocket limit",
            ));
        }
        let engine = GeneralPurpose::new(
            &alphabet::STANDARD,
            GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
        );
        let data = engine
            .decode(message.image_data.replace(['\r', '\n'], ""))
            .map_err(|_| err(400, "bad_request", "invalid base64"))?;
        let provider = core
            .snapshot(LiveSessionId(message.session_id))
            .map(|s| s.provider)
            .unwrap_or_default();
        self.save_attachment(
            message.session_id,
            &provider,
            &message.filename,
            &data,
            storage,
        )?;
        Ok(())
    }
    fn require_attachment_history(&self) -> Result<()> {
        if self.history.is_none() {
            return Err(err(
                503,
                "attachment_history_unavailable",
                "attachment history is not initialized",
            ));
        }
        Ok(())
    }
    pub(super) fn save_attachment(
        &self,
        session: i64,
        provider: &str,
        filename: &str,
        data: &[u8],
        _storage: Option<&dyn SessionStorage>,
    ) -> Result<serde_json::Value> {
        if data.len() > UPLOAD_MAX {
            return Err(err(400, "bad_request", "file too large"));
        }
        let root =
            Dir::open(self.paths.root()).map_err(|e| super::io_error(e, "home_dir_error"))?;
        let attach = root
            .child_dir("attachments", true)
            .map_err(|e| super::io_error(e, "save_failed"))?;
        let dir = attach
            .child_dir(&session.to_string(), true)
            .map_err(|e| super::io_error(e, "save_failed"))?;
        let now = SystemTime::now();
        let name = sanitized_filename(filename, data);
        let stamp = time::format(now)?;
        let date: String = stamp[..19].chars().filter(|c| c.is_ascii_digit()).collect();
        let prefix = format!(
            "{date}_{:09}",
            now.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        );
        let mut saved_name = String::new();
        for i in 0..1000 {
            let n = if i == 0 {
                format!("{prefix}_{name}")
            } else {
                format!("{prefix}_{i}_{name}")
            };
            match dir.create_new(&n, data, 0o600) {
                Ok(()) => {
                    saved_name = n;
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(super::io_error(e, "save_failed")),
            }
        }
        if saved_name.is_empty() {
            return Err(err(
                500,
                "save_failed",
                "attachment filename collision limit reached",
            ));
        }
        let saved = dir.path().join(&saved_name);
        let inject = match inject(provider, &saved) {
            Ok(s) => s,
            Err(e) => {
                let _ = dir.remove_file(&saved_name);
                return Err(super::io_error(e, "save_failed"));
            }
        };
        if let Some(history) = &self.history {
            let value = json!({"ts":crate::proto::time::format_rfc3339(now).map_err(|_|err(500,"invalid_timestamp","invalid attachment timestamp"))?,"type":"attach","session_id":session,"path":saved,"filename":filename,"provider":provider});
            if let serde_json::Value::Object(obj) = value {
                history
                    .apply(crate::proto::core::PersistenceEffect::Event {
                        session: LiveSessionId(session),
                        event: HistoryEvent(obj.into_iter().collect()),
                    })
                    .map_err(|_| {
                        err(
                            500,
                            "attachment_history_failed",
                            "attachment history could not be completed",
                        )
                    })?;
            }
        }
        Ok(json!({"ok":true,"inject":inject,"saved_path":saved,"filename":filename}))
    }
    pub(super) fn purge_attachments(&self, core: &dyn SessionCore) -> Result<Response> {
        self.purge_attachment_ids(&core.active_ids())
    }
    pub(super) fn purge_attachment_ids(&self, active_ids: &[LiveSessionId]) -> Result<Response> {
        let root = match Dir::open(&self.paths.resource(Resource::Attachments)) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Response::json(200, &json!({"ok":true,"folders":0})));
            }
            Err(e) => return Err(super::io_error(e, "readdir_failed")),
        };
        let active: std::collections::HashSet<_> =
            active_ids.iter().map(|id| id.0.to_string()).collect();
        let mut folders = 0;
        for name in root
            .entries()
            .map_err(|e| super::io_error(e, "readdir_failed"))?
        {
            if active.contains(&name) {
                continue;
            }
            if root.child_dir(&name, false).is_ok() && root.remove_tree(&name).is_ok() {
                folders += 1;
            }
        }
        Ok(Response::json(200, &json!({"ok":true,"folders":folders})))
    }
    /// Maintenance caller supplies configured values and the clock. Disabled
    /// retention and size limits remain no-ops; symlinks are never traversed.
    pub fn clean_attachments(
        &self,
        retention_days: i64,
        max_bytes: u64,
        now: SystemTime,
    ) -> io::Result<usize> {
        if retention_days <= 0 && max_bytes == 0 {
            return Ok(0);
        }
        let root = match Dir::open(&self.paths.resource(Resource::Attachments)) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut files = Vec::new();
        for session in root.entries()? {
            let Ok(dir) = root.child_dir(&session, false) else {
                continue;
            };
            for name in dir.entries()? {
                if let Ok(meta) = dir.metadata(&name) {
                    files.push((meta.modified()?, meta.len(), session.clone(), name));
                }
            }
        }
        files.sort_by_key(|f| f.0);
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        let mut count = 0;
        let local: chrono::DateTime<chrono::Local> = now.into();
        let cutoff: Option<SystemTime> = local
            .checked_sub_days(chrono::Days::new(retention_days.max(0) as u64))
            .map(Into::into);
        for (mtime, size, session, name) in files {
            if ((retention_days > 0 && cutoff.is_some_and(|t| mtime < t))
                || (max_bytes > 0 && total > max_bytes))
                && let Ok(dir) = root.child_dir(&session, false)
                && dir.remove_file(&name).is_ok()
            {
                total = total.saturating_sub(size);
                count += 1;
            }
        }
        for name in root.entries()? {
            if let Ok(dir) = root.child_dir(&name, false)
                && dir.entries()?.is_empty()
            {
                drop(dir);
                let _ = root.remove_tree(&name);
            }
        }
        Ok(count)
    }
}
fn parameter(value: &str, key: &str) -> Option<String> {
    let mut rest = value.split_once(';')?.1;
    while !rest.trim_start().is_empty() {
        rest = rest.trim_start();
        let (eq_name, after) = rest.split_once('=')?;
        let name = eq_name.trim();
        let mut v = String::new();
        rest = after.trim_start();
        if rest.starts_with('"') {
            let mut escape = false;
            let mut end = None;
            for (i, c) in rest[1..].char_indices() {
                if escape {
                    v.push(c);
                    escape = false;
                } else if c == '\\' {
                    escape = true;
                } else if c == '"' {
                    end = Some(i + 2);
                    break;
                } else {
                    v.push(c);
                }
            }
            rest = &rest[end?..];
        } else {
            let end = rest.find(';').unwrap_or(rest.len());
            v = rest[..end].trim().into();
            rest = &rest[end..];
        }
        if name.eq_ignore_ascii_case(key) {
            return Some(v);
        }
        rest = rest.trim_start();
        if let Some(next) = rest.strip_prefix(';') {
            rest = next;
        } else {
            break;
        }
    }
    None
}
fn delimiter(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len())
        .enumerate()
        .find(|(i, w)| {
            *w == needle
                && matches!(
                    hay.get(i + needle.len()..i + needle.len() + 2),
                    Some(b"--") | Some(b"\r\n")
                )
        })
        .map(|(i, _)| i)
}
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
fn multipart(r: &Request) -> Result<(i64, String, Vec<u8>)> {
    if r.body.len() > MULTIPART_MAX {
        return Err(err(
            400,
            "bad_request",
            "bad request: http: request body too large",
        ));
    }
    let c = r.header("content-type");
    if !c
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return Err(err(
            400,
            "bad_request",
            "bad request: request Content-Type isn't multipart/form-data",
        ));
    }
    let boundary = parameter(c, "boundary")
        .filter(|b| !b.is_empty() && b.len() <= 70 && !b.contains(['\r', '\n']))
        .ok_or_else(|| err(400, "bad_request", "invalid multipart boundary"))?;
    let marker = format!("--{boundary}").into_bytes();
    let mut rest = r.body.as_slice();
    let query_sid = r.query("session_id");
    let mut sid = if query_sid.is_empty() {
        None
    } else {
        Some(query_sid)
    };
    let mut file = None;
    let mut parts = 0;
    if !rest.starts_with(&marker) {
        return Err(err(400, "bad_request", "invalid multipart body"));
    }
    rest = &rest[marker.len()..];
    while !rest.starts_with(b"--") {
        parts += 1;
        if parts > 1000 {
            return Err(err(400, "bad_request", "too many multipart parts"));
        }
        if !rest.starts_with(b"\r\n") {
            return Err(err(400, "bad_request", "invalid multipart body"));
        }
        rest = &rest[2..];
        let head_end = find(rest, b"\r\n\r\n")
            .filter(|n| *n <= 64 * 1024)
            .ok_or_else(|| err(400, "bad_request", "invalid multipart headers"))?;
        let headers = String::from_utf8_lossy(&rest[..head_end]);
        let cd = headers
            .split("\r\n")
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.eq_ignore_ascii_case("content-disposition"))
            .map(|(_, v)| v.trim())
            .unwrap_or("");
        let field = parameter(cd, "name").unwrap_or_default();
        let filename = parameter(cd, "filename");
        rest = &rest[head_end + 4..];
        let next = [b"\r\n".as_slice(), marker.as_slice()].concat();
        let end = delimiter(rest, &next)
            .ok_or_else(|| err(400, "bad_request", "multipart body ended unexpectedly"))?;
        let data = &rest[..end];
        if field == "session_id" && sid.is_none() {
            sid = Some(String::from_utf8_lossy(data).into_owned());
        }
        if field == "file"
            && file.is_none()
            && let Some(filename) = filename.filter(|n| !n.is_empty())
        {
            if data.len() > UPLOAD_MAX {
                return Err(err(400, "bad_request", "file too large"));
            }
            let filename = Path::new(&filename)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            file = Some((filename, data.to_vec()));
        }
        rest = &rest[end + next.len()..];
    }
    let sid = sid
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| err(400, "bad_request", "invalid session_id"))?;
    let (name, data) = file.ok_or_else(|| err(400, "bad_request", "missing file"))?;
    Ok((sid, name, data))
}
pub fn sanitized_filename(filename: &str, data: &[u8]) -> String {
    let raw = filename.rsplit(['/', '\\']).next().unwrap_or("");
    let candidate = content::extension(Path::new(raw));
    let ext = if (2..=32).contains(&candidate.len())
        && candidate[1..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        candidate
    } else if data.len() >= 8 && data.starts_with(b"\x89PNG") {
        ".png".into()
    } else if data.starts_with(b"\xff\xd8\xff") {
        ".jpg".into()
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        ".gif".into()
    } else if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        ".webp".into()
    } else if data.starts_with(b"%PDF-") {
        ".pdf".into()
    } else {
        ".bin".into()
    };
    let stem = if raw.to_lowercase().ends_with(&ext) {
        &raw[..raw.len() - ext.len()]
    } else {
        raw
    };
    let mut out = String::new();
    for c in stem.chars() {
        let c = if allowed_filename_char(c) { c } else { '_' };
        if c == '_' && (out.is_empty() || out.ends_with('_')) {
            continue;
        }
        if out.len() + c.len_utf8() > 180 - ext.len() {
            break;
        }
        out.push(c);
    }
    let stem = out.trim_matches(['.', '_', '-']);
    format!("{}{ext}", if stem.is_empty() { "attachment" } else { stem })
}
pub fn inject(provider: &str, path: &Path) -> io::Result<String> {
    let path = path.to_string_lossy();
    static FORBIDDEN: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"[\p{Cc}\p{Cf}]").unwrap());
    if FORBIDDEN.is_match(&path) {
        return Err(io::Error::other(
            "attachment path contains unsupported character",
        ));
    }
    Ok(match provider {
        "claude" => format!("@{path} "),
        "codex" => format!("@{} ", path.replace('\\', "/")),
        _ => format!("{path} "),
    })
}

fn allowed_filename_char(c: char) -> bool {
    static ALLOWED: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^[\p{L}\p{N}\p{M}_.-]$").unwrap());
    ALLOWED.is_match(c.encode_utf8(&mut [0; 4]))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn upload(data: &[u8]) -> Request {
        let mut body="--bound\r\nContent-Disposition: form-data; name=\"session_id\"\r\n\r\n2\r\n--bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"日本語; sample.txt\"\r\nContent-Type: text/plain\r\n\r\n".as_bytes().to_vec();
        body.extend(data);
        body.extend(b"\r\n--bound--\r\n");
        Request {
            method: "POST".into(),
            path: "/api/attach".into(),
            headers: vec![(
                "Content-Type".into(),
                "multipart/form-data; boundary=bound".into(),
            )],
            body,
            ..Default::default()
        }
    }
    #[test]
    fn bounded_multipart_preserves_quoted_filename_and_false_boundary() {
        let data = b"prefix\r\n--boundNOT-A-BOUNDARY\r\nbytes";
        let (sid, name, bytes) = multipart(&upload(data)).unwrap();
        assert_eq!(sid, 2);
        assert_eq!(name, "日本語; sample.txt");
        assert_eq!(bytes, data);
        assert!(multipart(&upload(&vec![0; UPLOAD_MAX + 1])).is_err());
        let mut too_large = upload(b"");
        too_large.body = vec![0; MULTIPART_MAX + 1];
        assert!(multipart(&too_large).is_err());
    }
    #[test]
    fn multipart_query_precedence_and_duplicate_invalid_sid() {
        let mut r = upload(b"sample");
        r.query = "session_id=7".into();
        assert_eq!(multipart(&r).unwrap().0, 7);
        let mut r = upload(b"sample");
        r.body=String::from_utf8(r.body).unwrap().replace("\r\n2\r\n", "\r\nnot-an-int\r\n--bound\r\nContent-Disposition: form-data; name=\"session_id\"\r\n\r\n2\r\n").into_bytes();
        assert!(multipart(&r).is_err());
    }
    #[test]
    fn all_unicode_marks_survive_and_formatting_chars_are_denied() {
        assert!(allowed_filename_char('\u{3099}'));
        assert!(allowed_filename_char('\u{0651}'));
        assert!(!allowed_filename_char('\u{200e}'));
        assert!(inject("claude", Path::new("/owned/\u{061c}.md")).is_err());
    }
}
