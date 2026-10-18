//! Which Git processes are working in a repository right now.
//!
//! Used before removing an `index.lock` a stopped Git left behind. The lock
//! file cannot tell a live owner from a dead one (Git closes it while hooks
//! run, so even an exclusive open succeeds), but a live owner is a running
//! `git` whose working directory or command line names the repository. When a
//! process cannot be read, the answer is "unknown" and nothing is removed.
//!
//! Windows reads each `git.exe`'s current directory and command line from its
//! process parameters. Linux reads `/proc`. Other systems report that they
//! cannot check, so the lock stays.
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitProcess {
    pub pid: u32,
    pub cwd: Option<PathBuf>,
    pub command_line: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Using {
    /// No Git process works in the repository.
    None,
    /// These Git processes work in it.
    Running(Vec<u32>),
    /// These Git processes could not be read, so they might.
    Unknown(Vec<u32>),
}

/// Running Git processes other than this process.
pub fn git_processes() -> io::Result<Vec<GitProcess>> {
    platform::git_processes()
}

/// Whether a Git process works in the repository whose work tree is `root`.
pub fn using(root: &Path) -> io::Result<Using> {
    Ok(classify(root, &git_processes()?))
}

fn normal(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let path = if cfg!(windows) {
        path.replace('/', "\\").to_lowercase()
    } else {
        path.to_owned()
    };
    let separator = if cfg!(windows) { '\\' } else { '/' };
    path.trim_end_matches(separator).to_owned()
}

/// The spellings a process may use for `root`: as given, and resolved through
/// links (a junction to another disk, for example).
fn spellings(root: &Path) -> Vec<String> {
    let mut out = vec![normal(&root.to_string_lossy())];
    if let Ok(resolved) = std::fs::canonicalize(root) {
        let resolved = normal(&resolved.to_string_lossy());
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    out.retain(|v| !v.is_empty());
    out
}

fn inside(path: &str, root: &str) -> bool {
    let separator = if cfg!(windows) { '\\' } else { '/' };
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with(separator))
}

pub fn classify(root: &Path, processes: &[GitProcess]) -> Using {
    let roots = spellings(root);
    let mut running = Vec::new();
    let mut unknown = Vec::new();
    for process in processes {
        let in_cwd = process.cwd.as_ref().map(|cwd| {
            let cwd = normal(&cwd.to_string_lossy());
            roots.iter().any(|root| inside(&cwd, root))
        });
        let in_command = process.command_line.as_ref().is_some_and(|line| {
            let line = normal(line);
            roots.iter().any(|root| line.contains(root.as_str()))
        });
        match in_cwd {
            Some(true) => running.push(process.pid),
            _ if in_command => running.push(process.pid),
            None => unknown.push(process.pid),
            Some(false) => {}
        }
    }
    if !running.is_empty() {
        Using::Running(running)
    } else if !unknown.is_empty() {
        Using::Unknown(unknown)
    } else {
        Using::None
    }
}

#[cfg(windows)]
mod platform {
    use super::GitProcess;
    use std::{ffi::c_void, io, mem::size_of, os::windows::ffi::OsStringExt, path::PathBuf};
    use windows_sys::{
        Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation},
        Win32::{
            Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
            System::{
                Diagnostics::{
                    Debug::ReadProcessMemory,
                    ToolHelp::{
                        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                        TH32CS_SNAPPROCESS,
                    },
                },
                Threading::{
                    GetCurrentProcessId, GetExitCodeProcess, OpenProcess,
                    PROCESS_BASIC_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
                },
            },
        },
    };

    struct Owned(HANDLE);
    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful open and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn exe_name(entry: &PROCESSENTRY32W) -> String {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        String::from_utf16_lossy(&entry.szExeFile[..len])
    }

    fn read<T: Copy>(process: HANDLE, address: usize) -> Option<T> {
        let mut value = std::mem::MaybeUninit::<T>::uninit();
        let mut read = 0usize;
        // SAFETY: the buffer is exactly size_of::<T>() bytes; a failed or short
        // read leaves it unused.
        let ok = unsafe {
            ReadProcessMemory(
                process,
                address as *const c_void,
                value.as_mut_ptr().cast(),
                size_of::<T>(),
                &mut read,
            )
        } != 0;
        // SAFETY: the read filled every byte of the value.
        (ok && read == size_of::<T>()).then(|| unsafe { value.assume_init() })
    }

    /// A UNICODE_STRING in the other process: length in bytes, then a pointer.
    fn read_unicode(process: HANDLE, address: usize) -> Option<String> {
        let length = read::<u16>(process, address)? as usize;
        let buffer = read::<usize>(process, address + 8)?;
        if length == 0 || length > 64 * 1024 || buffer == 0 {
            return (length == 0).then(String::new);
        }
        let mut units = vec![0u16; length / 2];
        let mut read = 0usize;
        // SAFETY: `units` holds `length` bytes.
        let ok = unsafe {
            ReadProcessMemory(
                process,
                buffer as *const c_void,
                units.as_mut_ptr().cast(),
                length,
                &mut read,
            )
        } != 0;
        (ok && read == length).then(|| {
            std::ffi::OsString::from_wide(&units)
                .to_string_lossy()
                .into_owned()
        })
    }

    // x64 / ARM64 layouts: PEB.ProcessParameters at 0x20;
    // RTL_USER_PROCESS_PARAMETERS.CurrentDirectory.DosPath at 0x38 and
    // CommandLine at 0x70.
    const PEB_PROCESS_PARAMETERS: usize = 0x20;
    const PARAMETERS_CURRENT_DIRECTORY: usize = 0x38;
    const PARAMETERS_COMMAND_LINE: usize = 0x70;

    /// Exit code Windows reports for a process that has not exited.
    const STILL_ACTIVE: u32 = 259;
    /// What `OpenProcess` fails with once the id no longer exists.
    const ERROR_INVALID_PARAMETER: i32 = 87;

    fn exited(process: HANDLE) -> bool {
        let mut code = 0u32;
        // SAFETY: the handle was opened with query rights; `code` is written.
        (unsafe { GetExitCodeProcess(process, &mut code) } != 0) && code != STILL_ACTIVE
    }

    /// `None` when the process ended after the snapshot listed it (short Git
    /// commands often do); such a process holds nothing.
    pub(super) fn inspect(pid: u32) -> Option<GitProcess> {
        let mut out = GitProcess {
            pid,
            cwd: None,
            command_line: None,
        };
        if cfg!(not(target_pointer_width = "64")) {
            return Some(out);
        }
        // SAFETY: plain open by id; a null handle means it cannot be read.
        let handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if handle.is_null() {
            let gone = io::Error::last_os_error().raw_os_error() == Some(ERROR_INVALID_PARAMETER);
            return (!gone).then_some(out);
        }
        let process = Owned(handle);
        let read = read_parameters(process.0, &mut out);
        // A failed read of a process that has since exited is no evidence.
        if !read && exited(process.0) {
            return None;
        }
        Some(out)
    }

    /// Fill the current directory and command line; false if either failed.
    fn read_parameters(process: HANDLE, out: &mut GitProcess) -> bool {
        let mut basic: PROCESS_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let mut length = 0u32;
        // SAFETY: the buffer is a PROCESS_BASIC_INFORMATION of the given size.
        let status = unsafe {
            NtQueryInformationProcess(
                process,
                ProcessBasicInformation,
                (&mut basic as *mut PROCESS_BASIC_INFORMATION).cast(),
                size_of::<PROCESS_BASIC_INFORMATION>() as u32,
                &mut length,
            )
        };
        if status < 0 || basic.PebBaseAddress.is_null() {
            return false;
        }
        let Some(parameters) = read::<usize>(
            process,
            basic.PebBaseAddress as usize + PEB_PROCESS_PARAMETERS,
        ) else {
            return false;
        };
        if parameters == 0 {
            return false;
        }
        out.cwd = read_unicode(process, parameters + PARAMETERS_CURRENT_DIRECTORY)
            .filter(|cwd| !cwd.is_empty())
            .map(PathBuf::from);
        out.command_line = read_unicode(process, parameters + PARAMETERS_COMMAND_LINE);
        out.cwd.is_some() && out.command_line.is_some()
    }

    pub fn git_processes() -> io::Result<Vec<GitProcess>> {
        // SAFETY: a process-only snapshot; the handle is owned below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return Err(io::Error::last_os_error());
        }
        let snapshot = Owned(snapshot);
        // SAFETY: GetCurrentProcessId has no preconditions.
        let me = unsafe { GetCurrentProcessId() };
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut pids = Vec::new();
        // SAFETY: `entry.dwSize` is set as the API requires.
        let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while more {
            if entry.th32ProcessID != me && exe_name(&entry).eq_ignore_ascii_case("git.exe") {
                pids.push(entry.th32ProcessID);
            }
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        Ok(pids.into_iter().filter_map(inspect).collect())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::GitProcess;
    use std::{fs, io};

    pub fn git_processes() -> io::Result<Vec<GitProcess>> {
        let me = std::process::id();
        let mut out = Vec::new();
        for entry in fs::read_dir("/proc")? {
            let Ok(entry) = entry else { continue };
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|v| v.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == me {
                continue;
            }
            let dir = entry.path();
            let Ok(comm) = fs::read_to_string(dir.join("comm")) else {
                continue;
            };
            if comm.trim() != "git" {
                continue;
            }
            let cwd = fs::read_link(dir.join("cwd")).ok();
            let command_line = fs::read(dir.join("cmdline")).ok().map(|bytes| {
                String::from_utf8_lossy(&bytes)
                    .split('\0')
                    .collect::<Vec<_>>()
                    .join(" ")
            });
            out.push(GitProcess {
                pid,
                cwd,
                command_line,
            });
        }
        Ok(out)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::GitProcess;
    use std::io;

    pub fn git_processes() -> io::Result<Vec<GitProcess>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "running Git processes cannot be checked on this system",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, cwd: Option<&str>, command_line: Option<&str>) -> GitProcess {
        GitProcess {
            pid,
            cwd: cwd.map(PathBuf::from),
            command_line: command_line.map(str::to_owned),
        }
    }

    #[test]
    fn classify_matches_working_directory_or_command_line() {
        let root = if cfg!(windows) {
            Path::new(r"C:\Synthetic\Repo")
        } else {
            Path::new("/synthetic/repo")
        };
        let (inner, sibling, other) = if cfg!(windows) {
            (
                r"c:\synthetic\repo\src",
                r"C:\Synthetic\Repository",
                r"C:\Elsewhere",
            )
        } else {
            ("/synthetic/repo/src", "/synthetic/repository", "/elsewhere")
        };
        let root_text = root.to_string_lossy().into_owned();
        assert_eq!(classify(root, &[]), Using::None);
        assert_eq!(
            classify(root, &[process(1, Some(inner), None)]),
            Using::Running(vec![1])
        );
        assert_eq!(
            classify(root, &[process(2, Some(&root_text), None)]),
            Using::Running(vec![2])
        );
        // A neighbour whose name merely starts with the root is not inside it.
        assert_eq!(
            classify(root, &[process(3, Some(sibling), None)]),
            Using::None
        );
        let dash_c = format!("git -C {root_text} status");
        assert_eq!(
            classify(root, &[process(4, Some(other), Some(&dash_c))]),
            Using::Running(vec![4])
        );
        assert_eq!(
            classify(root, &[process(5, None, None)]),
            Using::Unknown(vec![5])
        );
        // Unreadable working directory, but the command line names the root.
        assert_eq!(
            classify(root, &[process(6, None, Some(&dash_c))]),
            Using::Running(vec![6])
        );
        // Running wins over unknown; unrelated processes are ignored.
        assert_eq!(
            classify(
                root,
                &[
                    process(7, Some(other), Some("git status")),
                    process(8, None, None),
                    process(9, Some(inner), None),
                ]
            ),
            Using::Running(vec![9])
        );
    }

    /// The real lookup sees a Git it started itself, in the directory given.
    #[cfg(windows)]
    #[test]
    fn windows_lookup_reads_a_running_git_directory_and_command_line() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("synthetic live repo");
        std::fs::create_dir(&root).unwrap();
        // `git hash-object --stdin` waits for input, so it stays alive.
        let mut child = std::process::Command::new("git")
            .args(["hash-object", "--stdin"])
            .current_dir(&root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("tests require Git on PATH");
        let seen = (0..50).find_map(|_| {
            let found = git_processes().unwrap().into_iter().find(|p| {
                p.cwd.as_deref().is_some_and(|cwd| {
                    normal(&cwd.to_string_lossy()) == normal(&root.to_string_lossy())
                })
            });
            if found.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            found
        });
        let using_root = using(&root).unwrap();
        let elsewhere = using(dir.path().join("not-a-repo").as_path()).unwrap();
        child.kill().ok();
        child.wait().ok();
        let seen = seen.expect("the started git is listed with its working directory");
        assert!(
            seen.command_line
                .unwrap_or_default()
                .contains("hash-object")
        );
        assert!(matches!(using_root, Using::Running(_)), "{using_root:?}");
        assert!(!matches!(elsewhere, Using::Running(_)), "{elsewhere:?}");
    }

    /// A Git that ended after the snapshot listed it is dropped, not counted
    /// as unreadable (2026-10-18: a parallel test's short Git made removal
    /// refuse with "could not be read").
    #[cfg(windows)]
    #[test]
    fn windows_lookup_drops_a_git_that_already_exited() {
        let mut child = std::process::Command::new("git")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("tests require Git on PATH");
        let pid = child.id();
        child.wait().unwrap();
        // `child` still holds its handle, so the id names the exited process.
        assert_eq!(platform::inspect(pid), None);
    }
}
