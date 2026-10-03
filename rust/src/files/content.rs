use super::{
    FilesService, Result, WorkspaceReads, err,
    scope::{self, Scope},
    time,
};
use crate::{
    hub::http::{Request, Response},
    proto::core::SessionStorage,
};
use serde_json::{Value, json};
use std::{fs::File, io::Read, path::Path};
pub const CONTENT_MAX: usize = 1024 * 1024;
pub fn extension(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    name.rfind('.')
        .map(|i| name[i..].to_ascii_lowercase())
        .unwrap_or_default()
}
pub fn text_file(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        ".txt"
            | ".md"
            | ".markdown"
            | ".rst"
            | ".log"
            | ".json"
            | ".jsonl"
            | ".yaml"
            | ".yml"
            | ".toml"
            | ".ini"
            | ".cfg"
            | ".conf"
            | ".env"
            | ".csv"
            | ".tsv"
            | ".xml"
            | ".html"
            | ".htm"
            | ".css"
            | ".scss"
            | ".sass"
            | ".less"
            | ".js"
            | ".mjs"
            | ".cjs"
            | ".jsx"
            | ".ts"
            | ".tsx"
            | ".vue"
            | ".go"
            | ".rs"
            | ".py"
            | ".rb"
            | ".php"
            | ".java"
            | ".kt"
            | ".kts"
            | ".c"
            | ".cc"
            | ".cpp"
            | ".cxx"
            | ".h"
            | ".hh"
            | ".hpp"
            | ".cs"
            | ".sh"
            | ".bash"
            | ".zsh"
            | ".fish"
            | ".ps1"
            | ".psm1"
            | ".bat"
            | ".cmd"
            | ".sql"
            | ".graphql"
            | ".gql"
            | ".proto"
            | ".diff"
            | ".patch"
            | ".gitignore"
            | ".gitattributes"
            | ".editorconfig"
    ) || matches!(
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase()
            .as_str(),
        "dockerfile" | "makefile" | "readme" | "license" | "changelog" | "notice"
    )
}
pub fn media_file(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        ".png"
            | ".jpg"
            | ".jpeg"
            | ".gif"
            | ".webp"
            | ".bmp"
            | ".mp4"
            | ".webm"
            | ".ogv"
            | ".mov"
            | ".m4v"
    )
}
pub fn mime(path: &Path) -> &'static str {
    match extension(path).as_str() {
        ".png" => "image/png",
        ".jpg" | ".jpeg" => "image/jpeg",
        ".gif" => "image/gif",
        ".webp" => "image/webp",
        ".bmp" => "image/bmp",
        ".mp4" | ".m4v" => "video/mp4",
        ".webm" => "video/webm",
        ".ogv" => "video/ogg",
        ".mov" => "video/quicktime",
        ".pdf" => "application/pdf",
        ".json" => "application/json",
        ".html" | ".htm" => "text/html; charset=utf-8",
        ".css" => "text/css; charset=utf-8",
        ".txt" => "text/plain; charset=utf-8",
        ".csv" => "text/csv; charset=utf-8",
        _ => "application/octet-stream",
    }
}
impl FilesService {
    pub(super) fn read_handle(
        &self,
        r: &Request,
        s: &Scope,
        storage: Option<&dyn SessionStorage>,
        remote: bool,
        workspace: &dyn WorkspaceReads,
    ) -> Result<Response> {
        match r.path.as_str() {
            "/api/files-roots" => {
                let docs = s.git_root.join("docs");
                return Ok(Response::json(
                    200,
                    &json!({"gitRoot":s.git_root,"candidates":[{"name":"docs","absPath":docs,"exists":docs.is_dir()}]}),
                ));
            }
            "/api/files-list" => return list(r, s, remote),
            _ => {}
        }
        let grant = s.read_grant(r, storage, remote, workspace)?;
        let path = &grant.path;
        if r.path == "/api/files-content" && !text_file(path) {
            return Err(err(403, "forbidden", "not a previewable text file"));
        }
        if r.path == "/api/files-asset" && !media_file(path) {
            return Err(err(403, "forbidden", "not a previewable media file"));
        }
        if r.path == "/api/files-download"
            && grant.via_mention
            && !text_file(path)
            && !media_file(path)
        {
            return Err(err(
                403,
                "forbidden",
                "not a downloadable file outside allowed roots",
            ));
        }
        if std::fs::metadata(path).is_ok_and(|m| m.is_dir()) {
            return Err(err(400, "bad_request", "path is a directory"));
        }
        let (dir, name) =
            scope::parent_capability(path).map_err(|e| super::io_error(e, "open_failed"))?;
        let file = dir
            .open_file(&name, false)
            .map_err(|e| super::io_error(e, "open_failed"))?;
        let meta = file
            .metadata()
            .map_err(|e| super::io_error(e, "internal_error"))?;
        if r.path == "/api/files-content" {
            let mut data = Vec::new();
            file.take((CONTENT_MAX + 1) as u64)
                .read_to_end(&mut data)
                .map_err(|e| super::io_error(e, "read_failed"))?;
            let truncated = data.len() > CONTENT_MAX;
            data.truncate(CONTENT_MAX);
            return Ok(Response::json(
                200,
                &json!({"path":path,"size":meta.len(),"mtime":time::format(meta.modified().map_err(|e|super::io_error(e,"internal_error"))?)?,"content":go_lossy(&data),"truncated":truncated,"readOnly":grant.read_only}),
            ));
        }
        serve_file(r, path, file, meta.len())
    }
}
fn list(r: &Request, s: &Scope, remote: bool) -> Result<Response> {
    let raw = r.query("root");
    let root = if raw.is_empty() {
        s.cwd.join("docs").join("local")
    } else {
        let p = Path::new(&raw);
        if !p.is_absolute() {
            return Err(err(400, "bad_request", "root must be an absolute path"));
        }
        if remote
            && !scope::under_roots(
                &scope::canonical(p).unwrap_or_else(|_| scope::clean(p)),
                &[
                    s.cwd.clone(),
                    s.git_root.clone(),
                    s.paths
                        .resource(crate::config::paths::Resource::Orchestration),
                ],
            )
        {
            return Err(err(403, "forbidden", "path is outside allowed roots"));
        }
        p.to_path_buf()
    };
    let canonical_root = scope::canonical(&root).unwrap_or_else(|_| scope::clean(&root));
    if remote
        && !scope::under_roots(
            &canonical_root,
            &[
                s.cwd.clone(),
                s.git_root.clone(),
                s.paths
                    .resource(crate::config::paths::Resource::Orchestration),
            ],
        )
    {
        return Err(err(403, "forbidden", "path is outside allowed roots"));
    }
    let cap = super::safe_fs::Dir::open(&canonical_root);
    let exists = cap.is_ok();
    let mut items = Vec::new();
    let mut truncated = false;
    if let Ok(cap) = cap {
        walk(&root, &cap, &s.cwd, s, 1, &mut items, &mut truncated);
    }
    items.sort_by(|a, b| b.0.cmp(&a.0));
    let items: Vec<Value> = items.into_iter().map(|(_, v)| v).collect();
    Ok(Response::json(
        200,
        &json!({"root":root,"exists":exists,"truncated":truncated,"items":items}),
    ))
}
fn walk(
    dir: &Path,
    cap: &super::safe_fs::Dir,
    cwd: &Path,
    s: &Scope,
    depth: usize,
    out: &mut Vec<(std::time::SystemTime, Value)>,
    truncated: &mut bool,
) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = cap.entries() else {
        return;
    };
    for name in entries {
        let full = dir.join(&name);
        let child = cap.child_dir(&name, false).ok();
        let info = match &child {
            Some(d) => d.own_metadata(),
            None => cap.metadata(&name),
        };
        let Ok(info) = info else {
            continue;
        };
        let is_dir = child.is_some();
        if is_dir
            && matches!(
                name.as_str(),
                "node_modules" | "vendor" | "target" | "dist" | "build" | "out" | "__pycache__"
            )
        {
            continue;
        }
        if out.len() >= 2000 {
            *truncated = true;
            return;
        }
        let summary = if !is_dir
            && text_file(&full)
            && !scope::secret_denied(&full, &s.paths)
            && !scope::secret_denied(&cap.path().join(&name), &s.paths)
        {
            cap.read(&name, 32 * 1024)
                .ok()
                .map(|b| summary(&String::from_utf8_lossy(&b)))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let Ok(mtime) = info.modified() else {
            continue;
        };
        let Ok(mtime_text) = time::format(mtime) else {
            continue;
        };
        out.push((mtime,json!({"path":full,"rel":scope::relative(&full,cwd),"name":name,"type":if is_dir{"dir"}else{"file"},"size":info.len(),"mtime":mtime_text,"summary":summary})));
        if is_dir
            && !matches!(name.as_str(), ".git" | ".hg" | ".svn")
            && let Some(child) = child
        {
            walk(&full, &child, cwd, s, depth + 1, out, truncated);
        }
    }
}
pub fn summary(content: &str) -> String {
    let normalize = |v: &str| {
        let v = v.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut out: String = v.chars().take(200).collect();
        if v.chars().count() > 200 {
            out.push('…');
        }
        out
    };
    let lower = content.to_lowercase();
    if let Some(start) = lower.find("<!--")
        && let Some(end) = lower[start + 4..].find("-->")
    {
        let text = content[start + 4..start + 4 + end].trim();
        if text
            .get(..8)
            .is_some_and(|s| s.eq_ignore_ascii_case("summary:"))
        {
            return normalize(text[8..].trim());
        }
    }
    let lines: Vec<_> = content.lines().collect();
    let mut start = 0;
    if content.starts_with("---")
        && let Some(end) = lines.iter().skip(1).position(|l| l.trim_end() == "---")
    {
        start = end + 2;
        for line in &lines[1..start - 1] {
            for key in ["description:", "summary:", "subtitle:"] {
                if line.to_lowercase().starts_with(key) {
                    let v = line[key.len()..].trim().trim_matches(['\'', '"']);
                    if !v.is_empty() {
                        return normalize(v);
                    }
                }
            }
        }
    }
    if let Some(h) = lines
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, l)| l.starts_with("# "))
    {
        start = h.0 + 1;
    }
    let mut para = Vec::new();
    let mut fence: Option<&str> = None;
    for line in &lines[start..] {
        if let Some(f) = fence {
            if line.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if line.starts_with("```") || line.starts_with("~~~") {
            if !para.is_empty() {
                break;
            }
            fence = Some(&line[..3]);
            continue;
        }
        if line.trim().is_empty() || line.starts_with(['#', '>']) {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        para.push(*line);
    }
    normalize(&para.join(" "))
}
pub fn disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() && !matches!(c, '"' | '\\' | '/' | ';') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let fallback = fallback.trim();
    let fallback = if matches!(fallback, "" | "." | "..") {
        "download"
    } else {
        fallback
    };
    let mut encoded = String::new();
    for b in name.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~$&+:=@".contains(b) {
            encoded.push(*b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}
pub(super) fn serve_file(r: &Request, path: &Path, file: File, size: u64) -> Result<Response> {
    use crate::hub::http::{StreamedFile, StreamedRange};
    let modified = file
        .metadata()
        .and_then(|m| m.modified())
        .map_err(|e| super::io_error(e, "stat_failed"))?;
    let modified_seconds = truncate_seconds(modified);
    let last_modified = http_date(modified)?;
    let if_match = r.header("if-match");
    let if_none_match = r.header("if-none-match");
    let precondition = if (!if_match.is_empty() && if_match.trim() != "*")
        || (if_match.is_empty()
            && parse_http_date(r.header("if-unmodified-since"))
                .is_some_and(|date| modified_seconds > date))
    {
        Some(412)
    } else if if_none_match.split(',').any(|tag| tag.trim() == "*")
        || (if_none_match.is_empty()
            && parse_http_date(r.header("if-modified-since"))
                .is_some_and(|date| modified_seconds <= date))
    {
        Some(304)
    } else {
        None
    };
    if let Some(status) = precondition {
        let mut response = Response::bytes(status, "", Vec::new());
        response.headers.remove("Content-Type");
        response
            .headers
            .insert("Last-Modified".into(), last_modified);
        return Ok(response);
    }
    let if_range = r.header("if-range");
    let range = if !if_range.is_empty() && parse_http_date(if_range) != Some(modified_seconds) {
        ""
    } else {
        r.header("range")
    };
    let ranges = if range.is_empty() {
        Vec::new()
    } else {
        match parse_ranges(range, size) {
            Ok(ranges) => ranges,
            Err(message) => {
                let mut response =
                    Response::bytes(416, "text/plain; charset=utf-8", format!("{message}\n"));
                if message == "invalid range: failed to overlap" {
                    response
                        .headers
                        .insert("Content-Range".into(), format!("bytes */{size}"));
                }
                return Ok(response);
            }
        }
    };
    let ranges = if ranges.iter().map(|(_, len)| *len as u128).sum::<u128>() > size as u128 {
        Vec::new()
    } else {
        ranges
    };
    let mut status = 200;
    let mut content_type = mime(path).to_owned();
    let mut total = size;
    let mut suffix = Vec::new();
    let mut streams = Vec::new();
    let mut content_range = None;
    match ranges.len() {
        0 => streams.push(StreamedRange {
            prefix: vec![],
            offset: 0,
            len: size,
        }),
        1 => {
            status = 206;
            let (start, len) = ranges[0];
            total = len;
            content_range = Some(format!(
                "bytes {start}-{}/{size}",
                start as i128 + len as i128 - 1
            ));
            streams.push(StreamedRange {
                prefix: vec![],
                offset: start,
                len,
            });
        }
        _ => {
            status = 206;
            let boundary =
                crate::process::random_token().map_err(|e| super::io_error(e, "read_failed"))?;
            content_type = format!("multipart/byteranges; boundary={boundary}");
            total = 0;
            for (i, (start, len)) in ranges.iter().enumerate() {
                let prefix=format!("{}--{boundary}\r\nContent-Range: bytes {start}-{}/{size}\r\nContent-Type: {}\r\n\r\n",if i==0{""}else{"\r\n"},*start as i128+*len as i128-1,mime(path)).into_bytes();
                total += prefix.len() as u64 + len;
                streams.push(StreamedRange {
                    prefix,
                    offset: *start,
                    len: *len,
                });
            }
            suffix = format!("\r\n--{boundary}--\r\n").into_bytes();
            total += suffix.len() as u64;
        }
    }
    let mut response = Response::bytes(status, &content_type, Vec::new());
    response.file = Some(Box::new(StreamedFile {
        file,
        ranges: streams,
        suffix,
    }));
    response
        .headers
        .insert("Last-Modified".into(), last_modified);
    response
        .headers
        .insert("Accept-Ranges".into(), "bytes".into());
    response
        .headers
        .insert("Content-Length".into(), total.to_string());
    if let Some(range) = content_range {
        response.headers.insert("Content-Range".into(), range);
    }
    if r.path == "/api/files-download" {
        response.headers.insert(
            "Content-Disposition".into(),
            disposition(&path.file_name().unwrap_or_default().to_string_lossy()),
        );
    }
    Ok(response)
}
fn parse_ranges(header: &str, size: u64) -> std::result::Result<Vec<(u64, u64)>, &'static str> {
    let rest = header.strip_prefix("bytes=").ok_or("invalid range")?;
    let mut ranges = Vec::new();
    let mut no_overlap = false;
    for part in rest.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (a, b) = part.split_once('-').ok_or("invalid range")?;
        let a = a.trim();
        let b = b.trim();
        let (start, len) = if a.is_empty() {
            let n = b.parse::<u64>().map_err(|_| "invalid range")?.min(size);
            (size - n, n)
        } else {
            let start = a.parse::<u64>().map_err(|_| "invalid range")?;
            if start >= size {
                no_overlap = true;
                continue;
            }
            let end = if b.is_empty() {
                size - 1
            } else {
                b.parse::<u64>().map_err(|_| "invalid range")?.min(size - 1)
            };
            if end < start {
                return Err("invalid range");
            }
            (start, end - start + 1)
        };
        ranges.push((start, len));
    }
    if no_overlap && ranges.is_empty() {
        if size == 0 {
            return Ok(Vec::new());
        }
        return Err("invalid range: failed to overlap");
    }
    Ok(ranges)
}

fn truncate_seconds(time: std::time::SystemTime) -> std::time::SystemTime {
    use std::time::{Duration, UNIX_EPOCH};
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => UNIX_EPOCH + Duration::from_secs(d.as_secs()),
        Err(e) => {
            UNIX_EPOCH
                - Duration::from_secs(
                    e.duration().as_secs() + u64::from(e.duration().subsec_nanos() != 0),
                )
        }
    }
}
fn http_date(value: std::time::SystemTime) -> Result<String> {
    time::format_utc(value)?;
    let date: chrono::DateTime<chrono::Utc> = value.into();
    Ok(date.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
}
fn parse_http_date(value: &str) -> Option<std::time::SystemTime> {
    if value.is_empty() {
        return None;
    }
    httpdate::parse_http_date(value).ok().or_else(|| {
        chrono::NaiveDateTime::parse_from_str(value, "%a, %d %b %Y %H:%M:%S GMT")
            .ok()
            .map(|d| d.and_utc().into())
    })
}

fn go_lossy(mut bytes: &[u8]) -> String {
    let mut out = String::new();
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(s) => {
                out.push_str(s);
                break;
            }
            Err(e) => {
                out.push_str(std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap());
                out.push('\u{fffd}');
                bytes = &bytes[e.valid_up_to() + 1..];
            }
        }
    }
    out
}
