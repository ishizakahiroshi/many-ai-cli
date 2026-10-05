use super::{LauncherStore, profile::SCHEMAS};
use crate::{
    config::Resource,
    proto::{
        time::format_rfc3339_nano,
        wire::{GoWire, Schema},
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io,
    time::{Duration, SystemTime},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActiveConnection {
    pub profile: String,
    pub pid: i64,
    pub hub_url: String,
    pub started_at: String,
}
impl Default for ActiveConnection {
    fn default() -> Self {
        Self {
            profile: String::new(),
            pid: 0,
            hub_url: String::new(),
            started_at: "0001-01-01T00:00:00Z".into(),
        }
    }
}
impl GoWire for ActiveConnection {
    const GO_TYPE: &'static str = "LauncherActiveConnection";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct ActiveFile {
    version: i64,
    #[serde(skip_serializing_if = "empty_connections")]
    connections: Option<Vec<ActiveConnection>>,
}
fn empty_connections(records: &Option<Vec<ActiveConnection>>) -> bool {
    records.as_ref().is_none_or(Vec::is_empty)
}
impl GoWire for ActiveFile {
    const GO_TYPE: &'static str = "LauncherActiveFile";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct ConnectLock {
    profile: String,
    pid: i64,
    started_at: String,
}
impl GoWire for ConnectLock {
    const GO_TYPE: &'static str = "LauncherConnectLock";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
fn now() -> io::Result<String> {
    format_rfc3339_nano(crate::proto::time::Timestamp::now()).map_err(io::Error::other)
}
fn current_pid() -> i64 {
    i64::from(std::process::id())
}
impl LauncherStore {
    fn load_active(&self) -> io::Result<ActiveFile> {
        let bytes = match self
            .dir
            .read(&self.name(Resource::LauncherActive)?, 8 * 1024 * 1024 + 1)
        {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ActiveFile {
                    version: 1,
                    connections: None,
                });
            }
            Err(error) => return Err(error),
        };
        let mut file = crate::proto::decode_wire::<ActiveFile>(&bytes).unwrap_or(ActiveFile {
            version: 1,
            connections: None,
        });
        if file.version == 0 {
            file.version = 1;
        }
        Ok(file)
    }
    fn save_active(&self, file: &ActiveFile) -> io::Result<()> {
        self.cleanup_stale_active_temps(SystemTime::now());
        self.dir.replace(
            &self.name(Resource::LauncherActive)?,
            &serde_json::to_vec_pretty(file).map_err(io::Error::other)?,
            0o600,
        )
    }
    pub fn active_snapshot(&self) -> io::Result<Vec<ActiveConnection>> {
        let _lock = self.lock(&self.name(Resource::LauncherActiveLock)?)?;
        Ok(self.load_active()?.connections.unwrap_or_default())
    }
    pub fn register_active(&self, profile: &str, hub_url: &str) -> io::Result<()> {
        self.register_record(ActiveConnection {
            profile: profile.into(),
            pid: current_pid(),
            hub_url: hub_url.into(),
            started_at: now()?,
        })
    }
    /// Also useful for synthetic registry interoperability tests. Never kills a PID.
    pub fn register_record(&self, record: ActiveConnection) -> io::Result<()> {
        let _lock = self.lock(&self.name(Resource::LauncherActiveLock)?)?;
        let mut file = self.load_active().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("read launcher active registry: {error}"),
            )
        })?;
        let records = file.connections.get_or_insert_with(Vec::new);
        records.retain(|c| c.profile != record.profile || c.pid != record.pid);
        records.push(record);
        self.save_active(&file).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("save launcher active registry: {error}"),
            )
        })
    }
    pub fn unregister_active(&self, profile: &str) -> io::Result<()> {
        self.remove_own(|c| c.profile == profile)
    }
    pub fn unregister_all(&self) -> io::Result<()> {
        self.remove_own(|_| true)
    }
    fn remove_own(&self, predicate: impl Fn(&ActiveConnection) -> bool) -> io::Result<()> {
        let _lock = self.lock(&self.name(Resource::LauncherActiveLock)?)?;
        let mut file = self.load_active()?;
        let records = file.connections.get_or_insert_with(Vec::new);
        let old = records.len();
        records.retain(|c| c.pid != current_pid() || !predicate(c));
        if old != records.len() {
            self.save_active(&file)?;
        }
        Ok(())
    }
    pub fn collect_active(
        &self,
        alive: impl Fn(i64) -> bool,
        probe: impl Fn(&str) -> bool,
    ) -> io::Result<Vec<ActiveConnection>> {
        let _lock = self.lock(&self.name(Resource::LauncherActiveLock)?)?;
        let mut file = self.load_active()?;
        let records = file.connections.get_or_insert_with(Vec::new);
        let old = records.len();
        records.retain(|c| alive(c.pid) && probe(&c.hub_url));
        let result = records.clone();
        if old != result.len() {
            self.save_active(&file)?;
        }
        Ok(result)
    }
    /// Probe outside the file lock; recheck identities before pruning so a
    /// concurrent replacement cannot be lost. Every writer takes the same lock.
    pub async fn active_pruned(&self) -> io::Result<Vec<ActiveConnection>> {
        let store = self.clone();
        let snapshot = tokio::task::spawn_blocking(move || store.active_snapshot())
            .await
            .map_err(io::Error::other)??;
        let mut stale = Vec::new();
        for record in &snapshot {
            if !crate::process::pid_alive(record.pid)
                || !super::network::probe_hub(&record.hub_url, Duration::from_millis(800)).await
            {
                stale.push(record.clone());
            }
        }
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = store.lock(&store.name(Resource::LauncherActiveLock)?)?;
            let mut file = store.load_active()?;
            let records = file.connections.get_or_insert_with(Vec::new);
            let old = records.len();
            records.retain(|record| !stale.contains(record));
            let result = records.clone();
            if old != result.len() {
                store.save_active(&file)?;
            }
            // Only return records verified in this call. New writers survive
            // persistence and will be verified on the next read.
            Ok(result
                .into_iter()
                .filter(|r| snapshot.contains(r))
                .collect())
        })
        .await
        .map_err(io::Error::other)?
    }
    pub async fn wait_for_active(
        &self,
        profile: &str,
        timeout: Duration,
        cancel: &crate::process::Cancellation,
    ) -> Option<ActiveConnection> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Ok(records) = self.active_pruned().await
                && let Some(record) = records.into_iter().find(|r| r.profile == profile)
            {
                return Some(record);
            }
            if tokio::time::Instant::now() > deadline {
                return None;
            }
            tokio::select! { _=cancel.cancelled()=>return None, _=tokio::time::sleep(Duration::from_millis(500))=>{} }
        }
    }
    pub fn startup_lock_name(profile: &str) -> String {
        let hash = Sha256::digest(profile.as_bytes());
        format!(
            "launcher-connect-{}.json",
            hash[..8]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    }
    pub fn try_acquire_connect(&self, profile: &str) -> io::Result<Option<ProfileConnectLock>> {
        self.try_acquire_connect_with(profile, crate::process::pid_alive)
    }
    pub fn try_acquire_connect_with(
        &self,
        profile: &str,
        alive: impl Fn(i64) -> bool,
    ) -> io::Result<Option<ProfileConnectLock>> {
        // Serializing creation AND stale replacement closes Go's small
        // empty-create/write race without broadening ownership.
        let _guard = self.lock(&self.name(Resource::LauncherActiveLock)?)?;
        let name = Self::startup_lock_name(profile);
        if let Ok(bytes) = self.dir.read(&name, 1024 * 1024) {
            if let Ok(lock) = crate::proto::decode_wire::<ConnectLock>(&bytes)
                && alive(lock.pid)
            {
                return Ok(None);
            }
            self.dir.remove_file(&name)?;
        } else {
            match self.dir.metadata(&name) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
                Ok(_) => return Err(io::Error::other("cannot read startup lock")),
            }
        }
        let started_at = now()?;
        let bytes = serde_json::to_vec(&ConnectLock {
            profile: profile.into(),
            pid: current_pid(),
            started_at: started_at.clone(),
        })
        .map_err(io::Error::other)?;
        self.dir.create_new(&name, &bytes, 0o600)?;
        Ok(Some(ProfileConnectLock {
            store: self.clone(),
            name: Some(name),
            started_at,
        }))
    }
    pub fn cleanup_stale_active_temps(&self, now: SystemTime) {
        // Enumeration supplies names only; metadata/removal are relative to the
        // held capability and cannot follow a substituted directory symlink.
        let Ok(entries) = std::fs::read_dir(self.dir.path()) else {
            return;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(String::from) else {
                continue;
            };
            if !name.starts_with("launcher-active-") || !name.ends_with(".json.tmp") {
                continue;
            }
            if let Ok(metadata) = self.dir.metadata(&name)
                && let Ok(modified) = metadata.modified()
                && now
                    .duration_since(modified)
                    .is_ok_and(|age| age >= Duration::from_secs(3600))
            {
                let _ = self.dir.remove_file(&name);
            }
        }
    }
}
pub struct ProfileConnectLock {
    store: LauncherStore,
    name: Option<String>,
    started_at: String,
}
impl ProfileConnectLock {
    pub fn release(&mut self) -> io::Result<()> {
        let Some(name) = &self.name else {
            return Ok(());
        };
        let _guard = self
            .store
            .lock(&self.store.name(Resource::LauncherActiveLock)?)?;
        match self.store.dir.read(name, 1024 * 1024) {
            Ok(bytes) => {
                // Unlike PID alone, timestamp plus PID prevents an old handle
                // from deleting a replacement owned by this same process.
                let ours = crate::proto::decode_wire::<ConnectLock>(&bytes).is_ok_and(|lock| {
                    lock.pid == current_pid() && lock.started_at == self.started_at
                });
                if ours {
                    self.store.dir.remove_file(name)?;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.name = None;
        Ok(())
    }
}
impl Drop for ProfileConnectLock {
    fn drop(&mut self) {
        let _ = self.release();
    }
}
