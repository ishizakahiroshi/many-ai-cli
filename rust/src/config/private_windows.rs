//! Windows private ACL and atomic replacement adapter. Native acceptance remains separate.
use std::{io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{Authorization::*, *},
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Local(*mut std::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let v: Vec<u16> = path.as_os_str().encode_wide().collect();
    if v.contains(&0) {
        return Err(io::Error::other("path contains null"));
    }
    Ok(v.into_iter().chain([0]).collect())
}

pub fn restrict(path: &Path, is_dir: bool) -> io::Result<()> {
    let path = wide(path)?;
    // SAFETY: all Win32-owned allocations and process handles are RAII managed,
    // TOKEN_USER storage is pointer-aligned and remains alive while the SID is read.
    unsafe {
        let mut raw = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(raw);
        let mut needed = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut needed);
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = &*(storage.as_ptr().cast::<TOKEN_USER>());
        let mut sid = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _sid_memory = Local(sid.cast());
        let mut len = 0;
        while *sid.add(len) != 0 {
            len += 1;
        }
        let sid = String::from_utf16(std::slice::from_raw_parts(sid, len))
            .map_err(|_| io::Error::other("invalid SID encoding"))?;
        let inheritance = if is_dir { "OICI" } else { "" };
        let sddl = format!(
            "D:P(A;{inheritance};FA;;;{sid})(A;{inheritance};FA;;;SY)(A;{inheritance};FA;;;BA)"
        );
        let sddl: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
        let mut descriptor = ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let _descriptor = Local(descriptor);
        if SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
pub fn replace(from: &Path, to: &Path) -> io::Result<()> {
    let from = wide(from)?;
    let to = wide(to)?;
    // SAFETY: nul-terminated UTF-16 paths remain live for the call.
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
