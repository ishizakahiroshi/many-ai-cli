use super::http::{Request, Response, decode_json, require_method};
use crate::{
    application::nvidia_nim::{NvidiaNim, SettingsError},
    process::Cancellation,
    proto::wire::{Field, GoWire, Schema},
};
use std::sync::Arc;
pub struct NvidiaHttp {
    owner: Arc<NvidiaNim>,
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct SettingsRequest {
    enabled: bool,
    api_key: String,
}
impl GoWire for SettingsRequest {
    const GO_TYPE: &'static str = "nvidiaNIMSettingsRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "nvidiaNIMSettingsRequest",
        fields: &[
            Field {
                name: "enabled",
                kind: "bool",
            },
            Field {
                name: "api_key",
                kind: "string",
            },
        ],
    }];
}
impl NvidiaHttp {
    pub fn new(owner: Arc<NvidiaNim>) -> Arc<Self> {
        Arc::new(Self { owner })
    }
    pub fn methods(path: &str) -> Option<&'static [&'static str]> {
        match path {
            "/api/nvidia-nim" => Some(&["GET", "PUT"]),
            "/api/nvidia-nim/key" => Some(&["DELETE"]),
            "/api/nvidia-nim/test" => Some(&["POST"]),
            _ => None,
        }
    }
    pub async fn handle_authenticated(
        &self,
        r: &Request,
        cancel: &Cancellation,
    ) -> Option<Response> {
        let methods = Self::methods(&r.path)?;
        if let Err(error) = require_method(r, methods) {
            return Some(error);
        }
        Some(match (r.path.as_str(), r.method.as_str()) {
            ("/api/nvidia-nim", "GET") => status(self.owner.status()),
            ("/api/nvidia-nim", "PUT") => match decode_json::<SettingsRequest>(r) {
                Ok(body) => status(self.owner.save_settings(body.enabled, &body.api_key)),
                Err(error) => error,
            },
            ("/api/nvidia-nim/key", _) => status(self.owner.delete_key()),
            ("/api/nvidia-nim/test", _) => match self.owner.test(cancel).await {
                Ok(Ok(())) => Response::json(200, &serde_json::json!({"ok":true})),
                Ok(Err(error)) => Response::json(
                    200,
                    &serde_json::json!({"ok":false,"code":error.test_code()}),
                ),
                Err(SettingsError::InvalidKey) => Response::error(
                    400,
                    "api_key_missing",
                    "Configure an NVIDIA API key before testing the connection",
                ),
                Err(error) => failure(error),
            },
            _ => unreachable!(),
        })
    }
}
fn status(
    result: Result<crate::application::nvidia_nim::SettingsStatus, SettingsError>,
) -> Response {
    match result {
        Ok(status) => Response::json(200, &status),
        Err(error) => failure(error),
    }
}
fn failure(error: SettingsError) -> Response {
    match error {
        SettingsError::Status => Response::error(
            500,
            "key_status_unavailable",
            "NVIDIA API key status is unavailable",
        ),
        SettingsError::Environment => Response::error(
            409,
            "key_managed_by_environment",
            "NVIDIA_API_KEY is managed by the Hub environment",
        ),
        SettingsError::InvalidKey => {
            Response::error(400, "invalid_api_key", "NVIDIA API key is invalid")
        }
        SettingsError::SaveKey => {
            Response::error(500, "key_save_failed", "NVIDIA API key could not be saved")
        }
        SettingsError::SaveConfig => {
            Response::error(500, "save_failed", "NVIDIA NIM settings could not be saved")
        }
        SettingsError::DeleteKey => Response::error(
            500,
            "key_delete_failed",
            "NVIDIA API key could not be deleted",
        ),
    }
}
