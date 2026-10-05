use super::super::{MainContext, context::environment_value};
use crate::{
    hub::{router::InfoFactory, session_routes::InfoContext},
    profile::store::ProviderRegistryStore,
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    sync::{Arc, Mutex},
};
fn binary_hash(path: &std::path::Path) -> io::Result<(String, u64, std::time::SystemTime)> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok((
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        metadata.len(),
        metadata.modified()?,
    ))
}
/// Fixed Go binaryGuard: stat first, rehash only when size/mtime changed.
struct BinaryGuard {
    path: std::path::PathBuf,
    original: String,
    observed: Mutex<Option<(String, u64, std::time::SystemTime)>>,
}
impl BinaryGuard {
    fn new(path: std::path::PathBuf) -> Self {
        let observed = binary_hash(&path).ok();
        let original = observed
            .as_ref()
            .map(|value| value.0.clone())
            .unwrap_or_default();
        Self {
            path,
            original,
            observed: Mutex::new(observed),
        }
    }
    fn stale(&self) -> bool {
        if self.original.is_empty() {
            return false;
        }
        let Ok(metadata) = std::fs::metadata(&self.path) else {
            return false;
        };
        let Ok(modified) = metadata.modified() else {
            return false;
        };
        let mut observed = self.observed.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((hash, size, time)) = observed.as_ref()
            && *size == metadata.len()
            && *time == modified
        {
            return hash != &self.original;
        }
        let Ok(current) = binary_hash(&self.path) else {
            return false;
        };
        let stale = current.0 != self.original;
        // Source records prehash stat, so a racing replacement is checked again.
        *observed = Some((current.0, metadata.len(), modified));
        stale
    }
}
pub(super) fn factory(
    context: &MainContext,
    registry: Arc<ProviderRegistryStore>,
    hints: Arc<crate::hub::runtime_routes::NetHints>,
) -> io::Result<Arc<InfoFactory>> {
    let guard = BinaryGuard::new(context.executable.clone());
    let cwd = context.cwd.to_string_lossy().into_owned();
    let environment = context.environment.clone();
    let ssh = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|key| environment_value(&environment, key).is_some_and(|value| !value.is_empty()));
    let host = environment_value(&environment, "MANY_AI_CLI_HOST_LABEL")
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            environment_value(&environment, "SSH_CONNECTION")
                .and_then(|value| value.split_whitespace().nth(2))
                .map(str::to_owned)
        })
        .unwrap_or_else(local_ip);
    let src_hash = String::from_utf8(
        crate::assets::web_asset(".src-hash")
            .ok_or_else(|| io::Error::other("embedded asset source hash missing"))?
            .to_vec(),
    )
    .map_err(io::Error::other)?;
    let runtime_mode = if cfg!(windows) {
        "windows-native"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    // Configuration/env observations come from the same bootstrap snapshot as
    // child launches. This factory never looks up a second user home or binary.
    Ok(Arc::new(move |http, config| {
        let registry = registry.snapshot().map_err(|_| {
            crate::hub::http::Response::error(
                500,
                "provider_registry_unavailable",
                "provider registry unavailable",
            )
        })?;
        let stale = guard.stale();
        let hint = hints.snapshot();
        let context = InfoContext {
            cwd: &cwd,
            version: env!("MANY_AI_BUILD_VERSION"),
            git_commit: env!("MANY_AI_BUILD_COMMIT"),
            build_time: env!("MANY_AI_BUILD_TIME"),
            binary_sha256: &guard.original,
            binary_stale: stale,
            web_src_hash: &src_hash,
            web_dist_fresh: true, // build.rs validates the embedded source/dist pair
            runtime_mode,
            user_name_fallback: environment_value(&environment, "USERNAME")
                .or_else(|| environment_value(&environment, "USER"))
                .unwrap_or(""),
            ssh,
            host_ip: &host,
            env_kind_override: environment_value(&environment, "MANY_AI_CLI_ENV_KIND")
                .unwrap_or(""),
            net_hint_ssh: hint.ssh,
            net_hint_host: &hint.host_label,
            net_hint_env_kind: &hint.env_kind,
            registry: Some(registry.registry.as_ref()),
        };
        Ok(http.handle_info_authenticated(config, &context))
    }))
}
fn local_ip() -> String {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            NetworkManagement::IpHelper::{
                GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
                GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
            },
            Networking::WinSock::{AF_INET, SOCKADDR_IN},
        };
        let mut size = 15 * 1024u32;
        for _ in 0..3 {
            let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
            let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
            let status = GetAdaptersAddresses(
                AF_INET as u32,
                GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER,
                std::ptr::null(),
                first,
                &mut size,
            );
            if status == windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW {
                continue;
            }
            if status != 0 {
                break;
            }
            let mut adapter = first;
            while !adapter.is_null() {
                let mut address = (*adapter).FirstUnicastAddress;
                while !address.is_null() {
                    let socket = (*address).Address.lpSockaddr;
                    if !socket.is_null() && (*socket).sa_family == AF_INET {
                        let address = &*socket.cast::<SOCKADDR_IN>();
                        let ip =
                            std::net::Ipv4Addr::from(address.sin_addr.S_un.S_addr.to_ne_bytes());
                        if !ip.is_loopback() && !ip.is_link_local() {
                            return ip.to_string();
                        }
                    }
                    address = (*address).Next;
                }
                adapter = (*adapter).Next;
            }
            break;
        }
    }
    #[cfg(unix)]
    unsafe {
        let mut first = std::ptr::null_mut();
        if libc::getifaddrs(&mut first) == 0 {
            let mut item = first;
            let mut result = String::new();
            while !item.is_null() {
                let address = (*item).ifa_addr;
                if !address.is_null() && i32::from((*address).sa_family) == libc::AF_INET {
                    let address = &*address.cast::<libc::sockaddr_in>();
                    let ip = std::net::Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes());
                    if !ip.is_loopback() && !ip.is_link_local() {
                        result = ip.to_string();
                        break;
                    }
                }
                item = (*item).ifa_next;
            }
            libc::freeifaddrs(first);
            return result;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_binary_guard_caches_unchanged_metadata_and_missing_file_is_not_stale() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("binary");
        std::fs::write(&path, b"initial").unwrap();
        let guard = BinaryGuard::new(path.clone());
        assert!(!guard.stale());
        std::fs::write(&path, b"changed length").unwrap();
        assert!(guard.stale());
        assert!(guard.stale());
        std::fs::remove_file(&path).unwrap();
        assert!(!guard.stale());
        let unavailable = BinaryGuard::new(path);
        assert!(unavailable.original.is_empty());
        assert!(!unavailable.stale());
    }
}
