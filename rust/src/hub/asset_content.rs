//! Embedded zero-modtime FileServer/ServeContent behavior from pinned Go 1.26.8.
//! Assets have no ETag or modification time. HTTP API guards belong to callers.
use crate::hub::http::{Request, Response};
use std::collections::BTreeMap;

fn response(status: u16) -> Response {
    Response {
        status,
        headers: BTreeMap::new(),
        cookies: vec![],
        body: vec![],
        file: None,
    }
}
fn error(status: u16, message: &str) -> Response {
    let mut result = response(status);
    result
        .headers
        .insert("Content-Type".into(), "text/plain; charset=utf-8".into());
    result
        .headers
        .insert("X-Content-Type-Options".into(), "nosniff".into());
    result.body = format!("{message}\n").into_bytes();
    result
}
fn trim(value: &str) -> &str {
    // net/textproto.TrimString's ASCII set is exactly SP, HTAB, LF, CR.
    // Vertical tab, form feed, and Unicode whitespace remain invalid syntax.
    value.trim_matches([' ', '\t', '\n', '\r'])
}

fn scan_etag(value: &str) -> Option<&str> {
    let value = trim(value);
    let start = usize::from(value.starts_with("W/")) * 2;
    let bytes = value.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    for (i, &byte) in bytes.iter().enumerate().skip(start + 1) {
        if byte == b'"' {
            return Some(&value[i + 1..]);
        }
        if !(byte == 0x21 || (0x23..=0x7e).contains(&byte) || byte >= 0x80) {
            return None;
        }
    }
    None
}
// No response ETag exists; only a syntactically reached wildcard can match.
fn wildcard(mut value: &str) -> bool {
    loop {
        value = trim(value);
        if let Some(rest) = value.strip_prefix(',') {
            value = rest;
            continue;
        }
        if value.starts_with('*') {
            return true;
        }
        let Some(rest) = scan_etag(value) else {
            return false;
        };
        value = rest;
    }
}

#[derive(Clone, Copy)]
struct Range {
    start: usize,
    len: usize,
}
impl Range {
    fn header(self, size: usize) -> String {
        // Suffix -0 produces the source's zero-length range at EOF.
        format!(
            "bytes {}-{}/{size}",
            self.start,
            self.start as i64 + self.len as i64 - 1
        )
    }
}
fn integer(value: &str) -> Result<i64, &'static str> {
    // strconv.ParseInt allows one leading sign but rejects underscores/space.
    value.parse::<i64>().map_err(|_| "invalid range")
}
fn parse_ranges(value: &str, size: usize) -> Result<Vec<Range>, &'static str> {
    if value.is_empty() {
        return Ok(vec![]);
    }
    let Some(value) = value.strip_prefix("bytes=") else {
        return Err("invalid range");
    };
    let mut ranges = vec![];
    let mut no_overlap = false;
    for part in value.split(',').map(trim).filter(|part| !part.is_empty()) {
        let Some((start, end)) = part.split_once('-') else {
            return Err("invalid range");
        };
        let (start, end) = (trim(start), trim(end));
        let range = if start.is_empty() {
            if end.is_empty() || end.starts_with('-') {
                return Err("invalid range");
            }
            let suffix = integer(end)?;
            if suffix < 0 {
                return Err("invalid range");
            }
            let len = (suffix as u64).min(size as u64) as usize;
            Range {
                start: size - len,
                len,
            }
        } else {
            let start = integer(start)?;
            if start < 0 {
                return Err("invalid range");
            }
            if start as u64 >= size as u64 {
                no_overlap = true;
                continue;
            }
            let start = start as usize;
            let len = if end.is_empty() {
                size - start
            } else {
                let end = integer(end)?;
                if end < start as i64 {
                    return Err("invalid range");
                }
                (end as u64).min(size as u64 - 1) as usize - start + 1
            };
            Range { start, len }
        };
        ranges.push(range);
    }
    if no_overlap && ranges.is_empty() {
        return Err("invalid range: failed to overlap");
    }
    Ok(ranges)
}

pub(super) fn serve_content(request: &Request, mime: &str, bytes: &[u8]) -> Response {
    let if_match = request.header("If-Match");
    if !if_match.is_empty() && !wildcard(if_match) {
        return response(412);
    }
    if wildcard(request.header("If-None-Match")) {
        return response(if matches!(request.method.as_str(), "GET" | "HEAD") {
            304
        } else {
            412
        });
    }
    // Date preconditions do nothing for embedded files' unknown modification time.
    let mut range_header = request.header("Range");
    if matches!(request.method.as_str(), "GET" | "HEAD") && !request.header("If-Range").is_empty() {
        range_header = "";
    }
    let size = bytes.len();
    let mut ranges = match parse_ranges(range_header, size) {
        Ok(ranges) => ranges,
        Err("invalid range: failed to overlap") if size == 0 => vec![],
        Err(message) => {
            let mut result = error(416, message);
            if message == "invalid range: failed to overlap" {
                result
                    .headers
                    .insert("Content-Range".into(), format!("bytes */{size}"));
            }
            return result;
        }
    };
    if ranges.iter().map(|range| range.len as u128).sum::<u128>() > size as u128 {
        ranges.clear();
    }
    let mut result = response(if ranges.is_empty() { 200 } else { 206 });
    result.headers.insert("Content-Type".into(), mime.into());
    let body = if ranges.len() == 1 {
        let range = ranges[0];
        result
            .headers
            .insert("Content-Range".into(), range.header(size));
        bytes[range.start..range.start + range.len].to_vec()
    } else if ranges.len() > 1 {
        // Go uses 30 cryptographically random bytes as 60 lowercase hex digits.
        let boundary = match crate::hub::http::random_hex(30) {
            Ok(value) => value,
            Err(_) => return error(500, "multipart boundary unavailable"),
        };
        result.headers.insert(
            "Content-Type".into(),
            format!("multipart/byteranges; boundary={boundary}"),
        );
        let mut body = vec![];
        for (i, range) in ranges.iter().enumerate() {
            if i != 0 {
                body.extend_from_slice(b"\r\n");
            }
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Range: {}\r\nContent-Type: {mime}\r\n\r\n",
                    range.header(size)
                )
                .as_bytes(),
            );
            body.extend_from_slice(&bytes[range.start..range.start + range.len]);
        }
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    } else {
        bytes.to_vec()
    };
    result
        .headers
        .insert("Accept-Ranges".into(), "bytes".into());
    result
        .headers
        .insert("Content-Length".into(), body.len().to_string());
    if request.method != "HEAD" {
        result.body = body;
    }
    result
}

fn clean_path(value: &str) -> String {
    let mut parts = vec![];
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}
fn redirect(request: &Request, destination: &str) -> Response {
    let mut result = response(301);
    result.headers.insert(
        "Location".into(),
        if request.query.is_empty() {
            destination.into()
        } else {
            format!("{destination}?{}", request.query)
        },
    );
    result
}
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
        .replace('\'', "&#39;")
}
fn escape_path(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~/:@&=+$".contains(&byte) {
            result.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(result, "%{byte:02X}").expect("String write");
        }
    }
    result
}

/// FileServer path normalization, directory redirects/index/listing over the
/// build-time embedded table, with no ambient filesystem access.
pub(super) fn file_server(request: &Request, assets: &[(&str, &[u8])]) -> Response {
    if request.path.ends_with("/index.html") {
        return redirect(request, "./");
    }
    let path = clean_path(&request.path);
    let name = path.trim_start_matches('/');
    let file = assets.iter().find(|(entry, _)| *entry == name);
    let prefix = if name.is_empty() {
        String::new()
    } else {
        format!("{name}/")
    };
    let directory = name.is_empty() || assets.iter().any(|(entry, _)| entry.starts_with(&prefix));
    if let Some((_, bytes)) = file {
        if request.path.ends_with('/') {
            return redirect(
                request,
                &format!("../{}", name.rsplit('/').next().unwrap_or(name)),
            );
        }
        return serve_content(request, super::mime(name), bytes);
    }
    if !directory {
        return error(404, "404 page not found");
    }
    if !request.path.ends_with('/') {
        return redirect(
            request,
            &format!("{}/", name.rsplit('/').next().unwrap_or(name)),
        );
    }
    if let Some((_, bytes)) = assets
        .iter()
        .find(|(entry, _)| *entry == format!("{prefix}index.html"))
    {
        return serve_content(request, "text/html; charset=utf-8", bytes);
    }
    let mut entries = BTreeMap::new();
    for (entry, _) in assets {
        if let Some(relative) = entry.strip_prefix(&prefix) {
            if let Some((first, _)) = relative.split_once('/') {
                entries.insert(first, true);
            } else if !relative.is_empty() {
                entries.insert(relative, false);
            }
        }
    }
    let mut body =
        "<!doctype html>\n<meta name=\"viewport\" content=\"width=device-width\">\n<pre>\n"
            .to_owned();
    for (name, directory) in entries {
        let name = format!("{name}{}", if directory { "/" } else { "" });
        body.push_str(&format!(
            "<a href=\"{}\">{}</a>\n",
            escape_path(&name),
            escape_html(&name)
        ));
    }
    body.push_str("</pre>\n");
    let mut result = response(200);
    result
        .headers
        .insert("Content-Type".into(), "text/html; charset=utf-8".into());
    // Go FileServer writes listing/error bodies even to a HEAD recorder; the
    // real HTTP transport suppresses HEAD bodies independently.
    result.body = body.into_bytes();
    result
}

#[cfg(test)]
#[path = "asset_content/tests.rs"]
mod tests;
