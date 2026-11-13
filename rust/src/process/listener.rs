//! Which process owns a loopback TCP listener. A connection that succeeds
//! proves only that something listens on the port: another program, including
//! one run by another OS user, can bind it before the process started for it
//! does (U31). The owner comes from the system's socket tables.
use std::io;

/// Whether `pid` owns every listener that a connection to 127.0.0.1:`port`
/// could reach, and there is at least one. Unsupported where the system gives
/// no owner information.
// No production caller since the Whisper runtime was removed; kept with its
// per-OS implementations and tests for the next managed local service.
#[allow(dead_code)]
pub(crate) fn loopback_listeners_owned_by(port: u16, pid: u32) -> io::Result<bool> {
    if port == 0 || pid == 0 {
        return Ok(false);
    }
    imp::owned_by(port, pid)
}

/// Whether the listener a connection to 127.0.0.1:`port` reaches runs as this
/// OS user. A client checks this before it sends the Hub token there: another
/// user who took the port would get the token, the terminal output and a way
/// to type into the session (U01, U02). On Windows and Linux every listener
/// that could take the connection must be this user's; on macOS, where other
/// users' sockets cannot be listed, one of this user's processes must hold it.
pub(crate) fn loopback_listener_is_own(port: u16) -> io::Result<bool> {
    if port == 0 {
        return Ok(false);
    }
    imp::listener_is_own(port)
}

/// Whether the loopback connection that comes from `peer_port` to this
/// process's `server_port` was opened by this OS user (U04: the Hub checks
/// who redeems a launch code).
pub(crate) fn loopback_peer_is_own(peer_port: u16, server_port: u16) -> io::Result<bool> {
    if peer_port == 0 || server_port == 0 {
        return Ok(false);
    }
    imp::peer_is_own(peer_port, server_port)
}

/// [`loopback_listener_is_own`] for callers that must not send anything when
/// the answer is unknown.
pub(crate) fn require_own_loopback_listener(port: u16) -> io::Result<()> {
    match loopback_listener_is_own(port) {
        Ok(true) => Ok(()),
        Ok(false) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("127.0.0.1:{port} is not held by this OS user's Hub; the token was not sent"),
        )),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("cannot tell who holds 127.0.0.1:{port}; the token was not sent ({error})"),
        )),
    }
}

/// The listening addresses a connection to 127.0.0.1 can reach.
#[cfg(any(windows, target_os = "linux", test))]
fn reaches_loopback_v4(address: std::net::Ipv4Addr) -> bool {
    address == std::net::Ipv4Addr::LOCALHOST || address.is_unspecified()
}
/// A dual-stack IPv6 listener can take IPv4 connections too.
#[cfg(any(windows, target_os = "linux", test))]
fn reaches_loopback_v6(address: std::net::Ipv6Addr) -> bool {
    address.is_unspecified() || address.to_ipv4_mapped().is_some_and(reaches_loopback_v4)
}
/// The local end of a loopback connection.
#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn is_loopback(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => v4.is_loopback(),
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// Reading Linux's /proc/net/tcp and tcp6. Built on every system in tests so
/// the parser is checked everywhere, not only on Linux.
#[cfg(any(target_os = "linux", test))]
mod proc_net {
    use super::{is_loopback, reaches_loopback_v4, reaches_loopback_v6};
    use std::{
        io,
        net::{IpAddr, Ipv4Addr, Ipv6Addr},
    };

    /// A table address. The kernel prints each 32-bit word of the address as
    /// read from memory.
    fn address(hex: &str) -> Option<IpAddr> {
        if hex.is_empty() || !hex.len().is_multiple_of(8) {
            return None;
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for word in 0..hex.len() / 8 {
            let value = u32::from_str_radix(hex.get(word * 8..word * 8 + 8)?, 16).ok()?;
            bytes.extend(value.to_ne_bytes());
        }
        match bytes.len() {
            4 => Some(IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(bytes).ok()?))),
            16 => Some(IpAddr::V6(Ipv6Addr::from(
                <[u8; 16]>::try_from(bytes).ok()?,
            ))),
            _ => None,
        }
    }

    /// Whether a table address reaches 127.0.0.1 as a listener.
    #[cfg(test)]
    pub(super) fn reaches(hex: &str) -> Option<bool> {
        Some(match address(hex)? {
            IpAddr::V4(v4) => reaches_loopback_v4(v4),
            IpAddr::V6(v6) => reaches_loopback_v6(v6),
        })
    }

    pub(super) struct Row {
        local: IpAddr,
        local_port: u16,
        remote_port: u16,
        listening: bool,
        pub(super) uid: u32,
        pub(super) inode: u64,
    }

    /// Every row of a table's `text`. A row it cannot read makes the whole
    /// table unreadable.
    fn rows(text: &str) -> io::Result<Vec<Row>> {
        let unreadable = || io::Error::other("unreadable socket table");
        let mut found = Vec::new();
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (Some(local), Some(remote), Some(state), Some(uid), Some(inode)) = (
                fields.get(1),
                fields.get(2),
                fields.get(3),
                fields.get(7),
                fields.get(9),
            ) else {
                return Err(unreadable());
            };
            let (local, local_port) = local.split_once(':').ok_or_else(unreadable)?;
            let (_, remote_port) = remote.split_once(':').ok_or_else(unreadable)?;
            found.push(Row {
                local: address(local).ok_or_else(unreadable)?,
                local_port: u16::from_str_radix(local_port, 16).map_err(|_| unreadable())?,
                remote_port: u16::from_str_radix(remote_port, 16).map_err(|_| unreadable())?,
                // 0A is TCP_LISTEN.
                listening: *state == "0A",
                uid: uid.parse().map_err(|_| unreadable())?,
                inode: inode.parse().map_err(|_| unreadable())?,
            });
        }
        Ok(found)
    }

    /// The listeners on `port` in a table's `text` that reach 127.0.0.1.
    pub(super) fn listeners(text: &str, port: u16) -> io::Result<Vec<Row>> {
        Ok(rows(text)?
            .into_iter()
            .filter(|row| {
                row.listening
                    && row.local_port == port
                    && match row.local {
                        IpAddr::V4(v4) => reaches_loopback_v4(v4),
                        IpAddr::V6(v6) => reaches_loopback_v6(v6),
                    }
            })
            .collect())
    }

    /// Socket inodes of the listeners on `port` that reach 127.0.0.1.
    pub(super) fn listening_inodes(text: &str, port: u16) -> io::Result<Vec<u64>> {
        Ok(listeners(text, port)?
            .into_iter()
            .map(|row| row.inode)
            .collect())
    }

    /// The client ends of loopback connections from `peer_port` to
    /// `server_port`.
    pub(super) fn peers(text: &str, peer_port: u16, server_port: u16) -> io::Result<Vec<Row>> {
        Ok(rows(text)?
            .into_iter()
            .filter(|row| {
                !row.listening
                    && row.local_port == peer_port
                    && row.remote_port == server_port
                    && is_loopback(row.local)
            })
            .collect())
    }
}

#[cfg(windows)]
mod imp {
    use super::{is_loopback, reaches_loopback_v4, reaches_loopback_v6};
    use std::{
        io,
        net::{IpAddr, Ipv4Addr, Ipv6Addr},
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_NOT_SUPPORTED, NO_ERROR},
        NetworkManagement::IpHelper::{
            GetExtendedTcpTable, MIB_TCP_STATE_LISTEN, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
            TCP_TABLE_CLASS, TCP_TABLE_OWNER_PID_CONNECTIONS, TCP_TABLE_OWNER_PID_LISTENER,
        },
        Networking::WinSock::{AF_INET, AF_INET6},
    };

    /// The owning PIDs of the rows of one address family and table class that
    /// `wanted` selects. The table is a row count followed by rows of `R`.
    fn owners<R>(
        family: u16,
        class: TCP_TABLE_CLASS,
        wanted: impl Fn(&R) -> Option<u32>,
    ) -> io::Result<Vec<u32>> {
        assert!(std::mem::align_of::<R>() <= std::mem::align_of::<u32>());
        let mut size = 0u32;
        // u32 units keep the rows aligned.
        let mut buffer: Vec<u32> = Vec::new();
        for _ in 0..4 {
            let table = if buffer.is_empty() {
                std::ptr::null_mut()
            } else {
                buffer.as_mut_ptr().cast()
            };
            // SAFETY: `table` is null or holds at least `size` writable bytes.
            let status =
                unsafe { GetExtendedTcpTable(table, &mut size, 0, u32::from(family), class, 0) };
            match status {
                NO_ERROR => {
                    let Some(&count) = buffer.first() else {
                        return Ok(Vec::new());
                    };
                    let count = count as usize;
                    if count > (buffer.len() - 1) * 4 / std::mem::size_of::<R>().max(1) {
                        return Err(io::Error::other("socket table is truncated"));
                    }
                    // SAFETY: the system wrote `count` rows after the count,
                    // within the buffer as checked above, aligned for R.
                    let rows = unsafe {
                        std::slice::from_raw_parts(buffer.as_ptr().add(1).cast::<R>(), count)
                    };
                    return Ok(rows.iter().filter_map(&wanted).collect());
                }
                ERROR_INSUFFICIENT_BUFFER => {
                    buffer = vec![0; (size as usize).div_ceil(4) + 1];
                    size = u32::try_from(buffer.len() * 4).map_err(io::Error::other)?;
                }
                status => return Err(io::Error::from_raw_os_error(status as i32)),
            }
        }
        Err(io::Error::other("socket table kept growing"))
    }

    /// Ports are kept in network byte order in the low 16 bits.
    fn port_of(value: u32) -> u16 {
        u16::from_be(value as u16)
    }

    /// Both families; a system without IPv6 has no IPv6 rows.
    fn both(
        class: TCP_TABLE_CLASS,
        v4: impl Fn(&MIB_TCPROW_OWNER_PID) -> bool,
        v6: impl Fn(&MIB_TCP6ROW_OWNER_PID) -> bool,
    ) -> io::Result<Vec<u32>> {
        let mut found = owners::<MIB_TCPROW_OWNER_PID>(AF_INET, class, |row| {
            v4(row).then_some(row.dwOwningPid)
        })?;
        match owners::<MIB_TCP6ROW_OWNER_PID>(AF_INET6, class, |row| {
            v6(row).then_some(row.dwOwningPid)
        }) {
            Ok(more) => found.extend(more),
            Err(error) if error.raw_os_error() == Some(ERROR_NOT_SUPPORTED as i32) => {}
            Err(error) => return Err(error),
        }
        Ok(found)
    }

    fn listener_pids(port: u16) -> io::Result<Vec<u32>> {
        both(
            TCP_TABLE_OWNER_PID_LISTENER,
            |row| {
                port_of(row.dwLocalPort) == port
                    && reaches_loopback_v4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()))
            },
            |row| {
                port_of(row.dwLocalPort) == port
                    && reaches_loopback_v6(Ipv6Addr::from(row.ucLocalAddr))
            },
        )
    }

    fn all_own(pids: Vec<u32>) -> io::Result<bool> {
        if pids.is_empty() {
            return Ok(false);
        }
        for pid in pids {
            if !crate::config::process_runs_as_current_user(pid)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn owned_by(port: u16, pid: u32) -> io::Result<bool> {
        let found = listener_pids(port)?;
        Ok(!found.is_empty() && found.iter().all(|owner| *owner == pid))
    }

    pub(super) fn listener_is_own(port: u16) -> io::Result<bool> {
        all_own(listener_pids(port)?)
    }

    pub(super) fn peer_is_own(peer_port: u16, server_port: u16) -> io::Result<bool> {
        let pids = both(
            TCP_TABLE_OWNER_PID_CONNECTIONS,
            |row| {
                row.dwState != MIB_TCP_STATE_LISTEN as u32
                    && port_of(row.dwLocalPort) == peer_port
                    && port_of(row.dwRemotePort) == server_port
                    && is_loopback(IpAddr::V4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes())))
            },
            |row| {
                row.dwState != MIB_TCP_STATE_LISTEN as u32
                    && port_of(row.dwLocalPort) == peer_port
                    && port_of(row.dwRemotePort) == server_port
                    && is_loopback(IpAddr::V6(Ipv6Addr::from(row.ucLocalAddr)))
            },
        )?;
        all_own(pids)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::{collections::HashSet, io};

    fn table(path: &str) -> io::Result<String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(text),
            // No IPv6 stack: nothing can listen there.
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error),
        }
    }

    fn inodes(path: &str, port: u16) -> io::Result<Vec<u64>> {
        let text = table(path)?;
        if text.is_empty() {
            return Ok(Vec::new());
        }
        super::proc_net::listening_inodes(&text, port)
    }

    /// The uids of `select`'s rows in both tables.
    fn uids(
        select: impl Fn(&str) -> io::Result<Vec<super::proc_net::Row>>,
    ) -> io::Result<Vec<u32>> {
        let mut found = Vec::new();
        for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
            let text = table(path)?;
            if !text.is_empty() {
                found.extend(select(&text)?.into_iter().map(|row| row.uid));
            }
        }
        Ok(found)
    }

    fn all_own(uids: Vec<u32>) -> bool {
        // SAFETY: geteuid has no preconditions.
        let me = unsafe { libc::geteuid() };
        !uids.is_empty() && uids.iter().all(|uid| *uid == me)
    }

    pub(super) fn owned_by(port: u16, pid: u32) -> io::Result<bool> {
        let mut wanted = inodes("/proc/net/tcp", port)?;
        wanted.extend(inodes("/proc/net/tcp6", port)?);
        if wanted.is_empty() {
            return Ok(false);
        }
        let entries = match std::fs::read_dir(format!("/proc/{pid}/fd")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let mut held = HashSet::new();
        for entry in entries {
            // A descriptor closed while listing is simply not held.
            let Ok(target) = std::fs::read_link(entry?.path()) else {
                continue;
            };
            if let Some(inode) = target
                .to_str()
                .and_then(|target| target.strip_prefix("socket:["))
                .and_then(|target| target.strip_suffix(']'))
                .and_then(|inode| inode.parse::<u64>().ok())
            {
                held.insert(inode);
            }
        }
        Ok(wanted.iter().all(|inode| held.contains(inode)))
    }

    pub(super) fn listener_is_own(port: u16) -> io::Result<bool> {
        Ok(all_own(uids(|text| {
            super::proc_net::listeners(text, port)
        })?))
    }

    pub(super) fn peer_is_own(peer_port: u16, server_port: u16) -> io::Result<bool> {
        Ok(all_own(uids(|text| {
            super::proc_net::peers(text, peer_port, server_port)
        })?))
    }
}

/// macOS lists only this user's processes' sockets, through libproc. The
/// layout below is `struct socket_fdinfo` of XNU's sys/proc_info.h on 64-bit
/// systems; only the fields read here are named.
#[cfg(target_os = "macos")]
mod imp {
    use super::is_loopback;
    use std::{
        ffi::{c_int, c_void},
        io,
        net::{IpAddr, Ipv4Addr, Ipv6Addr},
    };

    const PROC_UID_ONLY: u32 = 4;
    const PROC_PIDLISTFDS: c_int = 1;
    const PROC_PIDFDSOCKETINFO: c_int = 3;
    const PROX_FDTYPE_SOCKET: u32 = 2;
    const SOCKINFO_TCP: i32 = 2;
    const TSI_S_LISTEN: i32 = 1;
    const INI_IPV4: u8 = 0x1;
    const INI_IPV6: u8 = 0x2;
    // Offsets in socket_fdinfo: proc_fileinfo is 24 bytes, then socket_info,
    // whose soi_kind is at 232 and soi_proto at 240. In soi_proto, tcp_sockinfo
    // starts with in_sockinfo: insi_fport 0, insi_lport 4, insi_vflag 24,
    // insi_faddr 32, insi_laddr 48; tcpsi_state follows it at 80.
    const KIND: usize = 24 + 232;
    const PROTO: usize = 24 + 240;
    const FPORT: usize = PROTO;
    const LPORT: usize = PROTO + 4;
    const VFLAG: usize = PROTO + 24;
    const LADDR: usize = PROTO + 48;
    const STATE: usize = PROTO + 80;

    struct Tcp {
        local: IpAddr,
        local_port: u16,
        remote_port: u16,
        listening: bool,
    }

    fn i32_at(buffer: &[u8], at: usize) -> Option<i32> {
        Some(i32::from_ne_bytes(buffer.get(at..at + 4)?.try_into().ok()?))
    }
    /// insi_lport and insi_fport hold the port in network order.
    fn port_at(buffer: &[u8], at: usize) -> Option<u16> {
        Some(u16::from_be(i32_at(buffer, at)? as u16))
    }

    fn own_pids() -> io::Result<Vec<c_int>> {
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };
        let mut pids: Vec<c_int> = vec![0; 1024];
        for _ in 0..4 {
            let bytes = c_int::try_from(pids.len() * 4).map_err(io::Error::other)?;
            // SAFETY: the buffer holds `bytes` writable bytes.
            let used =
                unsafe { libc::proc_listpids(PROC_UID_ONLY, uid, pids.as_mut_ptr().cast(), bytes) };
            if used < 0 {
                return Err(io::Error::last_os_error());
            }
            let count = used as usize / 4;
            if count < pids.len() {
                pids.truncate(count);
                pids.retain(|pid| *pid > 0);
                return Ok(pids);
            }
            pids = vec![0; pids.len() * 4];
        }
        Err(io::Error::other("process list kept growing"))
    }

    /// The TCP sockets `pid` holds. A process that ended meanwhile holds none.
    fn sockets(pid: c_int) -> Vec<Tcp> {
        let mut fds: Vec<libc::proc_fdinfo> = Vec::new();
        let mut capacity = 64usize;
        loop {
            fds.resize(
                capacity,
                libc::proc_fdinfo {
                    proc_fd: 0,
                    proc_fdtype: 0,
                },
            );
            let bytes = (capacity * std::mem::size_of::<libc::proc_fdinfo>()) as c_int;
            // SAFETY: the buffer holds `bytes` writable bytes of proc_fdinfo.
            let used = unsafe {
                libc::proc_pidinfo(pid, PROC_PIDLISTFDS, 0, fds.as_mut_ptr().cast(), bytes)
            };
            if used <= 0 {
                return Vec::new();
            }
            let count = used as usize / std::mem::size_of::<libc::proc_fdinfo>();
            if count < capacity || capacity >= 1 << 16 {
                fds.truncate(count.min(capacity));
                break;
            }
            capacity *= 4;
        }
        let mut found = Vec::new();
        // u64 units keep the structure aligned; 2 KiB is more than its size.
        let mut info = vec![0u64; 256];
        for fd in fds.iter().filter(|fd| fd.proc_fdtype == PROX_FDTYPE_SOCKET) {
            // SAFETY: the buffer holds 2048 writable bytes.
            let used = unsafe {
                libc::proc_pidfdinfo(
                    pid,
                    fd.proc_fd,
                    PROC_PIDFDSOCKETINFO,
                    info.as_mut_ptr().cast::<c_void>(),
                    2048,
                )
            };
            if used <= 0 {
                continue;
            }
            // SAFETY: the buffer is 2048 initialised bytes.
            let bytes = unsafe { std::slice::from_raw_parts(info.as_ptr().cast::<u8>(), 2048) };
            let bytes = &bytes[..(used as usize).min(2048)];
            if i32_at(bytes, KIND) != Some(SOCKINFO_TCP) {
                continue;
            }
            let (Some(local_port), Some(remote_port), Some(state), Some(&vflag)) = (
                port_at(bytes, LPORT),
                port_at(bytes, FPORT),
                i32_at(bytes, STATE),
                bytes.get(VFLAG),
            ) else {
                continue;
            };
            let local = if vflag & INI_IPV6 != 0 {
                let Some(octets) = bytes.get(LADDR..LADDR + 16) else {
                    continue;
                };
                IpAddr::V6(Ipv6Addr::from(
                    <[u8; 16]>::try_from(octets).unwrap_or([0; 16]),
                ))
            } else if vflag & INI_IPV4 != 0 {
                // in4in6_addr: 12 bytes of padding, then the IPv4 address.
                let Some(octets) = bytes.get(LADDR + 12..LADDR + 16) else {
                    continue;
                };
                IpAddr::V4(Ipv4Addr::from(
                    <[u8; 4]>::try_from(octets).unwrap_or([0; 4]),
                ))
            } else {
                continue;
            };
            found.push(Tcp {
                local,
                local_port,
                remote_port,
                listening: state == TSI_S_LISTEN,
            });
        }
        found
    }

    fn any_own(wanted: impl Fn(&Tcp) -> bool) -> io::Result<bool> {
        for pid in own_pids()? {
            if sockets(pid).iter().any(&wanted) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn owned_by(_: u16, _: u32) -> io::Result<bool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "listener owners are unavailable on this system",
        ))
    }

    /// The Hub binds 127.0.0.1 without SO_REUSEPORT, so while one of this
    /// user's processes listens there no other user's listener takes the
    /// connection.
    pub(super) fn listener_is_own(port: u16) -> io::Result<bool> {
        any_own(|socket| {
            socket.listening
                && socket.local_port == port
                && (is_loopback(socket.local) || socket.local.is_unspecified())
        })
    }

    pub(super) fn peer_is_own(peer_port: u16, server_port: u16) -> io::Result<bool> {
        any_own(|socket| {
            !socket.listening
                && socket.local_port == peer_port
                && socket.remote_port == server_port
                && is_loopback(socket.local)
        })
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    fn unsupported() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "socket owners are unavailable on this system",
        )
    }
    pub(super) fn owned_by(_: u16, _: u32) -> std::io::Result<bool> {
        Err(unsupported())
    }
    pub(super) fn listener_is_own(_: u16) -> std::io::Result<bool> {
        Err(unsupported())
    }
    pub(super) fn peer_is_own(_: u16, _: u16) -> std::io::Result<bool> {
        Err(unsupported())
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::*;
    use crate::process::wrapper_startup::recovery_identity::test_support::Bystander;

    #[test]
    fn audit_u31_the_owner_is_the_process_that_holds_the_loopback_listener() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(loopback_listeners_owned_by(port, std::process::id()).unwrap());
        let bystander = Bystander::spawn();
        assert!(!loopback_listeners_owned_by(port, bystander.pid()).unwrap());
        drop(listener);
        assert!(!loopback_listeners_owned_by(port, std::process::id()).unwrap());
        assert!(!loopback_listeners_owned_by(port, 0).unwrap());
    }

    #[test]
    fn audit_u31_wildcard_and_dual_stack_listeners_count_as_reachable() {
        use std::net::{Ipv4Addr, Ipv6Addr};
        assert!(reaches_loopback_v4(Ipv4Addr::LOCALHOST));
        assert!(reaches_loopback_v4(Ipv4Addr::UNSPECIFIED));
        assert!(!reaches_loopback_v4(Ipv4Addr::new(127, 0, 0, 2)));
        assert!(reaches_loopback_v6(Ipv6Addr::UNSPECIFIED));
        assert!(reaches_loopback_v6(Ipv4Addr::LOCALHOST.to_ipv6_mapped()));
        assert!(!reaches_loopback_v6(Ipv6Addr::LOCALHOST));
    }

    #[test]
    fn audit_u01_a_listener_and_a_connection_of_this_user_are_its_own() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(loopback_listener_is_own(port).unwrap());
        require_own_loopback_listener(port).unwrap();
        let client = std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
        let (_server, _) = listener.accept().unwrap();
        let peer_port = client.local_addr().unwrap().port();
        assert!(loopback_peer_is_own(peer_port, port).unwrap());
        // No connection comes from the next port: nobody is vouched for.
        assert!(!loopback_peer_is_own(peer_port.wrapping_add(1).max(1), port).unwrap());
        drop(client);
        drop(listener);
        // Nobody listens any more: nothing is sent. Another test's child that
        // is forked but not yet exec'd can hold a copy of the listener for a
        // moment on Linux, so give it that moment.
        let gone = (0..100).any(|_| {
            let gone = !loopback_listener_is_own(port).unwrap();
            if !gone {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            gone
        });
        assert!(gone);
        let error = require_own_loopback_listener(port).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("the token was not sent"));
        assert!(!loopback_listener_is_own(0).unwrap());
        assert!(!loopback_peer_is_own(0, port).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn audit_u01_a_process_another_user_runs_is_not_this_users() {
        // The System process (PID 4) and the idle process are never this
        // user's, and querying them must not fail.
        assert!(!crate::config::process_runs_as_current_user(4).unwrap());
        assert!(!crate::config::process_runs_as_current_user(0).unwrap());
        assert!(crate::config::process_runs_as_current_user(std::process::id()).unwrap());
    }
}

#[cfg(test)]
mod proc_net_tests {
    use super::proc_net::{listeners, listening_inodes, peers, reaches};

    /// An address word as the kernel prints it: the bytes read as one u32.
    fn word(bytes: [u8; 4]) -> String {
        format!("{:08X}", u32::from_ne_bytes(bytes))
    }
    fn v6(octets: [u8; 16]) -> String {
        octets
            .chunks(4)
            .map(|chunk| word(chunk.try_into().unwrap()))
            .collect()
    }
    fn row(address: &str, port: u16, state: &str, inode: u64) -> String {
        connection(address, port, 0, state, 1000, inode)
    }
    fn connection(
        address: &str,
        port: u16,
        remote_port: u16,
        state: &str,
        uid: u32,
        inode: u64,
    ) -> String {
        format!(
            "   0: {address}:{port:04X} {}:{remote_port:04X} {state} 00000000:00000000 00:00000000 00000000  {uid}        0 {inode} 1 0000000000000000 100 0 0 10 0",
            word([127, 0, 0, 1])
        )
    }
    const HEADER: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode";

    #[test]
    fn audit_u31_table_addresses_are_read_word_by_word() {
        assert_eq!(reaches(&word([127, 0, 0, 1])), Some(true));
        assert_eq!(reaches(&word([0, 0, 0, 0])), Some(true));
        assert_eq!(reaches(&word([127, 0, 0, 2])), Some(false));
        let mapped = std::net::Ipv4Addr::LOCALHOST.to_ipv6_mapped().octets();
        assert_eq!(reaches(&v6(mapped)), Some(true));
        assert_eq!(reaches(&v6([0; 16])), Some(true));
        assert_eq!(
            reaches(&v6(std::net::Ipv6Addr::LOCALHOST.octets())),
            Some(false)
        );
        assert_eq!(reaches("XYZ"), None);
        assert_eq!(reaches(""), None);
    }

    #[test]
    fn audit_u31_only_listeners_on_the_port_that_reach_loopback_are_taken() {
        let loopback = word([127, 0, 0, 1]);
        let table = [
            HEADER.to_owned(),
            row(&loopback, 8080, "0A", 111),
            row(&word([0, 0, 0, 0]), 8080, "0A", 222),
            row(&word([127, 0, 0, 2]), 8080, "0A", 333),
            row(&loopback, 8081, "0A", 444),
            // An established connection on the port is not a listener.
            row(&loopback, 8080, "01", 555),
        ]
        .join("\n");
        assert_eq!(listening_inodes(&table, 8080).unwrap(), [111, 222]);
        let table6 = [HEADER.to_owned(), row(&v6([0; 16]), 8080, "0A", 666)].join("\n");
        assert_eq!(listening_inodes(&table6, 8080).unwrap(), [666]);
        assert!(listening_inodes(HEADER, 8080).unwrap().is_empty());
        assert!(listening_inodes(&format!("{HEADER}\n   0: garbage"), 8080).is_err());
    }

    #[test]
    fn audit_u01_listener_and_peer_rows_carry_their_owner() {
        let loopback = word([127, 0, 0, 1]);
        let table = [
            HEADER.to_owned(),
            connection(&loopback, 47777, 0, "0A", 1001, 1),
            // Another user's listener on the wildcard address of the port.
            connection(&word([0, 0, 0, 0]), 47777, 0, "0A", 1002, 2),
            // The client end of a connection to the Hub, and the Hub's end.
            connection(&loopback, 51000, 47777, "01", 1003, 3),
            connection(&loopback, 47777, 51000, "01", 1001, 4),
            // A connection from another address is not a loopback peer.
            connection(&word([10, 0, 0, 5]), 51000, 47777, "01", 1004, 5),
        ]
        .join("\n");
        let owners: Vec<u32> = listeners(&table, 47777)
            .unwrap()
            .iter()
            .map(|row| row.uid)
            .collect();
        assert_eq!(owners, [1001, 1002]);
        let owners: Vec<u32> = peers(&table, 51000, 47777)
            .unwrap()
            .iter()
            .map(|row| row.uid)
            .collect();
        assert_eq!(owners, [1003]);
        assert!(peers(&table, 51001, 47777).unwrap().is_empty());
        assert!(peers(&format!("{HEADER}\n   0: garbage"), 1, 2).is_err());
    }
}
