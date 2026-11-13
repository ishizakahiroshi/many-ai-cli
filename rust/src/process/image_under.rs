//! Which running processes were started from an executable under a directory.
//!
//! Read-only. Used by `doctor` to point at a `whisper-server` that an older
//! many-ai-cli left running from `~/.many-ai-cli/whisper/`; nothing is ever
//! signalled or ended here. A process whose image path cannot be read is not
//! listed (the answer is "not shown", never a guess).
use std::{io, path::Path};

/// Ids of the processes (other than this one) whose executable file lies
/// under `dir`. Unsupported systems report that they cannot check.
pub fn processes_under(dir: &Path) -> io::Result<Vec<u32>> {
    let me = std::process::id();
    let mut pids: Vec<u32> = platform::images()?
        .into_iter()
        .filter(|(pid, image)| *pid != me && is_under(image, dir))
        .map(|(pid, _)| pid)
        .collect();
    pids.sort_unstable();
    Ok(pids)
}

fn normal(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if cfg!(windows) {
        text.replace('/', "\\").to_lowercase()
    } else {
        text.to_owned()
    }
}

/// Lexical containment, comparing the path as given and as resolved through
/// links for `dir`.
fn is_under(image: &Path, dir: &Path) -> bool {
    let separator = if cfg!(windows) { '\\' } else { '/' };
    let mut roots = vec![normal(dir)];
    if let Ok(resolved) = std::fs::canonicalize(dir) {
        roots.push(normal(&resolved));
    }
    let image = normal(image);
    roots.iter().any(|root| {
        let root = root.trim_end_matches(separator);
        !root.is_empty()
            && image.len() > root.len()
            && image.starts_with(root)
            && image[root.len()..].starts_with(separator)
    })
}

#[cfg(windows)]
mod platform {
    use std::{io, path::PathBuf};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    };

    fn image(pid: u32) -> Option<PathBuf> {
        // SAFETY: plain open by id; a null handle means it cannot be read.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        // SAFETY: `buffer` holds `length` units; the handle is closed once.
        let ok =
            unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } != 0;
        unsafe { CloseHandle(handle) };
        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize])))
    }

    pub fn images() -> io::Result<Vec<(u32, PathBuf)>> {
        // SAFETY: a process-only snapshot, closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut pids = Vec::new();
        // SAFETY: `dwSize` is set as the API requires.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more {
            pids.push(entry.th32ProcessID);
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snapshot) };
        Ok(pids
            .into_iter()
            .filter_map(|pid| image(pid).map(|path| (pid, path)))
            .collect())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::{io, path::PathBuf};

    pub fn images() -> io::Result<Vec<(u32, PathBuf)>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir("/proc")? {
            let Ok(entry) = entry else { continue };
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|v| v.parse::<u32>().ok())
            else {
                continue;
            };
            if let Ok(path) = std::fs::read_link(entry.path().join("exe")) {
                out.push((pid, path));
            }
        }
        Ok(out)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use std::{io, path::PathBuf};

    pub fn images() -> io::Result<Vec<(u32, PathBuf)>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "running processes cannot be listed on this system",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn containment_is_by_path_component_not_prefix() {
        let (dir, inside, sibling) = if cfg!(windows) {
            (
                r"C:\Synthetic\.many-ai-cli\whisper",
                r"c:\synthetic\.many-ai-cli\WHISPER\bin\whisper-server.exe",
                r"C:\Synthetic\.many-ai-cli\whisper-other\x.exe",
            )
        } else {
            (
                "/synthetic/.many-ai-cli/whisper",
                "/synthetic/.many-ai-cli/whisper/bin/whisper-server",
                "/synthetic/.many-ai-cli/whisper-other/x",
            )
        };
        let dir = PathBuf::from(dir);
        assert!(is_under(Path::new(inside), &dir));
        assert!(!is_under(Path::new(sibling), &dir));
        assert!(!is_under(&dir, &dir));
    }

    #[test]
    fn unrelated_directory_has_no_processes() {
        let t = tempfile::tempdir().unwrap();
        if let Ok(pids) = processes_under(t.path()) {
            assert!(pids.is_empty());
        }
    }
}
