//! Hub adapters reuse the launcher store and its contained connection owner.
use super::http::{Request, Response, decode_json, require_method};
use crate::{
    launcher::{ConnectionManager, LauncherError, Profile},
    process::Cancellation,
    proto::wire::{GoWire, Schema},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn methods(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/servers" => Some(&["GET", "POST"]),
        "/api/servers/connect/status" => Some(&["GET"]),
        "/api/servers/connect" | "/api/servers/disconnect" | "/api/profiles/fetch" => {
            Some(&["POST"])
        }
        _ => None,
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ProfilesRequest {
    profiles: Option<Vec<Profile>>,
}
impl GoWire for ProfilesRequest {
    const GO_TYPE: &'static str = "LauncherProfilesRequest";
    const SCHEMAS: &'static [Schema] = <Profile as GoWire>::SCHEMAS;
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ConnectRequest {
    name: String,
}
impl GoWire for ConnectRequest {
    const GO_TYPE: &'static str = "LauncherConnectRequest";
    const SCHEMAS: &'static [Schema] = <Profile as GoWire>::SCHEMAS;
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct DisconnectRequest {
    name: String,
    mode: String,
}
impl GoWire for DisconnectRequest {
    const GO_TYPE: &'static str = "LauncherDisconnectRequest";
    const SCHEMAS: &'static [Schema] = <Profile as GoWire>::SCHEMAS;
}
pub struct ServerHttp {
    pub manager: Arc<ConnectionManager>,
}
impl ServerHttp {
    pub fn new(manager: Arc<ConnectionManager>) -> Self {
        Self { manager }
    }
    /// Authentication is performed by the Hub before entering this adapter.
    pub async fn handle_authenticated(&self, request: &Request, cancel: &Cancellation) -> Response {
        let Some(allowed) = methods(&request.path) else {
            return Response::error(404, "not_found", "not found");
        };
        if let Err(response) = require_method(request, allowed) {
            return response;
        }
        let result: Result<Response, LauncherError> = async {
            match (request.path.as_str(), request.method.as_str()) {
                ("/api/servers", "GET") => Ok(Response::json(200, &self.manager.profiles().await?)),
                ("/api/servers", "POST") => {
                    let body: ProfilesRequest = decode_json(request).map_err(decode_error)?;
                    self.manager.replace_profiles(body.profiles).await?;
                    Ok(Response::json(200, &json!({"ok":true})))
                }
                ("/api/servers/connect", "POST") => {
                    let body: ConnectRequest = decode_json(request).map_err(decode_error)?;
                    self.manager.connect(body.name.trim()).await?;
                    Ok(Response::json(
                        200,
                        &json!({"ok":true,"status":"connecting"}),
                    ))
                }
                ("/api/servers/connect/status", "GET") => {
                    Ok(Response::json(200, &self.manager.status().await))
                }
                ("/api/profiles/fetch", "POST") => {
                    let mut body: crate::launcher::FetchParams = decode_json(request).map_err(decode_error)?;
                    body.trim();
                    if body.host.is_empty() { return Err(LauncherError::new(400, "bad_request", "host is required")); }
                    let exported = self.manager.connector.fetch_remote_profile(&body, cancel).await
                        .map_err(|error| LauncherError::new(502, "fetch_failed", error))?;
                    let store = self.manager.store.clone();
                    let existing = tokio::task::spawn_blocking(move || store.load_profiles()).await
                        .map_err(|_| LauncherError::new(500, "load_failed", "profile read interrupted"))?
                        .map_err(|error| LauncherError::new(500, "load_failed", error.to_string()))?;
                    let exported = crate::launcher::complete_fetched_profile(exported, &body, &existing)
                        .map_err(|error| LauncherError::new(400, "invalid_profile", error))?;
                    Ok(Response::json(200, &json!({"ok":true,"profile":exported.profile,"host_candidates":exported.host_candidates})))
                }
                ("/api/servers/disconnect", "POST") => {
                    let body: DisconnectRequest = decode_json(request).map_err(decode_error)?;
                    let warning = self
                        .manager
                        .disconnect(&body.name, &body.mode, cancel, "Hub")
                        .await?;
                    let mut value = json!({"ok":true});
                    if let Some(warning) = warning {
                        value["warning"] = warning.into();
                    }
                    Ok(Response::json(200, &value))
                }
                _ => unreachable!("method guard"),
            }
        }
        .await;
        result.unwrap_or_else(|error| Response::error(error.status, error.code, &error.detail))
    }
}
fn decode_error(_: Response) -> LauncherError {
    LauncherError::new(400, "bad_request", "invalid json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::RuntimePaths,
        launcher::{ConnectorConfig, LauncherStore},
    };
    #[tokio::test]
    async fn real_store_preserves_profiles_and_rejects_unvalidated_connect_before_spawn() {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49317, installed.path()).unwrap();
        let store = LauncherStore::open(paths.clone()).unwrap();
        let mut connector = ConnectorConfig::new(paths.clone(), root.path().into());
        connector.ssh_executable = root.path().join("must-never-spawn.exe");
        let manager = ConnectionManager::new(store, connector);
        let http = ServerHttp::new(manager.clone());
        let cancel = Cancellation::default();
        let response = http
            .handle_authenticated(
                &Request {
                    method: "POST".into(),
                    path: "/api/servers".into(),
                    body: br#"{"Profiles":[{"name":"one","type":"ssh","host":"example.invalid"}]}"#
                        .to_vec(),
                    ..Default::default()
                },
                &cancel,
            )
            .await;
        assert_eq!(response.status, 200);
        let response = http
            .handle_authenticated(
                &Request {
                    method: "GET".into(),
                    path: "/api/servers".into(),
                    ..Default::default()
                },
                &cancel,
            )
            .await;
        let value: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(value["profiles"][0]["name"], "one");
        // Hand edited source data must be validated again before connector start.
        std::fs::write(paths.resource(crate::config::Resource::LauncherProfiles), b"version: 1\nprofiles:\n  - name: injected\n    type: ssh\n    host: -oProxyCommand=synthetic\n").unwrap();
        let response = http
            .handle_authenticated(
                &Request {
                    method: "POST".into(),
                    path: "/api/servers/connect".into(),
                    body: br#"{"name":" injected "}"#.to_vec(),
                    ..Default::default()
                },
                &cancel,
            )
            .await;
        assert_eq!(response.status, 400);
        let value: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(value["error"], "invalid_profile");
        assert_eq!(manager.status().await["status"], "idle");
        manager.close_all().await;
    }
}
