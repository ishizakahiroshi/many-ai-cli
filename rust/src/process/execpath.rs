//! Shared executable lookup and Windows npm-shim resolution.
//!
//! Sources: internal/execpath/{execpath_other,execpath_windows}.go and wrapper
//! resolveCmd at oracle 21d0bc7. The supplied environment and filesystem are the
//! complete lookup context; this code never reads or changes the process env.
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Unix,
    Windows,
}
impl Platform {
    pub fn native() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// Tests may use an entirely synthetic filesystem, including Windows paths on
/// Unix. Lookup uses is_executable; shim recovery intentionally uses exists,
/// matching the Go os.Stat contract (not a stricter new-record validator).
pub trait ExecutableFs: Send + Sync {
    fn exists(&self, path: &str) -> bool;
    fn is_executable(&self, path: &str, platform: Platform) -> bool;
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;
    fn same_file(&self, first: &str, second: &str) -> bool {
        first == second
    }
}
pub struct NativeFs;
impl ExecutableFs for NativeFs {
    fn exists(&self, path: &str) -> bool {
        std::fs::metadata(path).is_ok()
    }
    fn is_executable(&self, path: &str, platform: Platform) -> bool {
        let Ok(metadata) = std::fs::metadata(path) else {
            return false;
        };
        if metadata.is_dir() {
            return false;
        }
        if platform == Platform::Windows {
            return true;
        }
        #[cfg(unix)]
        {
            use std::{ffi::CString, os::unix::fs::PermissionsExt};
            let Ok(path) = CString::new(path) else {
                return false;
            };
            // SAFETY: the NUL-terminated path and access flags are valid. This
            // checks effective-identity execute access without executing a file.
            if unsafe {
                libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::X_OK, libc::AT_EACCESS)
            } == 0
            {
                return true;
            }
            let error = io::Error::last_os_error().raw_os_error();
            matches!(error, Some(libc::ENOSYS) | Some(libc::EPERM))
                && metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    }
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }
    fn same_file(&self, first: &str, second: &str) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            match (
                std::fs::symlink_metadata(first),
                std::fs::symlink_metadata(second),
            ) {
                (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
                _ => false,
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
            };
            fn identity(path: &str) -> Option<(u32, u32, u32)> {
                let file = std::fs::File::open(path).ok()?;
                let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
                // SAFETY: the File owns a live handle and information is writable.
                if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) }
                    == 0
                {
                    return None;
                }
                Some((
                    information.dwVolumeSerialNumber,
                    information.nFileIndexHigh,
                    information.nFileIndexLow,
                ))
            }
            matches!((identity(first),identity(second)),(Some(a),Some(b)) if a==b)
        }
        #[cfg(not(any(unix, windows)))]
        {
            first == second
        }
    }
}

/// No Debug implementation: custom arguments can contain user credentials.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub executable: String,
    pub args: Vec<String>,
    /// Actual cmd.exe /c shim fallback, for safe launch-prompt transport.
    pub shell_shim: Option<String>,
}
pub struct Resolver<'a> {
    platform: Platform,
    environment: &'a [String],
    cwd: &'a Path,
    fs: &'a dyn ExecutableFs,
}
impl<'a> Resolver<'a> {
    pub fn new(
        platform: Platform,
        environment: &'a [String],
        cwd: &'a Path,
        fs: &'a dyn ExecutableFs,
    ) -> Self {
        Self {
            platform,
            environment,
            cwd,
            fs,
        }
    }
    fn env(&self, key: &str) -> Option<&str> {
        self.environment.iter().rev().find_map(|kv| {
            // Windows exports drive-current-directory entries as =C:=C:\dir.
            let offset = usize::from(kv.starts_with('='));
            let index = kv[offset..].find('=')? + offset;
            let (name, value) = (&kv[..index], &kv[index + 1..]);
            ((self.platform == Platform::Windows && name.eq_ignore_ascii_case(key)) || name == key)
                .then_some(value)
        })
    }
    fn absolute(&self, path: &str) -> String {
        if is_absolute(path, self.platform) {
            clean(path, self.platform)
        } else if self.platform == Platform::Windows {
            let cwd = self.cwd.to_string_lossy();
            let path = path.replace('/', "\\");
            if path.as_bytes().get(1) == Some(&b':') {
                let drive = &path[..2];
                let rest = &path[2..];
                let base = if cwd.get(..2).is_some_and(|v| v.eq_ignore_ascii_case(drive)) {
                    cwd.into_owned()
                } else {
                    self.env(&format!("={drive}"))
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{drive}\\"))
                };
                join(&base, rest, self.platform)
            } else if path.starts_with('\\') {
                let volume = windows_volume(&cwd);
                clean(&format!("{volume}{path}"), self.platform)
            } else {
                join(&cwd, &path, self.platform)
            }
        } else {
            join(&self.cwd.to_string_lossy(), path, self.platform)
        }
    }
    fn exists(&self, path: &str) -> bool {
        self.fs.exists(&self.absolute(path))
    }
    fn executable(&self, path: &str) -> bool {
        self.fs.is_executable(&self.absolute(path), self.platform)
    }
    fn permit_relative_lookup(&self) -> bool {
        self.env("GODEBUG").and_then(|v| {
            v.split(',')
                .rev()
                .find_map(|entry| entry.strip_prefix("execerrdot="))
        }) == Some("0")
    }
    /// Executable lookup is independent of provider IDs. Update/version callers
    /// must pass their command's argv[0], never a launch provider as fallback.
    pub fn look_path(&self, name: &str) -> io::Result<String> {
        if matches!(name, "" | "." | "..") || name.contains('\0') {
            return Err(not_found(self.platform));
        }
        if self.platform == Platform::Unix {
            if name.contains('/') {
                return if self.executable(name) {
                    Ok(name.into())
                } else {
                    Err(not_found(self.platform))
                };
            }
            let path = self.env("PATH").unwrap_or("");
            if path.is_empty() {
                return Err(not_found(self.platform));
            }
            for directory in path.split(':') {
                let candidate = join(
                    if directory.is_empty() { "." } else { directory },
                    name,
                    self.platform,
                );
                if self.executable(&candidate) {
                    if !is_absolute(&candidate, self.platform) && !self.permit_relative_lookup() {
                        return Err(relative_lookup_error());
                    }
                    return Ok(candidate);
                }
            }
            return Err(not_found(self.platform));
        }
        let extensions = self.windows_extensions();
        if name.contains([':', '\\', '/']) {
            return self
                .find_windows_executable(name, &extensions)
                .ok_or_else(|| not_found(self.platform));
        }
        let mut relative_found = None;
        if self.env("NoDefaultCurrentDirectoryInExePath").is_none()
            && let Some(candidate) =
                self.find_windows_executable(&join(".", name, self.platform), &extensions)
        {
            if self.permit_relative_lookup() {
                return Ok(candidate);
            }
            relative_found = Some(candidate);
        }
        for directory in split_windows_path_list(self.env("PATH").unwrap_or("")) {
            if directory.is_empty() {
                continue;
            }
            let Some(candidate) =
                self.find_windows_executable(&join(&directory, name, self.platform), &extensions)
            else {
                continue;
            };
            if let Some(relative) = &relative_found
                && !self
                    .fs
                    .same_file(&self.absolute(relative), &self.absolute(&candidate))
            {
                return Err(relative_lookup_error());
            }
            if !is_absolute(&candidate, self.platform) && !self.permit_relative_lookup() {
                if relative_found.is_none() {
                    relative_found = Some(candidate);
                }
                continue;
            }
            return Ok(candidate);
        }
        if relative_found.is_some() {
            Err(relative_lookup_error())
        } else {
            Err(not_found(self.platform))
        }
    }
    fn windows_extensions(&self) -> Vec<String> {
        match self.env("PATHEXT").filter(|s| !s.is_empty()) {
            None => [".com", ".exe", ".bat", ".cmd"]
                .into_iter()
                .map(Into::into)
                .collect(),
            Some(value) => value
                .to_ascii_lowercase()
                .split(';')
                .filter(|s| !s.is_empty())
                .map(|v| {
                    if v.starts_with('.') {
                        v.into()
                    } else {
                        format!(".{v}")
                    }
                })
                .collect(),
        }
    }
    fn find_windows_executable(&self, file: &str, extensions: &[String]) -> Option<String> {
        if (extensions.is_empty() || !extension(file, self.platform).is_empty())
            && self.executable(file)
        {
            return Some(file.into());
        }
        extensions
            .iter()
            .map(|e| format!("{file}{e}"))
            .find(|candidate| self.executable(candidate))
    }
    /// Exact Go execpath.Resolve shim semantics. A .ps1/no-extension shim is
    /// unwrapped through a nearby .cmd only for an empty argument list. A .cmd
    /// itself is inspected with any args; fallback preserves all arguments.
    pub fn resolve(&self, executable: &str, args: &[String]) -> ResolvedCommand {
        if self.platform == Platform::Unix {
            return direct(executable, args);
        }
        let executable = sanitize_executable_path(executable);
        let lower = executable.to_ascii_lowercase();
        if args.is_empty()
            && (lower.ends_with(".cmd")
                || lower.ends_with(".ps1")
                || extension(&lower, self.platform).is_empty())
        {
            let ext = extension(&executable, self.platform);
            let cmd = format!("{}.cmd", &executable[..executable.len() - ext.len()]);
            if self.exists(&cmd)
                && let Some(real) = self.resolve_exe_from_cmd(&cmd)
            {
                return direct(&real, args);
            }
        }
        if lower.ends_with(".cmd") {
            if let Some(real) = self.resolve_exe_from_cmd(&executable) {
                return direct(&real, args);
            }
            let shell = self
                .env("COMSPEC")
                .filter(|s| !s.is_empty())
                .unwrap_or(r"C:\Windows\System32\cmd.exe");
            let mut wrapped = vec!["/c".into(), executable.clone()];
            wrapped.extend_from_slice(args);
            return ResolvedCommand {
                executable: shell.into(),
                args: wrapped,
                shell_shim: Some(executable),
            };
        }
        direct(&executable, args)
    }
    pub fn resolve_exe_from_cmd(&self, cmd_path: &str) -> Option<String> {
        let data = self.fs.read(&self.absolute(cmd_path)).ok()?;
        let directory = parent(cmd_path, self.platform);
        for line in String::from_utf8_lossy(&data).lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix('"') else {
                continue;
            };
            let Some((raw, _)) = rest.split_once('"') else {
                continue;
            };
            let raw = raw
                .replace("%dp0%", &directory)
                .replace("%~dp0", &directory);
            let candidate = clean(&sanitize_executable_path(&raw), self.platform);
            if extension(&candidate, self.platform).eq_ignore_ascii_case(".exe") {
                if let Some(real) = self.resolve_missing_claude_exe(&candidate) {
                    return Some(real);
                }
                if self.exists(&candidate) {
                    return Some(candidate);
                }
            }
        }
        None
    }
    fn resolve_missing_claude_exe(&self, candidate: &str) -> Option<String> {
        if self.exists(candidate) {
            return Some(candidate.into());
        }
        let suffix = join("claude-code", r"bin\claude.exe", self.platform).to_ascii_lowercase();
        if !clean(candidate, self.platform)
            .to_ascii_lowercase()
            .ends_with(&suffix)
        {
            return None;
        }
        let base = parent(&parent(candidate, self.platform), self.platform);
        for platform in ["claude-code-win32-x64", "claude-code-win32-arm64"] {
            let recovered = join(
                &base,
                &format!(r"node_modules\@anthropic-ai\{platform}\claude.exe"),
                self.platform,
            );
            if self.exists(&recovered) {
                return Some(recovered);
            }
        }
        None
    }
    /// Strict generic route for CLI update/version jobs. Missing command B is
    /// an error and cannot silently execute the provider's launch command A.
    pub fn resolve_command(&self, command: &str, args: &[String]) -> io::Result<ResolvedCommand> {
        Ok(self.resolve(&self.look_path(command)?, args))
    }
    /// Wrapper-specific compatibility route. Go intentionally retains the raw
    /// command on lookup failure so the process-start diagnostic owns the error.
    pub fn resolve_provider(
        &self,
        provider: &str,
        custom_argv: Option<&[String]>,
        args: &[String],
    ) -> ResolvedCommand {
        if let Some(custom) = custom_argv.filter(|v| !v.is_empty()) {
            let mut combined = custom[1..].to_vec();
            combined.extend_from_slice(args);
            return match self.look_path(&custom[0]) {
                Ok(path) => self.resolve(&path, &combined),
                Err(_) => direct(&custom[0], &combined),
            };
        }
        if provider == "shell" {
            return direct(&self.default_shell(), args);
        }
        if provider == "copilot" {
            if let Ok(path) = self.look_path("copilot") {
                return self.resolve(&path, args);
            }
            if let Ok(path) = self.look_path("gh") {
                let mut gh_args = vec!["copilot".into()];
                if !args.is_empty() {
                    gh_args.push("--".into());
                    gh_args.extend_from_slice(args);
                }
                return self.resolve(&path, &gh_args);
            }
            return direct(provider, args);
        }
        match self.look_path(provider) {
            Ok(path) => self.resolve(&path, args),
            Err(_) => direct(provider, args),
        }
    }
    pub fn launch_shell_shim(
        &self,
        provider: &str,
        custom_argv: Option<&[String]>,
    ) -> Option<String> {
        let resolved = self.resolve_provider(provider, custom_argv, &["probe".into()]);
        if self.platform == Platform::Windows
            && basename(&resolved.executable, self.platform).eq_ignore_ascii_case("cmd.exe")
            && resolved
                .args
                .first()
                .is_some_and(|v| v.eq_ignore_ascii_case("/c"))
        {
            resolved.args.get(1).cloned()
        } else {
            None
        }
    }
    fn default_shell(&self) -> String {
        if self.platform == Platform::Windows {
            for name in ["pwsh.exe", "powershell.exe"] {
                if let Ok(path) = self.look_path(name) {
                    return path;
                }
            }
            if let Some(shell) = self.env("COMSPEC").filter(|v| !v.is_empty())
                && self.exists(shell)
            {
                return shell.into();
            }
            r"C:\Windows\System32\cmd.exe".into()
        } else {
            if let Some(shell) = self.env("SHELL").filter(|v| !v.is_empty())
                && self.exists(shell)
            {
                return shell.into();
            }
            for name in ["bash", "sh"] {
                if let Ok(path) = self.look_path(name) {
                    return path;
                }
            }
            "/bin/sh".into()
        }
    }
}

fn direct(executable: &str, args: &[String]) -> ResolvedCommand {
    ResolvedCommand {
        executable: executable.into(),
        args: args.to_vec(),
        shell_shim: None,
    }
}
fn not_found(platform: Platform) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        if platform == Platform::Windows {
            "executable file not found in %PATH%"
        } else {
            "executable file not found in $PATH"
        },
    )
}
fn relative_lookup_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "cannot run executable found relative to current directory",
    )
}
pub fn sanitize_executable_path(path: &str) -> String {
    let mut value = path.trim();
    value = value.strip_prefix('\'').unwrap_or(value);
    value = value.strip_suffix('\'').unwrap_or(value);
    value = value.strip_prefix('"').unwrap_or(value);
    value.strip_suffix('"').unwrap_or(value).into()
}
fn basename(path: &str, platform: Platform) -> &str {
    if platform == Platform::Windows {
        path.rsplit(['/', '\\']).next().unwrap_or(path)
    } else {
        path.rsplit('/').next().unwrap_or(path)
    }
}
fn extension(path: &str, platform: Platform) -> &str {
    let name = basename(path, platform);
    name.rfind('.').map_or("", |index| &name[index..])
}
fn parent(path: &str, platform: Platform) -> String {
    let path = clean(path, platform);
    let separator = if platform == Platform::Windows {
        '\\'
    } else {
        '/'
    };
    match path.rfind(separator) {
        Some(0) => separator.to_string(),
        Some(2) if platform == Platform::Windows && path.as_bytes()[1] == b':' => path[..3].into(),
        Some(index) => path[..index].into(),
        None => ".".into(),
    }
}
fn windows_volume(path: &str) -> &str {
    if path.as_bytes().get(1) == Some(&b':') {
        return &path[..2];
    }
    if let Some(rest) = path.strip_prefix(r"\\") {
        let mut separators = rest.match_indices(['\\', '/']);
        separators.next();
        if let Some((index, _)) = separators.next() {
            return &path[..index + 2];
        }
        return path;
    }
    ""
}
fn is_absolute(path: &str, platform: Platform) -> bool {
    if platform == Platform::Unix {
        path.starts_with('/')
    } else {
        path.starts_with(r"\\")
            || (path.len() >= 3
                && path.as_bytes()[1] == b':'
                && matches!(path.as_bytes()[2], b'\\' | b'/'))
    }
}
fn join(first: &str, second: &str, platform: Platform) -> String {
    let separator = if platform == Platform::Windows {
        '\\'
    } else {
        '/'
    };
    if first.is_empty() {
        clean(second, platform)
    } else {
        clean(&format!("{first}{separator}{second}"), platform)
    }
}
fn clean(path: &str, platform: Platform) -> String {
    let windows = platform == Platform::Windows;
    let text = if windows {
        path.replace('/', "\\")
    } else {
        path.into()
    };
    let separator = if windows { '\\' } else { '/' };
    let mut prefix = String::new();
    let mut rest = text.as_str();
    let mut floor = 0;
    if windows && rest.starts_with(r"\\") {
        prefix.push_str(r"\\");
        rest = &rest[2..];
        floor = 2;
    } else if windows && rest.as_bytes().get(1) == Some(&b':') {
        prefix.push_str(&rest[..2]);
        rest = &rest[2..];
    }
    let rooted = rest.starts_with(separator);
    if rooted {
        prefix.push(separator);
        rest = rest.trim_start_matches(separator);
    }
    let mut parts = Vec::new();
    for part in rest.split(separator) {
        match part {
            "" | "." => {}
            ".." if parts.len() > floor && parts.last() != Some(&"..") => {
                parts.pop();
            }
            ".." if !rooted && prefix.is_empty() => parts.push(part),
            ".." => {}
            _ => parts.push(part),
        }
    }
    let result = format!("{prefix}{}", parts.join(&separator.to_string()));
    if result.is_empty() {
        ".".into()
    } else {
        result
    }
}
fn split_windows_path_list(path: &str) -> Vec<String> {
    if path.is_empty() {
        return vec![];
    }
    let mut entries = vec![];
    let mut current = String::new();
    let mut quoted = false;
    for ch in path.chars() {
        match ch {
            '"' => quoted = !quoted,
            ';' if !quoted => entries.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    entries.push(current);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct SyntheticFs {
        files: BTreeMap<String, Vec<u8>>,
    }
    impl SyntheticFs {
        fn put(&mut self, path: &str, content: &str) {
            self.files
                .insert(path.to_ascii_lowercase(), content.as_bytes().to_vec());
        }
    }
    impl ExecutableFs for SyntheticFs {
        fn exists(&self, path: &str) -> bool {
            self.files.contains_key(&path.to_ascii_lowercase())
        }
        fn is_executable(&self, path: &str, _: Platform) -> bool {
            self.exists(path)
        }
        fn read(&self, path: &str) -> io::Result<Vec<u8>> {
            self.files
                .get(&path.to_ascii_lowercase())
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn same_file(&self, a: &str, b: &str) -> bool {
            a.eq_ignore_ascii_case(b)
        }
    }
    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn unix_resolve_is_identity_even_for_cmd_and_quoted_paths() {
        let fs = SyntheticFs::default();
        let env = vec![];
        let resolver = Resolver::new(Platform::Unix, &env, Path::new("/synthetic"), &fs);
        let args = strings(&["line one\nline two", "a b", "\"quote\""]);
        let got = resolver.resolve(" 'x.cmd' ", &args);
        assert_eq!(got.executable, " 'x.cmd' ");
        assert_eq!(got.args, args);
        assert!(got.shell_shim.is_none());
    }
    #[test]
    fn windows_cmd_unwraps_percent_paths_and_preserves_every_argument() {
        let mut fs = SyntheticFs::default();
        fs.put(
            r"C:\bin\tool.cmd",
            "@echo off\n\"%dp0%\\node_modules\\tool\\real.exe\" %*\n",
        );
        fs.put(r"C:\bin\node_modules\tool\real.exe", "");
        let env = vec![];
        let resolver = Resolver::new(Platform::Windows, &env, Path::new(r"C:\work"), &fs);
        let args = strings(&["one two", "--flag", "line1\nline2"]);
        let got = resolver.resolve(" \"C:\\bin\\tool.cmd\" ", &args);
        assert_eq!(got.executable, r"C:\bin\node_modules\tool\real.exe");
        assert_eq!(got.args, args);
        assert!(got.shell_shim.is_none());
        assert_eq!(
            sanitize_executable_path(" '\"C:\\bin\\x.exe\"' "),
            r"C:\bin\x.exe"
        );
    }
    #[test]
    fn windows_shim_empty_args_branch_differs_from_prompt_probe() {
        let mut fs = SyntheticFs::default();
        fs.put(r"C:\bin\tool.ps1", "");
        fs.put(r"C:\bin\tool.cmd", "\"%~dp0\\real.exe\" %*\n");
        fs.put(r"C:\bin\real.exe", "");
        let env = vec![];
        let resolver = Resolver::new(Platform::Windows, &env, Path::new(r"C:\work"), &fs);
        assert_eq!(
            resolver.resolve(r"C:\bin\tool.ps1", &[]).executable,
            r"C:\bin\real.exe"
        );
        assert_eq!(
            resolver
                .resolve(r"C:\bin\tool.ps1", &strings(&["probe"]))
                .executable,
            r"C:\bin\tool.ps1"
        );
        assert_eq!(
            resolver.resolve(r"C:\bin\tool", &[]).executable,
            r"C:\bin\real.exe"
        );
        assert_eq!(
            resolver
                .resolve(r"C:\bin\tool", &strings(&["probe"]))
                .executable,
            r"C:\bin\tool"
        );
    }
    #[test]
    fn windows_cmd_fallback_and_claude_broken_package_recovery() {
        let mut fs = SyntheticFs::default();
        fs.put(r"C:\bin\tool.cmd", "@echo off\nnode script.js %*\n");
        let env = strings(&[r"ComSpec=C:\sys\cmd.exe"]);
        let resolver = Resolver::new(Platform::Windows, &env, Path::new(r"C:\work"), &fs);
        let got = resolver.resolve(r"C:\bin\tool.cmd", &strings(&["--version"]));
        assert_eq!(got.executable, r"C:\sys\cmd.exe");
        assert_eq!(got.args, strings(&["/c", r"C:\bin\tool.cmd", "--version"]));
        assert_eq!(got.shell_shim.as_deref(), Some(r"C:\bin\tool.cmd"));
        fs.put(
            r"C:\bin\claude.cmd",
            "\"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\" %*",
        );
        let recovered = r"C:\bin\node_modules\@anthropic-ai\claude-code\node_modules\@anthropic-ai\claude-code-win32-x64\claude.exe";
        fs.put(recovered, "");
        let resolver = Resolver::new(Platform::Windows, &env, Path::new(r"C:\work"), &fs);
        assert_eq!(
            resolver
                .resolve(r"C:\bin\claude.cmd", &strings(&["--version"]))
                .executable,
            recovered
        );
    }
    #[test]
    fn unix_lookup_custom_precedence_and_copilot_gh_fallback() {
        let mut fs = SyntheticFs::default();
        for p in ["/bin/gh", "/bin/custom", "/bin/sh"] {
            fs.put(p, "");
        }
        let env = strings(&["PATH=/bin"]);
        let resolver = Resolver::new(Platform::Unix, &env, Path::new("/work"), &fs);
        let got = resolver.resolve_provider("copilot", None, &strings(&["--model", "m"]));
        assert_eq!(got.executable, "/bin/gh");
        assert_eq!(got.args, strings(&["copilot", "--", "--model", "m"]));
        assert_eq!(
            resolver.resolve_provider("copilot", None, &[]).args,
            strings(&["copilot"])
        );
        let custom = strings(&["custom", "base"]);
        let got = resolver.resolve_provider("copilot", Some(&custom), &strings(&["tail"]));
        assert_eq!(got.executable, "/bin/custom");
        assert_eq!(got.args, strings(&["base", "tail"]));
        assert_eq!(
            resolver.resolve_provider("shell", None, &[]).executable,
            "/bin/sh"
        );
        assert_eq!(
            resolver.resolve_provider("missing", None, &[]).executable,
            "missing"
        );
        assert!(resolver.resolve_command("update-missing", &[]).is_err());
        assert_eq!(
            resolver
                .resolve_command("custom", &strings(&["update"]))
                .unwrap()
                .executable,
            "/bin/custom"
        );
    }
    #[test]
    fn windows_path_extensions_quotes_and_relative_lookup_guard() {
        let mut fs = SyntheticFs::default();
        fs.put(r"C:\semi;colon\tool.cmd", "");
        fs.put(r"C:\work\local.exe", "");
        let env = strings(&[r#"Path="C:\semi;colon";C:\other"#, r"PATHEXT=EXE;.CMD"]);
        let resolver = Resolver::new(Platform::Windows, &env, Path::new(r"C:\work"), &fs);
        // The Windows path list strips grouping quotes but preserves semicolons.
        assert_eq!(
            split_windows_path_list("\"C:\\semi;colon\";C:\\other"),
            strings(&[r"C:\semi;colon", r"C:\other"])
        );
        assert!(resolver.look_path("local").is_err());
        let env = strings(&["Path=\"C:\\semi;colon\";C:\\other", r"PATHEXT=EXE;.CMD"]);
        let resolver = Resolver::new(Platform::Windows, &env, Path::new(r"C:\work"), &fs);
        assert_eq!(
            resolver.look_path("tool").unwrap(),
            r"C:\semi;colon\tool.cmd"
        );
        assert_eq!(
            resolver.launch_shell_shim("tool", None).as_deref(),
            Some(r"C:\semi;colon\tool.cmd")
        );
        let env = strings(&["GODEBUG=execerrdot=0"]);
        let resolver = Resolver::new(Platform::Windows, &env, Path::new(r"C:\work"), &fs);
        assert_eq!(resolver.look_path("local").unwrap(), "local.exe");
        let env = strings(&[r"PATH=C:\work"]);
        let resolver = Resolver::new(Platform::Windows, &env, Path::new(r"C:\work"), &fs);
        assert_eq!(resolver.look_path("local").unwrap(), r"C:\work\local.exe");
    }
    #[test]
    fn unix_path_empty_entry_requires_explicit_relative_opt_in() {
        let mut fs = SyntheticFs::default();
        fs.put("/work/tool", "");
        let env = strings(&["PATH=:/other"]);
        let resolver = Resolver::new(Platform::Unix, &env, Path::new("/work"), &fs);
        assert!(resolver.look_path("tool").is_err());
        assert_eq!(resolver.look_path("./tool").unwrap(), "./tool");
        let env = strings(&["PATH=:/other", "GODEBUG=execerrdot=0"]);
        let resolver = Resolver::new(Platform::Unix, &env, Path::new("/work"), &fs);
        assert_eq!(resolver.look_path("tool").unwrap(), "tool");
        for invalid in ["", ".", ".."] {
            assert!(resolver.look_path(invalid).is_err());
        }
    }
    #[test]
    fn windows_drive_relative_and_root_shim_paths_use_supplied_context() {
        let mut fs = SyntheticFs::default();
        fs.put(r"C:\work\same.exe", "");
        fs.put(r"D:\other\different.exe", "");
        fs.put(r"C:\root.exe", "");
        fs.put(r"C:\root.cmd", "\"%dp0%\\root.exe\" %*");
        let env = strings(&[r"=D:=D:\other"]);
        let resolver = Resolver::new(Platform::Windows, &env, Path::new(r"C:\work"), &fs);
        assert_eq!(resolver.look_path(r"C:same.exe").unwrap(), r"C:same.exe");
        assert_eq!(
            resolver.look_path(r"D:different.exe").unwrap(),
            r"D:different.exe"
        );
        assert_eq!(resolver.look_path(r"\root.exe").unwrap(), r"\root.exe");
        assert_eq!(
            resolver.resolve(r"C:\root.cmd", &[]).executable,
            r"C:\root.exe"
        );
        assert_eq!(windows_volume(r"\\server\share\work"), r"\\server\share");
    }
    #[cfg(unix)]
    #[test]
    fn native_lookup_uses_synthetic_executable_permissions_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("synthetic");
        std::fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let env = vec![format!("PATH={}", root.path().display())];
        let resolver = Resolver::new(Platform::Unix, &env, root.path(), &NativeFs);
        assert!(resolver.look_path("synthetic").is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolver.look_path("synthetic").unwrap(),
            path.to_string_lossy()
        );
        assert!(resolver.resolve_command("absent-update", &[]).is_err());
    }
}
