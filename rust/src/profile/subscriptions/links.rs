use std::{io, path::Path};
pub(super) fn file(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(source, target)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (source, target);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "profile file links are not implemented for this platform",
        ))
    }
}
pub(super) fn directory(source: &Path, target: &Path) -> io::Result<()> {
    if !source.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "seed source is not a directory",
        ));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(source, target).is_ok() {
            return Ok(());
        }
        junction(source, target)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = target;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "profile directory links are not implemented for this platform",
        ))
    }
}
#[cfg(windows)]
fn junction(source: &Path, target: &Path) -> io::Result<()> {
    use std::{
        fs::File,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle},
        },
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, OPEN_EXISTING,
        },
        System::IO::DeviceIoControl,
    };
    let absolute = std::path::absolute(source)?;
    let display: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    // Rust canonical RuntimePaths can carry a Win32 verbatim prefix. The NT
    // namespace uses \??\ directly; never build the invalid \??\\\?\ form.
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let native = display
        .strip_prefix(verbatim.as_slice())
        .unwrap_or(&display);
    let substitute = "\\??\\"
        .encode_utf16()
        .chain(native.iter().copied())
        .collect::<Vec<_>>();
    let size = 16 + (substitute.len() + display.len() + 2) * 2;
    if size > 16 * 1024 || display.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid junction target",
        ));
    }
    let mut buffer = vec![0_u8; size];
    buffer[..4].copy_from_slice(&0xA0000003_u32.to_le_bytes());
    for (offset, value) in [
        (4, size - 8),
        (10, substitute.len() * 2),
        (12, (substitute.len() + 1) * 2),
        (14, display.len() * 2),
    ] {
        buffer[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
    }
    let units = substitute
        .into_iter()
        .chain(Some(0))
        .chain(display)
        .chain(Some(0));
    for (index, unit) in units.enumerate() {
        buffer[16 + index * 2..18 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    std::fs::create_dir(target)?;
    let result = (|| {
        let target = target
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: the target buffer is terminated. Returned handle is transferred
        // once into File. No subprocess, shell, registry mutation or credential.
        let raw = unsafe {
            CreateFileW(
                target.as_ptr(),
                GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_handle(raw) };
        let mut returned = 0;
        if unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                0x000900A4,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir(target);
    }
    result
}
