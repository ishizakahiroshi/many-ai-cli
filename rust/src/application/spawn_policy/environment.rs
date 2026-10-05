//! Spawn-time PATH cleanup. Registry/filesystem sources are explicit and can be
//! replaced by synthetic inputs; Linux fixtures do not prove native Windows.
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub trait PathEnvironment: Send + Sync {
    fn sanitize(&self, environment: &[String]) -> io::Result<Vec<String>>;
}
/// Only use in production. Trials should inject a sanitizer whose filesystem and
/// registry sources are confined to the trial's owned inputs.
pub struct NativePathEnvironment;
impl PathEnvironment for NativePathEnvironment {
    fn sanitize(&self, environment: &[String]) -> io::Result<Vec<String>> {
        #[cfg(windows)]
        {
            WindowsPathEnvironment::new(NativeWindowsSources).sanitize(environment)
        }
        #[cfg(not(windows))]
        {
            Ok(sanitize(environment, ':', |parts| parts.to_vec()))
        }
    }
}
fn sanitize(
    environment: &[String],
    separator: char,
    mut expand: impl FnMut(&[String]) -> Vec<String>,
) -> Vec<String> {
    environment
        .iter()
        .map(|entry| {
            let Some((key, value)) = entry.split_once('=') else {
                return entry.clone();
            };
            if !key.eq_ignore_ascii_case("Path") {
                return entry.clone();
            }
            let parts = value
                .split(separator)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let expanded = expand(&parts);
            let value = expanded
                .iter()
                .filter(|v| !v.trim().is_empty())
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(&separator.to_string());
            format!("{key}={value}")
        })
        .collect()
}
pub trait WindowsEnvironmentSources: Send + Sync {
    /// Return raw registry text, including unexpanded REG_EXPAND_SZ values.
    fn user_value(&self, name: &str) -> Option<String>;
    fn machine_value(&self, name: &str) -> Option<String>;
    fn is_dir(&self, path: &str) -> bool;
}
pub struct WindowsPathEnvironment<S: WindowsEnvironmentSources> {
    sources: S,
}
impl<S: WindowsEnvironmentSources> WindowsPathEnvironment<S> {
    pub fn new(sources: S) -> Self {
        Self { sources }
    }
    fn expand(&self, parts: &[String], environment: &[String]) -> Vec<String> {
        let mut context = WindowsExpansion {
            source: &self.sources,
            environment,
            cache: BTreeMap::new(),
        };
        let mut out = vec![];
        let mut seen = BTreeSet::new();
        let mut add = |value: String| {
            let key = value.trim_end_matches(['\\', '/']).to_lowercase();
            if !key.is_empty() && seen.insert(key) {
                out.push(value);
            }
        };
        for raw in parts {
            add(if raw.contains('%') {
                context.expand_refs(raw).unwrap_or_else(|| raw.clone())
            } else {
                raw.clone()
            });
        }
        for raw in [
            self.sources.user_value("Path"),
            self.sources.machine_value("Path"),
        ]
        .into_iter()
        .flatten()
        {
            for entry in raw.split(';').map(str::trim).filter(|e| !e.is_empty()) {
                if entry.contains('%') {
                    if let Some(value) = context.expand_refs(entry) {
                        add(value);
                    }
                } else {
                    add(entry.into());
                }
            }
        }
        let mut fallback = vec![];
        if let Some(value) = context.value("LOCALAPPDATA") {
            fallback.push(join(&value, "pnpm"));
        }
        if let Some(value) = context.value("APPDATA") {
            fallback.push(join(&value, "npm"));
        }
        if let Some(value) = context.value("USERPROFILE") {
            fallback.push(join(&value, "scoop\\shims"));
            fallback.push(join(&value, ".local\\bin"));
        }
        for candidate in fallback {
            if self.sources.is_dir(&candidate) {
                add(candidate);
            }
        }
        out
    }
}
impl<S: WindowsEnvironmentSources> PathEnvironment for WindowsPathEnvironment<S> {
    fn sanitize(&self, environment: &[String]) -> io::Result<Vec<String>> {
        Ok(sanitize(environment, ';', |parts| {
            self.expand(parts, environment)
        }))
    }
}
fn join(base: &str, suffix: &str) -> String {
    format!("{}\\{suffix}", base.trim_end_matches(['/', '\\']))
}
struct WindowsExpansion<'a> {
    source: &'a dyn WindowsEnvironmentSources,
    environment: &'a [String],
    cache: BTreeMap<String, Option<String>>,
}
impl WindowsExpansion<'_> {
    fn env(&self, name: &str) -> Option<String> {
        self.environment.iter().rev().find_map(|e| {
            e.split_once('=')
                .filter(|(key, value)| key.eq_ignore_ascii_case(name) && !value.is_empty())
                .map(|(_, value)| value.into())
        })
    }
    fn value(&mut self, name: &str) -> Option<String> {
        let key = name.to_uppercase();
        if let Some(value) = self.cache.get(&key) {
            return value.clone();
        }
        let value = self
            .env(name)
            .or_else(|| {
                self.source
                    .user_value(name)
                    .filter(|v| !v.is_empty() && !v.contains('%'))
            })
            .or_else(|| {
                if !name.eq_ignore_ascii_case("PNPM_HOME") {
                    return None;
                }
                let candidate = join(&self.env("LOCALAPPDATA")?, "pnpm");
                self.source.is_dir(&candidate).then_some(candidate)
            });
        self.cache.insert(key, value.clone());
        value
    }
    fn expand_refs(&mut self, input: &str) -> Option<String> {
        let mut rest = input;
        let mut out = String::new();
        let mut any = false;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let Some(end) = after.find('%') else {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let name = &after[..end];
            if name.is_empty() {
                out.push_str("%%");
            } else {
                out.push_str(&self.value(name)?);
                any = true;
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        any.then_some(out)
    }
}

#[cfg(windows)]
struct NativeWindowsSources;
#[cfg(windows)]
impl WindowsEnvironmentSources for NativeWindowsSources {
    fn user_value(&self, name: &str) -> Option<String> {
        native_registry(
            windows_sys::Win32::System::Registry::HKEY_CURRENT_USER,
            "Environment",
            name,
        )
    }
    fn machine_value(&self, name: &str) -> Option<String> {
        native_registry(
            windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE,
            "System\\CurrentControlSet\\Control\\Session Manager\\Environment",
            name,
        )
    }
    fn is_dir(&self, path: &str) -> bool {
        std::path::Path::new(path).is_dir()
    }
}
#[cfg(windows)]
fn native_registry(
    root: windows_sys::Win32::System::Registry::HKEY,
    path: &str,
    name: &str,
) -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        HKEY, KEY_QUERY_VALUE, REG_EXPAND_SZ, REG_SZ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    struct Owned(HKEY);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let path = wide(path);
    let name = wide(name);
    let mut raw = std::ptr::null_mut();
    // SAFETY: predefined HKEY and NUL-terminated input buffers; successful key
    // acquisition is owned by Owned, and all returned lengths are bounded.
    if unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_QUERY_VALUE, &mut raw) } != 0 {
        return None;
    }
    let key = Owned(raw);
    let mut bytes = 0;
    let mut kind = 0;
    if unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            std::ptr::null_mut(),
            &mut bytes,
        )
    } != 0
        || !matches!(kind, REG_SZ | REG_EXPAND_SZ)
        || bytes > 1024 * 1024
        || bytes % 2 != 0
    {
        return None;
    }
    let mut data = vec![0_u16; bytes as usize / 2];
    if unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            data.as_mut_ptr().cast(),
            &mut bytes,
        )
    } != 0
        || !matches!(kind, REG_SZ | REG_EXPAND_SZ)
        || bytes as usize > data.len() * 2
        || bytes % 2 != 0
    {
        return None;
    }
    data.truncate(bytes as usize / 2);
    while data.last() == Some(&0) {
        data.pop();
    }
    String::from_utf16(&data).ok()
}
