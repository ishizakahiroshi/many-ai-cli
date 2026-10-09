//! Authenticated managed Whisper HTTP operations; install survives its waiter.
use super::http::{Request, Response, require_method};
use crate::{
    application::whisper::{WhisperError, WhisperManager},
    process::Cancellation,
};
use std::sync::Arc;
pub struct WhisperHttp {
    owner: Arc<WhisperManager>,
}
impl WhisperHttp {
    pub fn new(owner: Arc<WhisperManager>) -> Self {
        Self { owner }
    }
    pub fn methods(path: &str) -> Option<&'static [&'static str]> {
        match path {
            "/api/whisper/status" => Some(&["GET"]),
            "/api/whisper/install"
            | "/api/whisper/uninstall"
            | "/api/whisper/start"
            | "/api/whisper/stop" => Some(&["POST"]),
            _ => None,
        }
    }
    pub async fn handle_authenticated(
        &self,
        request: &Request,
        cancel: &Cancellation,
    ) -> Option<Response> {
        let methods = Self::methods(&request.path)?;
        if let Err(error) = require_method(request, methods) {
            return Some(error);
        }
        if request.path == "/api/whisper/install" && !self.owner.supported() {
            return Some(failure(WhisperError {
                status: 400,
                code: "unsupported_platform",
                detail: crate::application::whisper::manifest::UNSUPPORTED.into(),
            }));
        }
        let result = match request.path.as_str() {
            "/api/whisper/install" => match model(request) {
                Ok(model) => match self.owner.install(&model) {
                    Ok(true) => Ok(()),
                    Ok(false) => {
                        return Some(match self.owner.status() {
                            Ok(status) => Response::json(
                                200,
                                &serde_json::json!({"ok":true,"install":status.install}),
                            ),
                            Err(error) => failure(error),
                        });
                    }
                    Err(error) => Err(error),
                },
                Err(response) => return Some(response),
            },
            "/api/whisper/start" => match model(request) {
                Ok(model) => self.owner.select_and_start(&model, cancel).await,
                Err(response) => return Some(response),
            },
            "/api/whisper/stop" => self.owner.stop().await,
            "/api/whisper/uninstall" => self.owner.uninstall().await,
            _ => Ok(()),
        };
        Some(match result {
            Err(error) => failure(error),
            Ok(()) => match self.owner.status() {
                Ok(status) => Response::json(200, &status),
                Err(error) => failure(error),
            },
        })
    }
}
pub fn failure(error: WhisperError) -> Response {
    Response::error(error.status, error.code, &error.detail)
}
#[derive(serde::Deserialize)]
struct ModelRequest {
    model: String,
}
impl crate::proto::wire::GoWire for ModelRequest {
    const GO_TYPE: &'static str = "WhisperInstallRequest";
    const SCHEMAS: &'static [crate::proto::wire::Schema] = &[crate::proto::wire::Schema {
        name: "WhisperInstallRequest",
        fields: &[crate::proto::wire::Field {
            name: "model",
            kind: "string",
        }],
    }];
}
fn model(request: &Request) -> Result<String, Response> {
    if request.body.is_empty() {
        return Ok(String::new());
    }
    super::http::decode_json::<ModelRequest>(request).map(|request| request.model)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_request_preserves_source_case_duplicates_null_and_trailing_json() {
        let request = Request {
            body: br#"{"model":"small","MODEL":"tiny-q5_1","Model":null} {}"#.to_vec(),
            ..Default::default()
        };
        assert_eq!(model(&request).unwrap(), "tiny-q5_1");
        assert!(
            model(&Request {
                body: br#"{"model":5,"model":"small"}"#.to_vec(),
                ..Default::default()
            })
            .is_err()
        );
        assert_eq!(model(&Request::default()).unwrap(), "");
    }
}
