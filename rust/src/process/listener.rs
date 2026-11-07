//! Which process owns a loopback TCP listener. A connection that succeeds
//! proves only that something listens on the port: another program, including
//! one run by another OS user, can bind it before the process started for it
//! does (U31). The owner comes from the system's socket tables.
use std::io;

/// Whether `pid` owns every listener that a connection to 127.0.0.1:`port`
/// could reach, and there is at least one. Unsupported where the system gives
/// no owner information.
pub(crate) fn loopback_listeners_owned_by(port: u16, pid: u32) -> io::Result<bool> {
    if port == 0 || pid == 0 {
        return Ok(false);
    }
    imp::owned_by(port, pid)
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

/// Reading Linux's /proc/net/tcp and tcp6. Built on every system in tests so
/// the parser is checked everywhere, not only on Linux.
#[cfg(any(target_os = "linux", test))]
mod proc_net {
    use super::{reaches_loopback_v4, reaches_loopback_v6};
    use std::{
        io,
        net::{Ipv4Addr, Ipv6Addr},
    };

    /// Whether a table address reaches 127.0.0.1. The kernel prints each
    /// 32-bit word of the address as read from memory.
    pub(super) fn reaches(hex: &str) -> Option<bool> {
        if hex.is_empty() || !hex.len().is_multiple_of(8) {
            return None;
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for word in 0..hex.len() / 8 {
            let value = u32::from_str_radix(hex.get(word * 8..word * 8 + 8)?, 16).ok()?;
            bytes.extend(value.to_ne_bytes());
        }
        match bytes.len() {
            4 => Some(reaches_loopback_v4(Ipv4Addr::from(
                <[u8; 4]>::try_from(bytes).ok()?,
            ))),
            16 => Some(reaches_loopback_v6(Ipv6Addr::from(
                <[u8; 16]>::try_from(bytes).ok()?,
            ))),
            _ => None,
        }
    }

    /// Socket inodes of the listeners on `port` in a table's `text` that
    /// reach 127.0.0.1. A row it cannot read makes the whole table unreadable.
    pub(super) fn listening_inodes(text: &str, port: u16) -> io::Result<Vec<u64>> {
        let unreadable = || io::Error::other("unreadable listener table");
        let mut found = Vec::new();
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (Some(local), Some(state), Some(inode)) =
                (fields.get(1), fields.get(3), fields.get(9))
            else {
                return Err(unreadable());
            };
            // 0A is TCP_LISTEN.
            if *state != "0A" {
                continue;
            }
            let (address, local_port) = local.split_once(':').ok_or_else(unreadable)?;
            if u16::from_str_radix(local_port, 16).map_err(|_| unreadable())? != port {
                continue;
            }
            if reaches(address).ok_or_else(unreadable)? {
                found.push(inode.parse().map_err(|_| unreadable())?);
            }
        }
        Ok(found)
    }
}

#[cfg(windows)]
mod imp {
    use super::{reaches_loopback_v4, reaches_loopback_v6};
    use std::{
        io,
        net::{Ipv4Addr, Ipv6Addr},
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_NOT_SUPPORTED, NO_ERROR},
        NetworkManagement::IpHelper::{
            GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
            TCP_TABLE_OWNER_PID_LISTENER,
        },
        Networking::WinSock::{AF_INET, AF_INET6},
    };

    /// The owning PIDs of the listeners of one address family that `wanted`
    /// selects. The table is a row count followed by rows of `R`.
    fn owners<R>(family: u16, wanted: impl Fn(&R) -> Option<u32>) -> io::Result<Vec<u32>> {
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
            let status = unsafe {
                GetExtendedTcpTable(
                    table,
                    &mut size,
                    0,
                    u32::from(family),
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            match status {
                NO_ERROR => {
                    let Some(&count) = buffer.first() else {
                        return Ok(Vec::new());
                    };
                    let count = count as usize;
                    if count > (buffer.len() - 1) * 4 / std::mem::size_of::<R>().max(1) {
                        return Err(io::Error::other("listener table is truncated"));
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
        Err(io::Error::other("listener table kept growing"))
    }

    pub(super) fn owned_by(port: u16, pid: u32) -> io::Result<bool> {
        // Ports are kept in network byte order in the low 16 bits.
        let on_port = |value: u32| u16::from_be(value as u16) == port;
        let mut found = owners::<MIB_TCPROW_OWNER_PID>(AF_INET, |row| {
            (on_port(row.dwLocalPort)
                && reaches_loopback_v4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes())))
            .then_some(row.dwOwningPid)
        })?;
        match owners::<MIB_TCP6ROW_OWNER_PID>(AF_INET6, |row| {
            (on_port(row.dwLocalPort) && reaches_loopback_v6(Ipv6Addr::from(row.ucLocalAddr)))
                .then_some(row.dwOwningPid)
        }) {
            Ok(more) => found.extend(more),
            // No IPv6 stack: nothing can listen there.
            Err(error) if error.raw_os_error() == Some(ERROR_NOT_SUPPORTED as i32) => {}
            Err(error) => return Err(error),
        }
        Ok(!found.is_empty() && found.iter().all(|owner| *owner == pid))
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::{collections::HashSet, io};

    fn inodes(table: &str, port: u16) -> io::Result<Vec<u64>> {
        match std::fs::read_to_string(table) {
            Ok(text) => super::proc_net::listening_inodes(&text, port),
            // No IPv6 stack: nothing can listen there.
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error),
        }
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
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub(super) fn owned_by(_: u16, _: u32) -> std::io::Result<bool> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "listener owners are unavailable on this system",
        ))
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
}

#[cfg(test)]
mod proc_net_tests {
    use super::proc_net::{listening_inodes, reaches};

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
        format!(
            "   0: {address}:{port:04X} 00000000:0000 {state} 00000000:00000000 00:00000000 00000000  1000        0 {inode} 1 0000000000000000 100 0 0 10 0"
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
}
