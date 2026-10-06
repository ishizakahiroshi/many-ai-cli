//! Live ./web/dist owner selected only by serve -dev. Embedded assets remain immutable.
use crate::hub::{
    assets::{AssetSource, file_table_response},
    http::{Request, Response},
};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
pub struct DevAssets {
    root: PathBuf,
    confined: bool,
    pinned: Option<crate::files::safe_fs::Dir>,
}
type PinnedResponseData = (
    std::fs::Metadata,
    Vec<(String, Vec<u8>)>,
    Option<SystemTime>,
);
impl DevAssets {
    pub fn new(cwd: &Path, confined: bool) -> Self {
        Self {
            root: cwd.join("web/dist"),
            confined,
            pinned: confined
                .then(|| crate::files::safe_fs::Dir::open(&cwd.join("web/dist")).ok())
                .flatten(),
        }
    }
    fn path(&self, name: &str) -> io::Result<PathBuf> {
        // Go http.Dir localizes the cleaned slash path before joining its root.
        // Windows separators, volume names and alternate streams are not URL
        // components; Unix keeps these characters as ordinary filename bytes.
        #[cfg(windows)]
        if name.contains(['\\', ':', '\0']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid or unsafe asset path",
            ));
        }
        let path = self.root.join(name);
        Ok(path)
    }
    fn pinned_directory(&self, names: &[&str]) -> io::Result<crate::files::safe_fs::Dir> {
        let root = self.pinned.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trial developer assets could not be pinned",
            )
        })?;
        let (first, remaining) = names.split_first().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "missing directory component")
        })?;
        let mut directory = root.child_dir(first, false)?;
        for name in remaining {
            directory = directory.child_dir(name, false)?;
        }
        Ok(directory)
    }
    fn pinned_file(&self, name: &str) -> io::Result<std::fs::File> {
        let names: Vec<_> = name.split('/').filter(|name| !name.is_empty()).collect();
        for component in &names {
            crate::files::safe_fs::basename(component)?;
        }
        let (leaf, parents) = names
            .split_last()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing asset name"))?;
        // The selected root stays pinned; every descendant is opened no-follow.
        let root = self.pinned.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trial developer assets could not be pinned",
            )
        })?;
        if parents.is_empty() {
            return root.open_file(leaf, false);
        }
        self.pinned_directory(parents)?.open_file(leaf, false)
    }
    fn pinned_response_data(&self, name: &str) -> io::Result<PinnedResponseData> {
        if let Ok(mut file) = self.pinned_file(name) {
            let metadata = file.metadata()?;
            let modified = metadata.modified().ok();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            return Ok((metadata, vec![(name.into(), bytes)], modified));
        }
        let names: Vec<_> = name.split('/').filter(|name| !name.is_empty()).collect();
        let root = self.pinned.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trial developer assets could not be pinned",
            )
        })?;
        let child;
        let dir = if names.is_empty() {
            root
        } else {
            child = self.pinned_directory(&names)?;
            &child
        };
        let metadata = dir.own_metadata()?;
        let mut files = Vec::new();
        let mut modified = None;
        for entry in dir.entries()? {
            let relative = if name.is_empty() {
                entry.clone()
            } else {
                format!("{name}/{entry}")
            };
            if dir.child_dir(&entry, false).is_ok() {
                files.push((format!("{relative}/.directory-sentinel"), Vec::new()));
            } else if let Ok(mut file) = dir.open_file(&entry, false) {
                let mut bytes = Vec::new();
                if entry == "index.html" {
                    modified = file.metadata()?.modified().ok();
                    file.read_to_end(&mut bytes)?;
                }
                files.push((relative, bytes));
            }
        }
        Ok((metadata, files, modified))
    }
    fn response(&self, request: &Request) -> io::Result<Response> {
        if request.path.ends_with("/index.html") {
            return Ok(file_table_response(request, &[]));
        }
        let mut pieces = Vec::new();
        for piece in request.path.split('/') {
            match piece {
                "" | "." => {}
                ".." => {
                    pieces.pop();
                }
                _ => pieces.push(piece),
            }
        }
        let name = pieces.join("/");
        let path = self.path(&name)?;
        let (metadata, mut files, modified) = if self.confined {
            self.pinned_response_data(&name)?
        } else {
            let metadata = std::fs::metadata(&path)?;
            let mut files: Vec<(String, Vec<u8>)> = Vec::new();
            let modified = if metadata.is_file() {
                files.push((name.clone(), std::fs::read(&path)?));
                Some(metadata.modified()?)
            } else {
                for entry in std::fs::read_dir(&path)? {
                    let entry = entry?;
                    let entry_name = entry.file_name().to_string_lossy().into_owned();
                    let relative = if name.is_empty() {
                        entry_name.clone()
                    } else {
                        format!("{name}/{entry_name}")
                    };
                    let child = self.path(&relative)?;
                    if entry.metadata()?.is_dir() {
                        files.push((format!("{relative}/.directory-sentinel"), Vec::new()));
                    } else if entry_name == "index.html" {
                        files.push((relative, std::fs::read(child)?));
                    } else {
                        files.push((relative, Vec::new()));
                    }
                }
                let index = path.join("index.html");
                std::fs::metadata(index)
                    .ok()
                    .and_then(|m| m.modified().ok())
            };
            (metadata, files, modified)
        };
        if metadata.is_dir() && files.is_empty() {
            if request.path.ends_with('/') {
                let mut response=Response::bytes(200,"text/html; charset=utf-8",b"<!doctype html>\n<meta name=\"viewport\" content=\"width=device-width\">\n<pre>\n</pre>\n".to_vec());
                response.security_headers();
                return Ok(response);
            }
            files.push((format!("{name}/.directory-sentinel"), Vec::new()));
        }
        let table: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        if (metadata.is_file() && request.path.ends_with('/'))
            || (metadata.is_dir() && !request.path.ends_with('/'))
        {
            return Ok(file_table_response(request, &table));
        }
        if !request.header("If-Match").is_empty() && request.header("If-Match").trim() != "*" {
            return Ok(file_table_response(request, &table));
        }
        let mut request = request.clone();
        if let Some(modified) = modified {
            let seconds = modified
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let before = |name: &str| date(request.header(name)).is_some_and(|date| seconds > date);
            if request.header("If-Match").is_empty() && before("If-Unmodified-Since") {
                return Ok(Response::bytes(412, "", Vec::new()));
            }
            if request.header("If-None-Match").is_empty()
                && matches!(request.method.as_str(), "GET" | "HEAD")
                && date(request.header("If-Modified-Since")).is_some_and(|date| seconds <= date)
            {
                let mut response = Response::bytes(304, "", Vec::new());
                response.headers.clear();
                response
                    .headers
                    .insert("Last-Modified".into(), http_date(modified));
                response.security_headers();
                return Ok(response);
            }
            if date(request.header("If-Range")).is_some_and(|date| seconds == date) {
                request
                    .headers
                    .retain(|(name, _)| !name.eq_ignore_ascii_case("If-Range"));
            }
        }
        let mut response = file_table_response(&request, &table);
        if matches!(response.status, 200 | 206 | 304)
            && let Some(modified) = modified
        {
            response
                .headers
                .insert("Last-Modified".into(), http_date(modified));
        }
        Ok(response)
    }
}
fn date(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|time| time.timestamp())
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%A, %d-%b-%y %H:%M:%S GMT")
                .ok()
                .map(|time| time.and_utc().timestamp())
        })
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%a %b %e %H:%M:%S %Y")
                .ok()
                .map(|time| time.and_utc().timestamp())
        })
}
fn http_date(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time)
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}
impl AssetSource for DevAssets {
    fn index_bytes(&self) -> io::Result<Vec<u8>> {
        if self.confined {
            let mut bytes = Vec::new();
            self.pinned_file("index.html")?.read_to_end(&mut bytes)?;
            return Ok(bytes);
        }
        std::fs::read(self.path("index.html")?)
    }
    fn static_response(&self, request: &Request) -> Response {
        match self.response(request) {
            Ok(response) => response,
            Err(error) => {
                let (status, text) = match error.kind() {
                    io::ErrorKind::NotFound => (404, "404 page not found\n"),
                    io::ErrorKind::PermissionDenied => (403, "403 Forbidden\n"),
                    _ => (500, "500 Internal Server Error\n"),
                };
                let mut response = Response::bytes(
                    status,
                    "text/plain; charset=utf-8",
                    text.as_bytes().to_vec(),
                );
                response.security_headers();
                response
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trial_root_must_be_pinned_before_serving_and_rejects_component_aliases() {
        let root = tempfile::tempdir().unwrap();
        let owner = DevAssets::new(root.path(), true);
        std::fs::create_dir_all(root.path().join("web/dist")).unwrap();
        std::fs::write(root.path().join("web/dist/index.html"), b"late file").unwrap();
        assert!(owner.index_bytes().is_err());
        let owner = DevAssets::new(root.path(), true);
        assert!(owner.pinned_file("../dist/index.html").is_err());
        assert!(owner.pinned_file("index.html:stream").is_err());
        assert_eq!(owner.index_bytes().unwrap(), b"late file");
    }
    #[cfg(unix)]
    #[test]
    fn trial_keeps_selected_inode_after_path_replacement_and_refuses_symlink_leaf() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("web/dist")).unwrap();
        std::fs::write(root.path().join("web/dist/index.html"), b"selected").unwrap();
        let owner = DevAssets::new(root.path(), true);
        std::fs::rename(
            root.path().join("web/dist"),
            root.path().join("web/selected"),
        )
        .unwrap();
        std::fs::create_dir(root.path().join("replacement")).unwrap();
        std::fs::write(root.path().join("replacement/index.html"), b"outside").unwrap();
        symlink(
            root.path().join("replacement"),
            root.path().join("web/dist"),
        )
        .unwrap();
        assert_eq!(owner.index_bytes().unwrap(), b"selected");
        symlink(
            root.path().join("replacement/index.html"),
            root.path().join("web/selected/link.js"),
        )
        .unwrap();
        assert!(owner.pinned_file("link.js").is_err());
    }
    #[test]
    fn live_assets_reload_and_native_modified_date_preserves_ranges() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("web/dist")).unwrap();
        let path = root.path().join("web/dist/app.js");
        std::fs::write(&path, b"first").unwrap();
        let owner = DevAssets::new(root.path(), true);
        let mut request = Request {
            method: "GET".into(),
            path: "/app.js".into(),
            ..Default::default()
        };
        let first = owner.static_response(&request);
        assert_eq!(first.body, b"first");
        let modified = first.headers["Last-Modified"].clone();
        request
            .headers
            .push(("If-Modified-Since".into(), modified.clone()));
        assert_eq!(owner.static_response(&request).status, 304);
        request.headers = vec![
            ("If-Range".into(), modified),
            ("Range".into(), "bytes=1-2".into()),
        ];
        let range = owner.static_response(&request);
        assert_eq!(range.status, 206);
        assert_eq!(range.body, b"ir");
        std::fs::write(&path, b"second").unwrap();
        request.headers.clear();
        assert_eq!(owner.static_response(&request).body, b"second");
    }
}

#[cfg(test)]
mod native_path_tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn unconfined_windows_assets_reject_backslash_and_colon_components() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("web/dist/nested")).unwrap();
        std::fs::write(
            root.path().join("web/outside.js"),
            b"outside synthetic asset",
        )
        .unwrap();
        std::fs::write(
            root.path().join("web/dist/app.js"),
            b"inside synthetic asset",
        )
        .unwrap();
        let owner = DevAssets::new(root.path(), false);
        // Abstract drive/root forms are validation-only: even a failing old
        // implementation must never open a path outside the synthetic fixture.
        for name in [
            r"\outside.js",
            r"C:\outside.js",
            r"C:outside.js",
            r"nested/C:outside.js",
        ] {
            assert_eq!(
                owner.path(name).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{name}"
            );
        }
        for path in [
            r"/..\outside.js",
            r"/nested/..\..\outside.js",
            "/app.js:synthetic-stream",
        ] {
            let request = Request {
                method: "GET".into(),
                path: path.into(),
                ..Default::default()
            };
            assert_eq!(
                owner.response(&request).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{path}"
            );
            let response = owner.static_response(&request);
            assert_eq!(response.status, 500, "{path}");
            assert!(
                !String::from_utf8_lossy(&response.body).contains("synthetic asset"),
                "{path}"
            );
        }
        let ordinary = Request {
            method: "GET".into(),
            path: "/app.js".into(),
            ..Default::default()
        };
        assert_eq!(
            owner.static_response(&ordinary).body,
            b"inside synthetic asset"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unconfined_unix_assets_keep_native_colon_and_backslash_filenames() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("web/dist")).unwrap();
        let owner = DevAssets::new(root.path(), false);
        for name in ["app:module.js", r"app\module.js"] {
            std::fs::write(
                root.path().join("web/dist").join(name),
                b"native synthetic asset",
            )
            .unwrap();
            let request = Request {
                method: "GET".into(),
                path: format!("/{name}"),
                ..Default::default()
            };
            let response = owner.static_response(&request);
            assert_eq!(response.status, 200, "{name}");
            assert_eq!(response.body, b"native synthetic asset", "{name}");
        }
    }
}
