use super::http::{Request, Response, decode_json};
use crate::{config::NotifyConfig, notify::Manager};
pub async fn send_test(request: &Request, manager: &Manager) -> Response {
    // Decode the nested fixed Go Backend struct through its exact shared schema;
    // missing/null Backend preserves a zero struct and fails validation.
    #[derive(Default, serde::Deserialize)]
    #[serde(default)]
    struct Test {
        backend: crate::config::NotifyBackendConfig,
    }
    impl crate::proto::wire::GoWire for Test {
        const GO_TYPE: &'static str = "NotifyTest";
        const SCHEMAS: &'static [crate::proto::wire::Schema] = &[
            crate::proto::wire::Schema {
                name: "NotifyTest",
                fields: &[crate::proto::wire::Field {
                    name: "backend",
                    kind: "NotifyBackendConfig",
                }],
            },
            crate::proto::wire::Schema {
                name: "NotifyBackendConfig",
                fields: <NotifyConfig as crate::proto::wire::GoWire>::SCHEMAS[1].fields,
            },
        ];
    }
    let body: Test = match decode_json(request) {
        Ok(body) => body,
        Err(error) => return error,
    };
    if let Err(error) = crate::notify::validate_backend(&body.backend) {
        return Response::error(400, "invalid_backend", error);
    }
    if manager
        .send_test(
            body.backend,
            "many-ai-cli test".into(),
            "Test notification from many-ai-cli Hub".into(),
        )
        .await
    {
        Response::json(200, &serde_json::json!({"ok":true}))
    } else {
        Response::error(502, "send_failed", "notification request failed")
    }
}
