//! Unix operations are relative to held directories; Windows retains directory
//! pins without delete sharing and rejects reparse points at acquisition.
//! Fixed macOS system aliases are expanded before the no-follow walk. Native
//! Windows concurrent reparse mutation is not an established security boundary.
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
    // Native first-create fixtures at 1d14fc1 reproduce Darwin ENOENT from
    // openat itself with a held, existing parent. Retry only that lock-leaf
    // operation, unchanged, at most three times; persistent absence still fails.
    // Other Unix platforms keep one syscall. No path re-resolution/fallback.
    const LOCK_OPEN_ATTEMPTS: usize = if cfg!(target_os = "macos") { 3 } else { 1 };
    fn lock_open<T>(attempts: usize, mut open: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        for attempt in 0..attempts {
            match open() {
                Err(error)
                    if error.raw_os_error() == Some(libc::ENOENT) && attempt + 1 < attempts => {}
                result => return result,
            }
        }
        unreachable!("lock open always has at least one attempt")
    }
    // Context contains only a fixed operation label and OS error, never paths or
    // file contents. Preserve ErrorKind for source-compatible caller handling.
    fn at(operation: &'static str, error: io::Error) -> io::Error {
        io::Error::new(error.kind(), format!("{operation}: {error}"))
    }
    impl Dir {
        pub fn open(path: &Path) -> io::Result<Self> {
            let path = crate::config::paths::native_system_path(path);
            let path = path.as_ref();
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
        /// Export one owned duplicate for a descriptor-anchored storage VFS.
        /// The caller never receives an unowned raw descriptor.
        pub fn try_clone_file(&self) -> io::Result<File> {
            self.file.try_clone()
        }

        /// Restrict this opened directory, without resolving its pathname again.
        pub fn restrict_private(&self) -> io::Result<()> {
            // SAFETY: this capability owns a verified directory descriptor.
            if unsafe { libc::fchmod(self.file.as_raw_fd(), 0o700) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
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
        /// Match write-only O_CREAT without truncating before inode validation.
        /// Existing permissions remain unchanged; all first creators share one
        /// directory-relative inode even when their opens overlap.
        pub fn open_write_or_create(&self, name: &str, mode: u32) -> io::Result<File> {
            let name = c(name)?;
            // SAFETY: one validated component beneath the pinned directory.
            let file = fd_file(unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC
                        | libc::O_NONBLOCK,
                    mode,
                )
            })?;
            if !file.metadata()?.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "not a regular file",
                ));
            }
            Ok(file)
        }
        /// Open a private log for kernel-atomic append. The handle stays bound
        /// to this directory inode across rename; no seek-then-write window exists.
        pub fn open_append(&self, name: &str) -> io::Result<File> {
            basename(name)?;
            let name = c(name)?;
            // SAFETY: openat resolves one validated component beneath our held
            // directory. O_NOFOLLOW rejects a symlink and O_NONBLOCK prevents
            // opening an attacker-created FIFO from hanging before fstat.
            let file = fd_file(unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_APPEND
                        | libc::O_CREAT
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC
                        | libc::O_NONBLOCK,
                    0o600u32,
                )
            })?;
            if !file.metadata()?.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "not a regular file",
                ));
            }
            // Existing logs must also be private. Change the opened inode,
            // never a path that a concurrent rename could replace.
            // SAFETY: file owns this descriptor and 0600 is a valid mode.
            if unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(file)
        }
        /// Open/create one private lock inode without a create-then-reopen
        /// sharing window. The caller owns the returned OS lock lifetime.
        pub fn open_lock(&self, name: &str) -> io::Result<File> {
            let name = c(name)?;
            // SAFETY: relative to the held directory; no truncation or following.
            // O_NONBLOCK prevents a non-regular entry from stalling before fstat.
            let file = lock_open(LOCK_OPEN_ATTEMPTS, || {
                fd_file(unsafe {
                    libc::openat(
                        self.file.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDWR
                            | libc::O_CREAT
                            | libc::O_NOFOLLOW
                            | libc::O_CLOEXEC
                            | libc::O_NONBLOCK,
                        0o600u32,
                    )
                })
            })
            .map_err(|e| at("lock openat", e))?;
            if !file.metadata().map_err(|e| at("lock fstat", e))?.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "not a regular lock file",
                ));
            }
            if unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
                return Err(at("lock fchmod", io::Error::last_os_error()));
            }
            Ok(file)
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
            // C varargs require promoted integer width; macOS mode_t is u16.
            let mut f = fd_file(unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    n.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    mode,
                )
            })
            .map_err(|e| at("create openat", e))?;
            if let Err(e) = f
                .write_all(bytes)
                .map_err(|e| at("create write", e))
                .and_then(|_| f.sync_all().map_err(|e| at("create sync", e)))
            {
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
                return Err(at("replace renameat", e));
            }
            self.file
                .sync_all()
                .map_err(|e| at("replace directory sync", e))
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
    #[cfg(test)]
    mod lock_open_tests {
        use super::*;
        #[test]
        fn persistent_absence_is_bounded_and_other_errors_are_immediate() {
            let mut calls = 0;
            let result: io::Result<()> = lock_open(3, || {
                calls += 1;
                Err(io::Error::from_raw_os_error(libc::ENOENT))
            });
            assert_eq!(calls, 3);
            assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::ENOENT));
            calls = 0;
            let result: io::Result<()> = lock_open(3, || {
                calls += 1;
                Err(io::Error::from_raw_os_error(libc::EACCES))
            });
            assert_eq!(calls, 1);
            assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EACCES));
            calls = 0;
            let result: io::Result<()> = lock_open(1, || {
                calls += 1;
                Err(io::Error::from_raw_os_error(libc::ENOENT))
            });
            assert_eq!(calls, 1);
            assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::ENOENT));
        }
        #[test]
        fn transient_first_creation_keeps_the_successful_result() {
            let mut calls = 0;
            let result = lock_open(3, || {
                calls += 1;
                if calls < 3 {
                    Err(io::Error::from_raw_os_error(libc::ENOENT))
                } else {
                    Ok(19)
                }
            });
            assert_eq!(result.unwrap(), 19);
            assert_eq!(calls, 3);
            assert_eq!(
                LOCK_OPEN_ATTEMPTS,
                if cfg!(target_os = "macos") { 3 } else { 1 }
            );
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
        use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, SYNCHRONIZE};
        let f = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            // Do not deny ordinary writes merely because a directory is held.
            // Delete sharing stays absent, so the pinned name cannot be renamed.
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
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
                    // A verbatim disk prefix alone (e.g. \\?\C:) is not
                    // the volume's root directory. Wait for RootDir before
                    // opening it; all actual directory components are pinned.
                    Component::Prefix(_) => p.push(comp),
                    Component::RootDir | Component::Normal(_) => {
                        p.push(comp);
                        if p.is_absolute() {
                            pins.push(pin(&p)?);
                        }
                    }
                }
            }
            if pins.is_empty() {
                return Err(io::Error::other("directory has no root component"));
            }
            Ok(Self {
                path: p,
                _pins: pins,
            })
        }
        pub fn path(&self) -> &Path {
            &self.path
        }
        /// The held pin denies rename/delete. Reopen that object with WRITE_DAC
        /// for ACL repair; concurrent reparse mutation still needs native review.
        pub fn restrict_private(&self) -> io::Result<()> {
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_READ_ATTRIBUTES, READ_CONTROL, SYNCHRONIZE, WRITE_DAC,
            };
            let file = OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC | SYNCHRONIZE)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&self.path)?;
            let metadata = file.metadata()?;
            if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "not a plain directory",
                ));
            }
            crate::config::private_io::PrivateSecurity::for_current_user(true)?.restrict_file(&file)
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
        pub fn open_write_or_create(&self, name: &str, _mode: u32) -> io::Result<File> {
            use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, OPEN_ALWAYS};
            basename(name)?;
            let security = crate::config::private_io::PrivateSecurity::for_current_user(false)?;
            let attributes = security.attributes();
            let path = wide(&self.path.join(name))?;
            // SAFETY: held ancestors cannot be replaced; OPEN_ALWAYS admits
            // concurrent first writers. No read-data, truncate or delete access.
            // The ACL applies only on creation; existing file permissions stay.
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &attributes,
                    OPEN_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                    std::ptr::null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: this successful call returns one newly owned handle.
            let file = unsafe { File::from_raw_handle(handle) };
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "not a plain file",
                ));
            }
            Ok(file)
        }
        /// Append-only access keeps concurrent writers at EOF in the kernel.
        /// Every ancestor is held without delete sharing by this Dir.
        pub fn open_append(&self, name: &str) -> io::Result<File> {
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_APPEND_DATA, FILE_READ_ATTRIBUTES, OPEN_ALWAYS, READ_CONTROL, SYNCHRONIZE,
                WRITE_DAC,
            };
            basename(name)?;
            let security = crate::config::private_io::PrivateSecurity::for_current_user(false)?;
            let attributes = security.attributes();
            let path = wide(&self.path.join(name))?;
            // SAFETY: the protected descriptor and path outlive CreateFileW;
            // the append-only access mask omits FILE_WRITE_DATA. OPEN_REPARSE
            // opens any final reparse object itself so validation can reject it.
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    FILE_APPEND_DATA
                        | FILE_READ_ATTRIBUTES
                        | READ_CONTROL
                        | WRITE_DAC
                        | SYNCHRONIZE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &attributes,
                    OPEN_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                    std::ptr::null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: successful creation/open returned one newly-owned handle.
            let file = unsafe { File::from_raw_handle(handle) };
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "not a plain file",
                ));
            }
            security.restrict_file(&file)?;
            Ok(file)
        }
        /// All contenders open the same lock file with compatible access and
        /// sharing before LockFileEx serializes them. CREATE_NEW with read-only
        /// sharing can fail while an earlier read/write contender holds it.
        pub fn open_lock(&self, name: &str) -> io::Result<File> {
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_GENERIC_READ, OPEN_ALWAYS, WRITE_DAC,
            };
            basename(name)?;
            let security = crate::config::private_io::PrivateSecurity::for_current_user(false)?;
            let attributes = security.attributes();
            let path = wide(&self.path.join(name))?;
            // SAFETY: descriptor/path outlive this call; no inheritance, truncate
            // or delete sharing. Reparse objects are opened themselves/rejected.
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &attributes,
                    OPEN_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                    std::ptr::null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: this successful call returned a new owned handle.
            let file = unsafe { File::from_raw_handle(handle) };
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "not a plain lock file",
                ));
            }
            security.restrict_file(&file)?;
            Ok(file)
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

impl Dir {
    /// Walk from the absolute filesystem root through directory capabilities.
    /// New components are private from creation; only the requested final
    /// directory is restricted if it already existed. Existing ancestors (for
    /// example the user's home or a volume root) keep their original policy.
    pub fn open_or_create_private(path: &Path) -> io::Result<Self> {
        let path = crate::config::paths::native_system_path(path);
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "directory must be absolute",
            ));
        }
        let mut root = PathBuf::new();
        let mut names = Vec::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) | Component::RootDir if names.is_empty() => {
                    root.push(component)
                }
                Component::Normal(name) => names.push(
                    name.to_str()
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "non-Unicode directory component",
                            )
                        })?
                        .to_owned(),
                ),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "unclean directory path",
                    ));
                }
            }
        }
        if names.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "private directory must not be a filesystem root",
            ));
        }
        let mut directory = Self::open(&root)?;
        for name in names {
            directory = directory.child_dir(&name, true)?;
        }
        directory.restrict_private()?;
        Ok(directory)
    }
}

#[cfg(all(test, windows))]
mod windows_path_tests {
    use super::*;

    #[test]
    fn canonical_verbatim_directory_opens_and_keeps_same_owned_path() {
        let root = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(root.path()).unwrap();
        assert!(matches!(
            canonical.components().next(),
            Some(Component::Prefix(_))
        ));
        let held = Dir::open(&canonical).unwrap();
        assert_eq!(held.path(), canonical);
        let child = held.child_dir("ordinary-child", true).unwrap();
        child
            .create_new("fixture.txt", b"owned fixture", 0o600)
            .unwrap();
        assert_eq!(child.read("fixture.txt", 100).unwrap(), b"owned fixture");
    }
}

#[cfg(test)]
mod lock_file_tests {
    use super::*;

    #[test]
    fn simultaneous_first_lock_opens_and_atomic_writes_use_one_inode() {
        use std::sync::{Arc, Barrier};
        // Each round starts with no lock entry. This exercises native concurrent
        // O_CREAT/OPEN_ALWAYS, rather than only reopening an existing lock.
        let root = tempfile::tempdir().unwrap();
        for round in 0..8 {
            let first = Dir::open(root.path()).unwrap();
            let second = Dir::open(root.path()).unwrap();
            let name = format!("first-open-{round}.lock");
            let barrier = Arc::new(Barrier::new(2));
            std::thread::scope(|scope| {
                for dir in [&first, &second] {
                    let barrier = barrier.clone();
                    let name = &name;
                    scope.spawn(move || {
                        barrier.wait();
                        for _ in 0..4 {
                            let lock = dir.open_lock(name).unwrap();
                            lock.lock().expect("acquire native lock");
                            dir.replace("owned-state.json", b"{\"fixture\":true}", 0o600)
                                .unwrap();
                        }
                    });
                }
            });
        }
    }

    #[test]
    fn independent_lock_opens_serialize_without_truncating_the_same_file() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open(root.path()).unwrap();
        let mut first = dir.open_lock("fixture.lock").unwrap();
        first.lock().unwrap();
        first.write_all(b"owned lock fixture").unwrap();
        first.sync_all().unwrap();
        let mut second = dir.open_lock("fixture.lock").unwrap();
        assert!(matches!(
            second.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(first);
        second.lock().unwrap();
        let mut bytes = vec![];
        second.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"owned lock fixture");
    }
}
