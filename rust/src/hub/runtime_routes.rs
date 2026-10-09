//! Native encoding observation and authenticated launcher network hints.
use super::{
    auth,
    http::{Request, Response, decode_json},
};
use crate::proto::wire::{Field, GoWire, Schema};
use serde::Deserialize;
use std::sync::Mutex;
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct NetHint {
    pub ssh: bool,
    pub host_label: String,
    pub env_kind: String,
}
impl GoWire for NetHint {
    const GO_TYPE: &'static str = "NetHintRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "NetHintRequest",
        fields: &[
            Field {
                name: "ssh",
                kind: "bool",
            },
            Field {
                name: "host_label",
                kind: "string",
            },
            Field {
                name: "env_kind",
                kind: "string",
            },
        ],
    }];
}
#[derive(Default)]
pub struct NetHints {
    value: Mutex<NetHint>,
}
impl NetHints {
    pub fn snapshot(&self) -> NetHint {
        self.value.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    pub fn receive(&self, request: &Request) -> Response {
        if auth::logically_remote(request) {
            return Response::error(
                403,
                "forbidden",
                "net-hint is only accepted from local loopback callers",
            );
        }
        let mut value = match decode_json::<NetHint>(request) {
            Ok(v) => v,
            Err(error) => return error,
        };
        value.host_label = value
            .host_label
            .chars()
            .filter(|c| *c >= '\u{20}' && *c != '\u{7f}')
            .take(128)
            .collect::<String>()
            .trim()
            .to_owned();
        value.env_kind =
            match crate::proto::unicode::simple_lower(value.env_kind.trim()).as_str() {
                "local" => "local",
                "wsl" => "wsl",
                "remote" => "remote",
                "remote-tunnel" | "remotetunnel" | "remote_tunnel" => "remote-tunnel",
                _ => "",
            }
            .into();
        *self.value.lock().unwrap_or_else(|p| p.into_inner()) = value;
        Response::json(200, &serde_json::json!({"ok":true}))
    }
}
pub fn encoding(parent_shell: &str) -> Response {
    #[cfg(windows)]
    let (input, output) = unsafe {
        (
            windows_sys::Win32::System::Console::GetConsoleCP(),
            windows_sys::Win32::System::Console::GetConsoleOutputCP(),
        )
    };
    #[cfg(not(windows))]
    let (input, output) = (0u32, 0u32);
    Response::json(
        200,
        &serde_json::json!({"is_windows":cfg!(windows),"is_powershell":crate::proto::unicode::simple_lower(parent_shell).contains("powershell"),"input_codepage":input,"output_codepage":output,"is_utf8":!cfg!(windows)||(input==65001&&output==65001)}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hints_reject_remote_before_decode_and_keep_go_scalar_limit() {
        let hints = NetHints::default();
        let mut req=Request { remote_addr:"127.0.0.1:1234".into(), body:serde_json::to_vec(&serde_json::json!({"ssh":true,"host_label":format!("\n{}\u{7f}tail", "界".repeat(128)),"env_kind":" REMOTE_TUNNEL "})).unwrap(),..Default::default() };
        assert_eq!(hints.receive(&req).status, 200);
        let got = hints.snapshot();
        assert!(got.ssh);
        assert_eq!(got.host_label, "界".repeat(128));
        assert_eq!(got.env_kind, "remote-tunnel");
        req.headers
            .push(("X-Forwarded-For".into(), "203.0.113.10".into()));
        req.host = "localhost:1234".into();
        req.body = b"bad".to_vec();
        assert_eq!(hints.receive(&req).status, 403);
        assert_eq!(hints.snapshot().host_label, got.host_label);
    }
}
