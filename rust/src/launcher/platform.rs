use std::{
    io,
    path::PathBuf,
    process::{Command, Stdio},
};

pub fn browser_command(url: &str) -> (PathBuf, Vec<String>) {
    if cfg!(windows) {
        (
            "rundll32".into(),
            vec!["url.dll,FileProtocolHandler".into(), url.into()],
        )
    } else if cfg!(target_os = "macos") {
        ("open".into(), vec![url.into()])
    } else {
        ("xdg-open".into(), vec![url.into()])
    }
}
pub fn open_browser(url: &str) -> io::Result<()> {
    let (executable, args) = browser_command(url);
    let mut child = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .spawn()?;
    // Browser launch is a detached user action, not a contained Hub subprocess.
    // Reap the helper so successful repeated launches do not leave zombies.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
#[cfg(not(windows))]
pub fn configure_console_utf8() {}
#[cfg(windows)]
pub fn configure_console_utf8() {
    use windows_sys::Win32::System::Console::*;
    // SAFETY: standard console handles are borrowed, modes preserved by OR.
    unsafe {
        SetConsoleCP(65001);
        SetConsoleOutputCP(65001);
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let handle = GetStdHandle(which);
            let mut mode = 0;
            if GetConsoleMode(handle, &mut mode) != 0 {
                SetConsoleMode(
                    handle,
                    mode | ENABLE_PROCESSED_OUTPUT
                        | ENABLE_WRAP_AT_EOL_OUTPUT
                        | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                );
            }
        }
    }
}
#[cfg(unix)]
pub fn hostname() -> String {
    let mut bytes = [0u8; 256];
    // SAFETY: fixed output buffer with explicit bound, terminated locally.
    if unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
        return String::new();
    }
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
#[cfg(windows)]
pub fn hostname() -> String {
    use windows_sys::Win32::{
        Foundation::{ERROR_MORE_DATA, GetLastError},
        System::SystemInformation::{ComputerNamePhysicalDnsHostname, GetComputerNameExW},
    };
    // Go1.26.8 os/sys_windows.go uses the physical DNS hostname (including
    // cluster identity), growing from64 only when ERROR_MORE_DATA requests it.
    let mut len = 64u32;
    loop {
        let mut bytes = vec![0u16; len as usize];
        // SAFETY: buffer is live for its declared UTF-16 length; API returns
        // the actual length or the larger required size through len.
        if unsafe {
            GetComputerNameExW(
                ComputerNamePhysicalDnsHostname,
                bytes.as_mut_ptr(),
                &mut len,
            )
        } != 0
        {
            let text = &bytes[..len as usize];
            let end = text.iter().position(|c| *c == 0).unwrap_or(text.len());
            return String::from_utf16_lossy(&text[..end]);
        }
        if unsafe { GetLastError() } != ERROR_MORE_DATA || len as usize <= bytes.len() {
            return String::new();
        }
    }
}
#[cfg(unix)]
pub fn local_ipv4() -> Vec<std::net::Ipv4Addr> {
    let mut first = std::ptr::null_mut();
    // SAFETY: getifaddrs owns its linked allocation until freeifaddrs. Each
    // pointer is checked and cast only for the reported AF_INET family.
    unsafe {
        if libc::getifaddrs(&mut first) != 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut cursor = first;
        while let Some(entry) = cursor.as_ref() {
            if !entry.ifa_addr.is_null() && (*entry.ifa_addr).sa_family as i32 == libc::AF_INET {
                let address = &*(entry.ifa_addr as *const libc::sockaddr_in);
                let ip = std::net::Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes());
                if !ip.is_loopback() && !ip.is_link_local() {
                    out.push(ip);
                }
            }
            cursor = entry.ifa_next;
        }
        libc::freeifaddrs(first);
        out
    }
}
#[cfg(windows)]
pub fn local_ipv4() -> Vec<std::net::Ipv4Addr> {
    use windows_sys::Win32::{
        Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR},
        NetworkManagement::IpHelper::*,
        Networking::WinSock::*,
    };
    let mut size = 15000u32;
    for _ in 0..3 {
        // u64 storage supplies the required alignment for the native structures.
        let mut memory = vec![0u64; (size as usize).div_ceil(8)];
        let start = memory.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let result =
            unsafe { GetAdaptersAddresses(AF_INET as u32, 0, std::ptr::null(), start, &mut size) };
        if result == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if result != NO_ERROR {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut cursor = start;
        unsafe {
            while let Some(adapter) = cursor.as_ref() {
                let mut unicast = adapter.FirstUnicastAddress;
                while let Some(address) = unicast.as_ref() {
                    if !address.Address.lpSockaddr.is_null()
                        && (*address.Address.lpSockaddr).sa_family == AF_INET
                    {
                        let socket = &*(address.Address.lpSockaddr as *const SOCKADDR_IN);
                        let ip =
                            std::net::Ipv4Addr::from(socket.sin_addr.S_un.S_addr.to_ne_bytes());
                        if !ip.is_loopback() && !ip.is_link_local() {
                            out.push(ip);
                        }
                    }
                    unicast = address.Next;
                }
                cursor = adapter.Next;
            }
        }
        return out;
    }
    Vec::new()
}
pub fn startup_banner(version: &str) -> String {
    const LINES: [&str; 6] = [
        "███╗   ███╗ █████╗ ███╗   ██╗██╗   ██╗       █████╗ ██╗",
        "████╗ ████║██╔══██╗████╗  ██║╚██╗ ██╔╝      ██╔══██╗██║",
        "██╔████╔██║███████║██╔██╗ ██║ ╚████╔╝ █████╗███████║██║",
        "██║╚██╔╝██║██╔══██║██║╚██╗██║  ╚██╔╝  ╚════╝██╔══██║██║",
        "██║ ╚═╝ ██║██║  ██║██║ ╚████║   ██║         ██║  ██║██║",
        "╚═╝     ╚═╝╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝         ╚═╝  ╚═╝╚═╝",
    ];
    let mut out = String::new();
    for line in LINES {
        let mut current = "";
        for c in line.chars() {
            let next = match c {
                '█' => "\x1b[97m",
                '╗' | '╔' | '╝' | '╚' | '║' | '═' => "\x1b[38;5;226m",
                _ => "",
            };
            if next != current {
                if !current.is_empty() {
                    out.push_str("\x1b[0m");
                }
                out.push_str(next);
                current = next;
            }
            out.push(c);
        }
        if !current.is_empty() {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }
    let version = version.trim();
    let version = if version.is_empty() {
        "dev".into()
    } else if version == "dev" || version.starts_with('v') {
        version.into()
    } else {
        format!("v{version}")
    };
    out.push_str(&format!("\nConnection launcher (WSL / SSH) {version}\nRuntime: Windows\nGitHub: https://github.com/ishizakahiroshi/many-ai-cli\n\n"));
    out
}
pub fn close_behavior_notice(p: &super::Profile) -> String {
    let detail = if p.kind == "ssh" && p.mode == "tunnel" {
        "Closing this window disconnects only the SSH tunnel. The persistent Hub and sessions on the target keep running. Launch the connector again to reconnect and continue where you left off."
    } else {
        "Closing this window also stops the Hub started on the target, including any running sessions."
    };
    format!(
        "\x1b[1m\x1b[7m\x1b[38;5;208m WARNING: This window is the connection itself. Do not close it while in use. \x1b[0m\n{detail}\n"
    )
}
