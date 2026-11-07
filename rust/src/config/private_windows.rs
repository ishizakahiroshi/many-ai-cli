//! Windows handle-based private ACL adapter. Native acceptance remains separate.
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{Authorization::*, *},
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
/// The string form of `sid`.
///
/// # Safety
/// `sid` must point to a valid SID for the duration of the call.
unsafe fn sid_string(sid: PSID) -> io::Result<String> {
    // SAFETY: the caller supplies a valid SID; the converted string is freed
    // by its guard after being copied.
    unsafe {
        let mut text = ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _memory = Local(text.cast());
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16(std::slice::from_raw_parts(text, len))
            .map_err(|_| io::Error::other("invalid SID encoding"))
    }
}
/// The SID of the user this process runs as.
fn current_user_sid() -> io::Result<String> {
    // SAFETY: Win32 allocations/handles are guarded and TOKEN_USER is aligned.
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
        sid_string(user.User.Sid)
    }
}
/// Owns a protected security descriptor for atomic private creation. Keep this
/// guard alive through CreateFile/CreateDirectory; its attributes do not inherit
/// ambient ACLs. Callers still own path/reparse containment checks.
pub struct PrivateSecurity {
    descriptor: Local,
}
impl PrivateSecurity {
    /// Whether the open `file` (READ_CONTROL access) is owned by this user, or
    /// by the administrators or the system, whom the private ACL lets in
    /// anyway. A file another user made in a shared folder is not (U54).
    pub fn trusted_owner(file: &std::fs::File) -> io::Result<bool> {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: the handle is live; the owner points into the descriptor,
        // which its guard frees after the owner has been read.
        let owner = unsafe {
            let mut owner = ptr::null_mut();
            let mut descriptor = ptr::null_mut();
            let status = GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor,
            );
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            let _descriptor = Local(descriptor);
            sid_string(owner)?
        };
        // Builtin Administrators and Local System.
        Ok(owner == current_user_sid()? || owner == "S-1-5-32-544" || owner == "S-1-5-18")
    }
    pub fn for_current_user(is_dir: bool) -> io::Result<Self> {
        let sid = current_user_sid()?;
        // SAFETY: the descriptor allocation is guarded.
        unsafe {
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
            Ok(Self {
                descriptor: Local(descriptor),
            })
        }
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.0,
            bInheritHandle: 0,
        }
    }
    /// Protect the already-open object, never a pathname resolved a second time.
    pub fn restrict_file(&self, file: &std::fs::File) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: descriptor and file stay alive; ACL points inside descriptor.
        unsafe {
            let mut present = 0;
            let mut defaulted = 0;
            let mut acl = ptr::null_mut();
            if GetSecurityDescriptorDacl(self.descriptor.0, &mut present, &mut acl, &mut defaulted)
                == 0
            {
                return Err(io::Error::last_os_error());
            }
            if present == 0 || acl.is_null() {
                return Err(io::Error::other("private DACL is missing"));
            }
            let status = SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                acl,
                ptr::null_mut(),
            );
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
        }
        Ok(())
    }
}
