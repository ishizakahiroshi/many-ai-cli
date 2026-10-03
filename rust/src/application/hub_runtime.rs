//! Source: internal/hubruntime/runtime.go. All filesystem operations retain the
//! explicit runtime directory capability; no process-global home lookup occurs.
use crate::proto::time::Timestamp;
use crate::{
    config::RuntimePaths,
    files::safe_fs::Dir,
    proto::wire::{Field, GoWire, Schema},
};
use serde::{Deserialize, Serialize};
use std::{fs::File, io, sync::Arc, time::Duration};
const FILE: &str = "hub-runtime.json";
const LOCK: &str = "hub-runtime.json.lock";
const MAX_BYTES: usize = 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeData {
    pub version: i64,
    pub pid: i64,
    pub port: i64,
    pub started_at: String,
}
impl Default for RuntimeData {
    fn default() -> Self {
        Self {
            version: 0,
            pid: 0,
            port: 0,
            started_at: "0001-01-01T00:00:00Z".into(),
        }
    }
}
impl GoWire for RuntimeData {
    const GO_TYPE: &'static str = "HubRuntimeData";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "HubRuntimeData",
        fields: &[
            Field {
                name: "version",
                kind: "int",
            },
            Field {
                name: "pid",
                kind: "int",
            },
            Field {
                name: "port",
                kind: "int",
            },
            Field {
                name: "started_at",
                kind: "time.Time",
            },
        ],
    }];
}
#[derive(Clone)]
pub struct RuntimeLedger {
    dir: Arc<Dir>,
    trial_port: Option<u16>,
}
impl RuntimeLedger {
    pub fn open(paths: &RuntimePaths) -> io::Result<Self> {
        Ok(Self {
            dir: Arc::new(Dir::open_or_create_private(paths.root())?),
            trial_port: paths.is_trial().then(|| paths.port()),
        })
    }
    fn lock(&self) -> io::Result<File> {
        match self.dir.create_new(LOCK, &[], 0o600) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let file = self.dir.open_file(LOCK, true)?;
        file.lock()?;
        Ok(file)
    }
    pub fn read(&self) -> io::Result<Option<RuntimeData>> {
        let bytes = match self.dir.read(FILE, MAX_BYTES + 1) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if bytes.len() > MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime ledger exceeds safe size",
            ));
        }
        let Ok(data) = crate::proto::decode_wire::<RuntimeData>(&bytes) else {
            return Ok(None);
        };
        if data.pid <= 0 || data.port <= 0 {
            return Ok(None);
        }
        Ok(Some(data))
    }
    pub fn write(&self, port: u16, pid: i64, now: Timestamp) -> io::Result<()> {
        if port == 0 || pid <= 0 || self.trial_port.is_some_and(|p| p != port) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid explicit runtime identity",
            ));
        }
        let _lock = self.lock()?;
        let data = RuntimeData {
            version: 1,
            pid,
            port: i64::from(port),
            started_at: crate::proto::time::format_rfc3339_nano(now).map_err(io::Error::other)?,
        };
        self.dir.replace(
            FILE,
            &serde_json::to_vec_pretty(&data).map_err(io::Error::other)?,
            0o600,
        )
    }
    pub fn remove_if_pid(&self, pid: i64) -> io::Result<()> {
        let _lock = self.lock()?;
        if self.read()?.is_some_and(|data| data.pid == pid) {
            self.dir.remove_file(FILE)?;
        }
        Ok(())
    }
    /// Source ordering: configured port first, then recorded PID, then fallback
    /// port probe. Trial mode refuses probes outside its one explicit port.
    pub fn running_port_with(
        &self,
        configured: i64,
        alive: impl Fn(i64) -> bool,
        probe: impl Fn(i64) -> bool,
    ) -> Option<u16> {
        let allowed = |port: i64| {
            u16::try_from(port)
                .ok()
                .filter(|p| *p > 0 && self.trial_port.is_none_or(|t| t == *p))
        };
        if let Some(port) = allowed(configured)
            && probe(configured)
        {
            return Some(port);
        }
        let data = self.read().ok()??;
        if !alive(data.pid) {
            let _ = self.remove_if_pid(data.pid);
            return None;
        }
        let port = allowed(data.port)?;
        if data.port == configured || !probe(data.port) {
            return None;
        }
        Some(port)
    }
    pub async fn running_port(&self, configured: i64, token: &str) -> Option<u16> {
        let allowed = |port: i64| {
            u16::try_from(port)
                .ok()
                .filter(|p| *p > 0 && self.trial_port.is_none_or(|t| t == *p))
        };
        if let Some(port) = allowed(configured)
            && probe_hub_info(port, token).await
        {
            return Some(port);
        }
        let data = self.read().ok()??;
        if !crate::process::pid_alive(data.pid) {
            let _ = self.remove_if_pid(data.pid);
            return None;
        }
        let port = allowed(data.port)?;
        if data.port == configured || !probe_hub_info(port, token).await {
            return None;
        }
        Some(port)
    }
}
/// Credentials are sent only to the explicit numeric loopback URL. Redirects
/// and environment proxy discovery are disabled, a deliberate confinement guard.
pub async fn probe_hub_info(port: u16, token: &str) -> bool {
    if port == 0 {
        return false;
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
    else {
        return false;
    };
    let Ok(mut url) = url::Url::parse(&format!("http://127.0.0.1:{port}/api/info")) else {
        return false;
    };
    url.query_pairs_mut().append_pair("token", token);
    client
        .get(url)
        .send()
        .await
        .is_ok_and(|response| response.status() == reqwest::StatusCode::OK)
}
