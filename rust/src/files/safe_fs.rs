//! Directory capabilities. Every operation is relative to a held directory, not
//! a pathname checked earlier. Symlink/reparse components are never followed.
//! A capability names the opened directory inode even if its name is moved; a
//! replacement symlink cannot redirect a write to a different object.
use std::{
    fs::{File, Metadata},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

pub fn basename(name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '\0']) {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a single basename",
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn component(name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a single native component",
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        os::fd::{AsRawFd, FromRawFd},
    };
    #[derive(Debug)]
    pub struct Dir {
        file: File,
        path: PathBuf,
    }
    fn c(name: &str) -> io::Result<CString> {
        component(name)?;
        CString::new(name).map_err(io::Error::other)
    }
    fn fd_file(fd: i32) -> io::Result<File> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: successful openat/dup returned a new owned descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    impl Dir {
        pub fn open(path: &Path) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "directory must be absolute",
                ));
            }
            let mut dir = Self {
                file: File::open("/")?,
                path: PathBuf::from("/"),
            };
            for part in path.components() {
                match part {
                    Component::RootDir => {}
                    Component::Normal(name) => {
                        let s = name
                            .to_str()
                            .ok_or_else(|| io::Error::other("non-UTF8 directory component"))?;
                        dir = dir.child_dir(s, false)?;
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "unclean directory path",
                        ));
                    }
                }
            }
            Ok(dir)
        }
        pub fn path(&self) -> &Path {
            &self.path
        }
        pub fn own_metadata(&self) -> io::Result<Metadata> {
            self.file.metadata()
        }
        pub fn mkdir_new(&self, name: &str) -> io::Result<()> {
            let n = c(name)?;
            if unsafe { libc::mkdirat(self.file.as_raw_fd(), n.as_ptr(), 0o700) } < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        pub fn child_dir(&self, name: &str, create: bool) -> io::Result<Self> {
            let n = c(name)?;
            if create {
                // SAFETY: directory fd and NUL-terminated single-component name are valid.
                let r = unsafe { libc::mkdirat(self.file.as_raw_fd(), n.as_ptr(), 0o700) };
                if r < 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() != io::ErrorKind::AlreadyExists {
                        return Err(e);
                    }
                }
            }
            // SAFETY: O_NOFOLLOW forbids replacement symlink traversal, O_DIRECTORY rejects files.
            let f = fd_file(unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    n.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            })?;
            Ok(Self {
                file: f,
                path: self.path.join(name),
            })
        }
        pub fn open_file(&self, name: &str, write: bool) -> io::Result<File> {
            let n = c(name)?;
            // Nonblocking prevents named pipes/device entries from hanging a file request.
            let flags = if write { libc::O_RDWR } else { libc::O_RDONLY };
            // SAFETY: one-component open relative to pinned directory; owned fd.
            let f = fd_file(unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    n.as_ptr(),
                    flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            })?;
            if !f.metadata()?.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "not a regular file",
                ));
            }
            Ok(f)
        }
        pub fn metadata(&self, name: &str) -> io::Result<Metadata> {
            self.open_file(name, false)?.metadata()
        }
        pub fn read(&self, name: &str, cap: usize) -> io::Result<Vec<u8>> {
            let mut out = Vec::new();
            self.open_file(name, false)?
                .take(cap as u64)
                .read_to_end(&mut out)?;
            Ok(out)
        }
        pub fn create_new(&self, name: &str, bytes: &[u8], mode: u32) -> io::Result<()> {
            let n = c(name)?;
            // SAFETY: create_new never follows/replaces an existing entry.
            let mut f = fd_file(unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    n.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    mode as libc::mode_t,
                )
            })?;
            if let Err(e) = f.write_all(bytes).and_then(|_| f.sync_all()) {
                let _ = self.remove_file(name);
                return Err(e);
            }
            Ok(())
        }
        pub fn replace(&self, name: &str, bytes: &[u8], mode: u32) -> io::Result<()> {
            basename(name)?;
            let temp = format!(".many-ai-cli-{}.tmp", crate::process::random_token()?);
            self.create_new(&temp, bytes, mode)?;
            let src = c(&temp)?;
            let dst = c(name)?;
            // SAFETY: both entries are relative to the same pinned directory.
            let r = unsafe {
                libc::renameat(
                    self.file.as_raw_fd(),
                    src.as_ptr(),
                    self.file.as_raw_fd(),
                    dst.as_ptr(),
                )
            };
            if r < 0 {
                let e = io::Error::last_os_error();
                let _ = self.remove_file(&temp);
                return Err(e);
            }
            self.file.sync_all()
        }
        pub fn rename_to(&self, name: &str, target: &Dir, new_name: &str) -> io::Result<()> {
            let src = c(name)?;
            let dst = c(new_name)?;
            #[cfg(target_os = "linux")]
            // SAFETY: renameat2 atomically refuses any existing target.
            let r = unsafe {
                libc::renameat2(
                    self.file.as_raw_fd(),
                    src.as_ptr(),
                    target.file.as_raw_fd(),
                    dst.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            #[cfg(target_os = "macos")]
            // SAFETY: Darwin exclusive rename is the native atomic no-replace operation.
            let r = unsafe {
                libc::renameatx_np(
                    self.file.as_raw_fd(),
                    src.as_ptr(),
                    target.file.as_raw_fd(),
                    dst.as_ptr(),
                    libc::RENAME_EXCL,
                )
            };
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "atomic exclusive rename unavailable",
            ));
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            if r < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
        pub fn remove_file(&self, name: &str) -> io::Result<()> {
            self.unlink(name, 0)
        }
        fn unlink(&self, name: &str, flags: i32) -> io::Result<()> {
            let n = c(name)?;
            if unsafe { libc::unlinkat(self.file.as_raw_fd(), n.as_ptr(), flags) } < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
        pub fn entries(&self) -> io::Result<Vec<String>> {
            // dup shares the directory offset; open "." creates an independent description.
            let fd = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    c".".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let dp = unsafe { libc::fdopendir(fd) };
            if dp.is_null() {
                unsafe { libc::close(fd) };
                return Err(io::Error::last_os_error());
            }
            let mut out = Vec::new();
            loop {
                let ent = unsafe { libc::readdir(dp) };
                if ent.is_null() {
                    break;
                }
                let raw = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
                let name = match raw.to_str() {
                    Ok(name) => name.to_owned(),
                    Err(_) => {
                        unsafe { libc::closedir(dp) };
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "non-UTF8 directory entry",
                        ));
                    }
                };
                if name != "." && name != ".." {
                    out.push(name);
                }
            }
            unsafe { libc::closedir(dp) };
            out.sort();
            Ok(out)
        }
        pub fn remove_tree(&self, name: &str) -> io::Result<()> {
            let dir = self.child_dir(name, false)?;
            dir.remove_contents(0)?;
            self.unlink(name, libc::AT_REMOVEDIR)
        }
        fn remove_contents(&self, depth: usize) -> io::Result<()> {
            if depth > 128 {
                return Err(io::Error::other("directory depth exceeds deletion limit"));
            }
            for name in self.entries()? {
                match self.child_dir(&name, false) {
                    Ok(dir) => {
                        dir.remove_contents(depth + 1)?;
                        self.unlink(&name, libc::AT_REMOVEDIR)?;
                    }
                    Err(_) => self.remove_file(&name)?,
                }
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        fs::OpenOptions,
        os::windows::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
            io::FromRawHandle,
        },
    };
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    #[derive(Debug)]
    pub struct Dir {
        path: PathBuf,
        _pins: Vec<File>,
    }
    fn pin(path: &Path) -> io::Result<File> {
        let f = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let m = f.metadata()?;
        if !m.is_dir() || m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::other("not a plain directory"));
        }
        Ok(f)
    }
    fn wide(path: &Path) -> io::Result<Vec<u16>> {
        let value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "null in path"));
        }
        Ok(value.into_iter().chain([0]).collect())
    }
    fn create_private_dir(path: &Path) -> io::Result<()> {
        let security = crate::config::private_io::PrivateSecurity::for_current_user(true)?;
        let attributes = security.attributes();
        let path = wide(path)?;
        // SAFETY: the private descriptor, attributes, and path outlive creation.
        if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    fn move_file(src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
        use std::os::windows::ffi::OsStrExt;
        let s: Vec<u16> = src.as_os_str().encode_wide().chain([0]).collect();
        let d: Vec<u16> = dst.as_os_str().encode_wide().chain([0]).collect();
        let f = MOVEFILE_WRITE_THROUGH
            | if replace {
                MOVEFILE_REPLACE_EXISTING
            } else {
                0
            };
        if unsafe { MoveFileExW(s.as_ptr(), d.as_ptr(), f) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    impl Dir {
        pub fn open(path: &Path) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(io::Error::other("directory must be absolute"));
            }
            let mut p = PathBuf::new();
            let mut pins = Vec::new();
            for comp in path.components() {
                match comp {
                    Component::ParentDir | Component::CurDir => {
                        return Err(io::Error::other("unclean directory path"));
                    }
                    _ => {
                        p.push(comp);
                        if p.is_absolute() {
                            pins.push(pin(&p)?);
                        }
                    }
                }
            }
            Ok(Self {
                path: p,
                _pins: pins,
            })
        }
        pub fn path(&self) -> &Path {
            &self.path
        }
        pub fn own_metadata(&self) -> io::Result<Metadata> {
            self._pins
                .last()
                .ok_or_else(|| io::Error::other("directory not pinned"))?
                .metadata()
        }
        pub fn mkdir_new(&self, name: &str) -> io::Result<()> {
            basename(name)?;
            std::fs::create_dir(self.path.join(name))
        }

        pub fn child_dir(&self, name: &str, create: bool) -> io::Result<Self> {
            basename(name)?;
            let p = self.path.join(name);
            if create {
                match create_private_dir(&p) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
            }
            Self::open(&p)
        }
        pub fn open_file(&self, name: &str, write: bool) -> io::Result<File> {
            basename(name)?;
            let f = OpenOptions::new()
                .read(true)
                .write(write)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join(name))?;
            let m = f.metadata()?;
            if !m.is_file() || m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(io::Error::other("not a plain file"));
            }
            Ok(f)
        }
        pub fn metadata(&self, name: &str) -> io::Result<Metadata> {
            self.open_file(name, false)?.metadata()
        }
        pub fn read(&self, name: &str, cap: usize) -> io::Result<Vec<u8>> {
            let mut out = Vec::new();
            self.open_file(name, false)?
                .take(cap as u64)
                .read_to_end(&mut out)?;
            Ok(out)
        }
        pub fn create_new(&self, name: &str, bytes: &[u8], mode: u32) -> io::Result<()> {
            basename(name)?;
            let path = self.path.join(name);
            let mut file = if mode & 0o077 == 0 {
                let security = crate::config::private_io::PrivateSecurity::for_current_user(false)?;
                let attributes = security.attributes();
                let path = wide(&path)?;
                // SAFETY: no payload exists before the protected DACL is installed
                // atomically by CreateFileW; existing entries are never followed.
                let handle = unsafe {
                    CreateFileW(
                        path.as_ptr(),
                        FILE_GENERIC_WRITE,
                        FILE_SHARE_READ,
                        &attributes,
                        CREATE_NEW,
                        FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                        std::ptr::null_mut(),
                    )
                };
                if handle == INVALID_HANDLE_VALUE {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: successful CreateFileW returns this newly-owned handle.
                unsafe { File::from_raw_handle(handle) }
            } else {
                // Ordinary workspace files retain Go's 0644/inherited ACL policy.
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?
            };
            if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                drop(file);
                let _ = self.remove_file(name);
                return Err(error);
            }
            Ok(())
        }
        pub fn replace(&self, name: &str, bytes: &[u8], mode: u32) -> io::Result<()> {
            basename(name)?;
            let temp = format!(".many-ai-cli-{}.tmp", crate::process::random_token()?);
            self.create_new(&temp, bytes, mode)?;
            let result = move_file(&self.path.join(&temp), &self.path.join(name), true);
            if result.is_err() {
                let _ = self.remove_file(&temp);
            }
            result
        }
        pub fn rename_to(&self, name: &str, target: &Dir, new_name: &str) -> io::Result<()> {
            basename(name)?;
            basename(new_name)?;
            move_file(&self.path.join(name), &target.path.join(new_name), false)
        }
        pub fn remove_file(&self, name: &str) -> io::Result<()> {
            basename(name)?;
            std::fs::remove_file(self.path.join(name))
        }
        pub fn entries(&self) -> io::Result<Vec<String>> {
            let mut out = std::fs::read_dir(&self.path)?
                .map(|e| {
                    e.and_then(|e| {
                        e.file_name().into_string().map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "non-Unicode directory entry",
                            )
                        })
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            out.sort();
            Ok(out)
        }
        pub fn remove_tree(&self, name: &str) -> io::Result<()> {
            let child = self.child_dir(name, false)?;
            for n in child.entries()? {
                if child.child_dir(&n, false).is_ok() {
                    child.remove_tree(&n)?;
                } else {
                    child.remove_file(&n)?;
                }
            }
            drop(child);
            std::fs::remove_dir(self.path.join(name))
        }
    }
}
pub use platform::Dir;
