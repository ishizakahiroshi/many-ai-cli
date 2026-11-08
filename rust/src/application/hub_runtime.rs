//! Source: internal/hubruntime/runtime.go. All filesystem operations retain the
//! explicit runtime directory capability; no process-global home lookup occurs.
use crate::proto::time::Timestamp;
use crate::{
    config::RuntimePaths,
    files::safe_fs::Dir,
    process::wrapper_startup::recovery_identity,
    proto::wire::{Field, GoWire, Schema},
};
use serde::{Deserialize, Serialize};
use std::{fs::File, io, sync::Arc, time::Duration};
pub const FILE: &str = "hub-runtime.json";
const LOCK: &str = "hub-runtime.json.lock";
const MAX_BYTES: usize = 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeData {
    pub version: i64,
    pub pid: i64,
    pub port: i64,
    pub started_at: String,
    /// The Hub process's identity (creation time, plus boot on Linux). A Hub
    /// that ends without removing this record leaves its PID to whatever
    /// process the OS starts next, so the PID alone never names it (U11).
    /// Empty in records written before identities were kept, or where the
    /// platform has none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub process: String,
}
impl Default for RuntimeData {
    fn default() -> Self {
        Self {
            version: 0,
            pid: 0,
            port: 0,
            started_at: "0001-01-01T00:00:00Z".into(),
            process: String::new(),
        }
    }
}
/// What holds the ledger's PID now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// No process has that PID.
    Gone,
    /// The Hub that wrote the record.
    Recorded,
    /// Another process, given the PID after the Hub ended.
    Other,
    /// A live process that cannot be identified here (macOS, or a failed
    /// read). Startup treats it as the Hub, as before; stop does not end it.
    Unverified,
}
pub fn owner(data: &RuntimeData) -> Owner {
    if !crate::process::pid_alive(data.pid) {
        return Owner::Gone;
    }
    let Ok(pid) = u32::try_from(data.pid) else {
        return Owner::Gone;
    };
    match recovery_identity::of_pid(pid) {
        Ok(Some(identity)) => match same_process(data, &identity) {
            Some(true) => Owner::Recorded,
            Some(false) => Owner::Other,
            None => Owner::Unverified,
        },
        Ok(None) => Owner::Gone,
        Err(_) => Owner::Unverified,
    }
}
/// Whether the process with `identity` is the Hub that wrote `data`. With a
/// recorded identity the two must match. A record written before identities
/// were kept has only its time: the Hub already existed then, while a process
/// that was given its PID later was created after the Hub ended. None when
/// neither can be judged.
pub fn same_process(data: &RuntimeData, identity: &str) -> Option<bool> {
    if !data.process.is_empty() {
        return Some(identity == data.process);
    }
    let started = crate::proto::time::parse_rfc3339(&data.started_at)
        .ok()
        .map(Timestamp::unix_nanos)
        .filter(|started| *started > 0)?;
    let created = recovery_identity::created_unix_nanos(identity)?;
    // Slack for Linux, whose creation times are whole boot seconds plus ticks.
    Some(created <= started + 2_000_000_000)
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
            Field {
                name: "process",
                kind: "string",
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
            dir: Arc::new(Dir::open_or_create_private_components(paths.root())?),
            trial_port: paths.is_trial().then(|| paths.port()),
        })
    }
    fn lock(&self) -> io::Result<File> {
        let file = self.dir.open_lock(LOCK)?;
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
            process: u32::try_from(pid)
                .ok()
                .and_then(|pid| recovery_identity::of_pid(pid).ok().flatten())
                .unwrap_or_default(),
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
    /// `proof` is the CLI's local proof (`hub::pin::cli_local_proof`), sent so a
    /// Hub with a PIN set counts this CLI as on this machine (U09).
    pub async fn running_port(
        &self,
        configured: i64,
        token: &str,
        proof: Option<&str>,
    ) -> Option<u16> {
        let allowed = |port: i64| {
            u16::try_from(port)
                .ok()
                .filter(|p| *p > 0 && self.trial_port.is_none_or(|t| t == *p))
        };
        if let Some(port) = allowed(configured)
            && probe_hub_info(port, token, proof).await
        {
            return Some(port);
        }
        let data = self.read().ok()??;
        if matches!(owner(&data), Owner::Gone | Owner::Other) {
            let _ = self.remove_if_pid(data.pid);
            return None;
        }
        let port = allowed(data.port)?;
        if data.port == configured || !probe_hub_info(port, token, proof).await {
            return None;
        }
        Some(port)
    }
}
/// Credentials are sent only to the explicit numeric loopback URL. Redirects
/// and environment proxy discovery are disabled, a deliberate confinement guard.
/// The token goes only to a listener this OS user holds (U01).
pub async fn probe_hub_info(port: u16, token: &str, proof: Option<&str>) -> bool {
    if port == 0 || !crate::process::listener::loopback_listener_is_own(port).unwrap_or(false) {
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
        .headers(local_proof_headers(proof))
        .send()
        .await
        .is_ok_and(|response| response.status() == reqwest::StatusCode::OK)
}

/// The header that carries the CLI's local proof, when there is one.
pub fn local_proof_headers(proof: Option<&str>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(Ok(mut value)) = proof.map(reqwest::header::HeaderValue::from_str) {
        value.set_sensitive(true);
        headers.insert(crate::hub::pin::LOCAL_PROOF_HEADER, value);
    }
    headers
}
/// Asks the Hub this OS user runs on `port` for a one-time code to open the
/// dashboard with, so the browser's command line carries the code and not the
/// token (U04). `None` when the Hub is not this user's or predates codes.
pub async fn request_launch_code(port: u16, token: &str, proof: Option<&str>) -> Option<String> {
    if port == 0 || !crate::process::listener::loopback_listener_is_own(port).unwrap_or(false) {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .ok()?;
    let mut response = client
        .post(format!("http://127.0.0.1:{port}/api/auth/launch-code"))
        .bearer_auth(token)
        .headers(local_proof_headers(proof))
        .header("Origin", format!("http://127.0.0.1:{port}"))
        .send()
        .await
        .ok()?;
    if response.status() != reqwest::StatusCode::OK {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > 4096 {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let code = value["code"].as_str()?;
    (!code.is_empty() && code.bytes().all(|b| b.is_ascii_hexdigit())).then(|| code.to_owned())
}
/// The dashboard URL to open: with a launch code when there is one, and with
/// nothing secret otherwise (the dashboard then uses the token it keeps).
pub fn dashboard_url(port: u16, code: Option<&str>) -> String {
    let mut url = format!("http://127.0.0.1:{port}/");
    if let Some(code) = code {
        url.push_str("?launch=");
        url.push_str(code);
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Windows-form identity of a process created at `unix_seconds`.
    fn created_at(unix_seconds: u64) -> String {
        format!(
            "windows:{:016x}",
            unix_seconds * 10_000_000 + 116_444_736_000_000_000
        )
    }
    fn record(started_at: &str, process: &str) -> RuntimeData {
        RuntimeData {
            version: 1,
            pid: 42,
            port: 48888,
            started_at: started_at.into(),
            process: process.into(),
        }
    }

    #[test]
    fn audit_u11_same_process_needs_the_recorded_identity_or_an_earlier_creation() {
        // 2026-01-01T00:00:00Z
        let hub = created_at(1_767_225_600);
        let later = created_at(1_767_225_601);
        assert_eq!(
            same_process(&record("2026-01-01T00:00:05Z", &hub), &hub),
            Some(true)
        );
        assert_eq!(
            same_process(&record("2026-01-01T00:00:05Z", &hub), &later),
            Some(false)
        );
        // Records written before identities were kept go by creation time.
        assert_eq!(
            same_process(&record("2026-01-01T00:00:05Z", ""), &hub),
            Some(true)
        );
        assert_eq!(
            same_process(&record("2025-12-31T23:59:00Z", ""), &hub),
            Some(false)
        );
        // Nothing to judge by.
        assert_eq!(
            same_process(&record("0001-01-01T00:00:00Z", ""), &hub),
            None
        );
        assert_eq!(
            same_process(&record("2026-01-01T00:00:05Z", ""), "synthetic:unknown"),
            None
        );
    }

    #[test]
    fn audit_u11_the_identity_survives_the_go_wire_and_is_omitted_when_empty() {
        let data = record("2026-01-01T00:00:05Z", &created_at(1_767_225_600));
        let bytes = serde_json::to_vec(&data).unwrap();
        assert_eq!(
            crate::proto::decode_wire::<RuntimeData>(&bytes).unwrap(),
            data
        );
        let old = serde_json::to_value(record("2026-01-01T00:00:05Z", "")).unwrap();
        assert!(old.get("process").is_none());
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[tokio::test]
    async fn audit_u11_running_port_drops_a_record_whose_pid_another_process_holds() {
        use crate::process::wrapper_startup::recovery_identity::test_support::Bystander;
        let home = tempfile::tempdir().unwrap();
        let ledger = RuntimeLedger::open(&RuntimePaths::production(home.path()).unwrap()).unwrap();
        let bystander = Bystander::spawn();
        let stale = record("2026-01-01T00:00:00Z", "synthetic:another-process");
        let stale = RuntimeData {
            pid: i64::from(bystander.pid()),
            ..stale
        };
        ledger
            .dir
            .replace(FILE, &serde_json::to_vec(&stale).unwrap(), 0o600)
            .unwrap();
        // Port 0 skips the configured probe, so only the record is consulted.
        assert_eq!(ledger.running_port(0, "synthetic-token", None).await, None);
        assert!(ledger.read().unwrap().is_none());
    }
}
