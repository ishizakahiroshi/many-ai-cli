//! Card edits are one atomic core patch, followed by the existing journal and
//! socket effect path. HTTP wait cancellation cannot strand an accepted edit.
use super::*;
use crate::{
    hub::{http::decode_json, task_owner::HubTaskHandle},
    proto::{
        SessionMeta,
        core::*,
        wire::{Field, GoWire, Schema},
    },
};
use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct MetadataBody {
    label: Option<String>,
    pinned: Option<bool>,
    color: Option<String>,
    note: Option<String>,
}
impl GoWire for MetadataBody {
    const GO_TYPE: &'static str = "sessionMetaPatch";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "sessionMetaPatch",
        fields: &[
            Field {
                name: "label",
                kind: "*string",
            },
            Field {
                name: "pinned",
                kind: "*bool",
            },
            Field {
                name: "color",
                kind: "*string",
            },
            Field {
                name: "note",
                kind: "*string",
            },
        ],
    }];
}
impl SessionHttp {
    /// Header/path guards precede body decoding. The router must use its owned
    /// request lane; the effect permit is reserved before committing the patch.
    pub async fn handle_metadata_authenticated(
        &self,
        request: &Request,
        effects: Arc<dyn CoreEffectSink>,
        tasks: &HubTaskHandle,
    ) -> Response {
        let id = match metadata_path(&request.path) {
            Ok(id) => id,
            Err(error) => return error,
        };
        let body: MetadataBody = match decode_json(request) {
            Ok(body) => body,
            Err(error) => return error,
        };
        if body.label.is_none()
            && body.pinned.is_none()
            && body.color.is_none()
            && body.note.is_none()
        {
            return bad_request("at least one meta field is required");
        }
        let permit = match tasks.effect_permit() {
            Ok(permit) => permit,
            Err(_) => {
                return Response::error(
                    503,
                    "session_meta_unavailable",
                    "session metadata service is unavailable",
                );
            }
        };
        let update = match self.core.patch_card_meta(
            id,
            SessionCardMetaPatch {
                label: body.label,
                pinned: body.pinned,
                color: body.color,
                note: body.note,
            },
        ) {
            Ok(update) => update,
            Err(SessionError::NotFound(_)) => {
                return Response::error(404, "not_found", "session not found");
            }
            Err(SessionError::InvalidRequest(detail)) => return bad_request(&detail),
            Err(_) => {
                return Response::error(
                    500,
                    "session_meta_failed",
                    "failed to save session metadata",
                );
            }
        };
        let meta = SessionMeta {
            label: update.meta.label,
            pinned: update.meta.pinned,
            color: update.meta.color,
            note: update.meta.note,
            auto_title: update.meta.auto_title,
        };
        let core = self.core.clone();
        let completion = permit.start(async move {
            effects.apply(update.effects).await?;
            effects.apply(core.broadcast_ui(update.notification)).await
        });
        match completion.wait().await {
            Ok(Ok(())) => Response::json(
                200,
                &json!({"ok":true,"session_id":id.0,"session_meta":meta}),
            ),
            _ => Response::error(
                500,
                "session_meta_failed",
                "failed to save session metadata",
            ),
        }
    }
}
