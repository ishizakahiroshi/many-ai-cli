use super::http::{Request, Response, decode_json, require_method};
use crate::{
    application::diagnostics::{
        Diagnostics,
        bug_report::{BugReport, FinalizeRequest, PreviewRequest, ReportError},
    },
    process::Cancellation,
    proto::wire::{Field, GoWire, Schema},
};
use std::sync::Arc;
pub struct DiagnosticHttp {
    doctor: Arc<Diagnostics>,
    reports: Arc<BugReport>,
}
impl DiagnosticHttp {
    pub fn new(doctor: Arc<Diagnostics>, reports: Arc<BugReport>) -> Arc<Self> {
        Arc::new(Self { doctor, reports })
    }
    pub fn methods(path: &str) -> Option<&'static [&'static str]> {
        match path {
            "/api/doctor" => Some(&["GET"]),
            "/api/bug-report/preview" | "/api/bug-report/finalize" => Some(&["POST"]),
            _ => None,
        }
    }
    pub async fn handle_authenticated(
        &self,
        request: &Request,
        cancel: &Cancellation,
    ) -> Option<Response> {
        let methods = Self::methods(&request.path)?;
        if let Err(e) = require_method(request, methods) {
            return Some(e);
        }
        Some(match request.path.as_str() {
            "/api/doctor" => match self.doctor.run(cancel).await {
                Ok(report) => Response::json(200, &report),
                Err(_) => Response::error(
                    503,
                    "doctor_unavailable",
                    "doctor was cancelled or unavailable",
                ),
            },
            "/api/bug-report/preview" => match decode_json::<PreviewRequest>(request) {
                Ok(body) => match self.reports.preview(body) {
                    Ok(reply) => Response::json(200, &reply),
                    Err(e) => failure(e),
                },
                Err(e) => e,
            },
            "/api/bug-report/finalize" => match decode_json::<FinalizeRequest>(request) {
                Ok(body) => match self.reports.finalize(body, cancel).await {
                    Ok(reply) => Response::json(200, &reply),
                    Err(e) => failure(e),
                },
                Err(e) => e,
            },
            _ => unreachable!(),
        })
    }
}
fn failure(e: ReportError) -> Response {
    Response::error(e.status, e.code, e.detail)
}
impl GoWire for PreviewRequest {
    const GO_TYPE: &'static str = "BugReportPreviewRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "BugReportPreviewRequest",
        fields: &[
            Field {
                name: "session_id",
                kind: "*int",
            },
            Field {
                name: "include_recent_log_lines",
                kind: "int",
            },
            Field {
                name: "locale",
                kind: "string",
            },
            Field {
                name: "user_agent",
                kind: "string",
            },
        ],
    }];
}
impl GoWire for FinalizeRequest {
    const GO_TYPE: &'static str = "BugReportFinalizeRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "BugReportFinalizeRequest",
        fields: &[
            Field {
                name: "symptom",
                kind: "string",
            },
            Field {
                name: "reproduction",
                kind: "string",
            },
            Field {
                name: "environment_markdown",
                kind: "string",
            },
            Field {
                name: "locale",
                kind: "string",
            },
            Field {
                name: "include_session_log",
                kind: "bool",
            },
            Field {
                name: "log_markdown",
                kind: "string",
            },
            Field {
                name: "log_preview_token",
                kind: "string",
            },
        ],
    }];
}
