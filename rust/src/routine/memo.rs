use crate::proto::time::Timestamp;
use crate::{
    files::safe_fs::Dir,
    hub::http::{Request, Response, decode_json, random_hex},
    proto::wire::{Field, GoWire, Schema},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

const MEMO_LIMIT: usize = 500;
const TEXT_LIMIT: usize = 4000;
pub const IMAGE_LIMIT: usize = 10 * 1024 * 1024;
const IMAGES_TOTAL: u64 = 500 * 1024 * 1024;
fn null_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Memo {
    pub id: String,
    pub text: String,
    pub project: String,
    pub done: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub done_at: String,
    #[serde(
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub images: Vec<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct MemoFile {
    version: i64,
    #[serde(deserialize_with = "null_default")]
    memos: Vec<Memo>,
}
const MEMO_SCHEMAS: &[Schema] = &[
    Schema {
        name: "MemoFile",
        fields: &[
            Field {
                name: "version",
                kind: "int",
            },
            Field {
                name: "memos",
                kind: "[]ServiceMemo",
            },
        ],
    },
    Schema {
        name: "ServiceMemo",
        fields: &[
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "text",
                kind: "string",
            },
            Field {
                name: "project",
                kind: "string",
            },
            Field {
                name: "done",
                kind: "bool",
            },
            Field {
                name: "created_at",
                kind: "string",
            },
            Field {
                name: "updated_at",
                kind: "string",
            },
            Field {
                name: "done_at",
                kind: "string",
            },
            Field {
                name: "images",
                kind: "[]string",
            },
        ],
    },
    Schema {
        name: "MemoCreate",
        fields: &[
            Field {
                name: "text",
                kind: "string",
            },
            Field {
                name: "session_id",
                kind: "int",
            },
            Field {
                name: "images",
                kind: "[]string",
            },
        ],
    },
    Schema {
        name: "MemoPatch",
        fields: &[
            Field {
                name: "text",
                kind: "*string",
            },
            Field {
                name: "done",
                kind: "*bool",
            },
        ],
    },
];
impl GoWire for MemoFile {
    const GO_TYPE: &'static str = "MemoFile";
    const SCHEMAS: &'static [Schema] = MEMO_SCHEMAS;
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct MemoCreate {
    text: String,
    session_id: i64,
    #[serde(deserialize_with = "null_default")]
    images: Vec<String>,
}
impl GoWire for MemoCreate {
    const GO_TYPE: &'static str = "MemoCreate";
    const SCHEMAS: &'static [Schema] = MEMO_SCHEMAS;
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct MemoPatch {
    text: Option<String>,
    done: Option<bool>,
}
impl GoWire for MemoPatch {
    const GO_TYPE: &'static str = "MemoPatch";
    const SCHEMAS: &'static [Schema] = MEMO_SCHEMAS;
}

type Writer = Arc<dyn Fn(&Dir, &[u8]) -> io::Result<()> + Send + Sync>;
struct State {
    data: MemoFile,
    load_error: bool,
}
pub struct MemoManager {
    root: Arc<Dir>,
    state: Mutex<State>,
    write: Writer,
}
fn storage_error() -> Response {
    Response::error(503, "memo_store_unavailable", "memo storage is unavailable")
}
fn operation_error() -> Response {
    Response::error(
        500,
        "memo_operation_failed",
        "memo operation could not be saved",
    )
}
fn text_error() -> Response {
    Response::error(400, "bad_request", "text must contain 1 to 4000 bytes")
}
fn timestamp(now: Timestamp) -> Result<String, Response> {
    crate::proto::time::format_with_offset(now, 0, true).map_err(|_| operation_error())
}
impl MemoManager {
    pub fn open(root: Arc<Dir>) -> Self {
        Self::open_with_writer(
            root,
            Arc::new(|root, bytes| root.replace("memos.json", bytes, 0o600)),
        )
    }
    fn open_with_writer(root: Arc<Dir>, write: Writer) -> Self {
        let (data, load_error) = match root.read("memos.json", 8 * 1024 * 1024 + 1) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (
                MemoFile {
                    version: 1,
                    memos: vec![],
                },
                false,
            ),
            Ok(bytes) if bytes.len() <= 8 * 1024 * 1024 => {
                match crate::proto::decode_wire::<MemoFile>(&bytes) {
                    Ok(data) if data.version == 1 => (data, false),
                    _ => (MemoFile::default(), true),
                }
            }
            _ => (MemoFile::default(), true),
        };
        Self {
            root,
            state: Mutex::new(State { data, load_error }),
            write,
        }
    }
    #[cfg(test)]
    pub(super) fn failed_writes(root: Arc<Dir>) -> Self {
        Self::open_with_writer(
            root,
            Arc::new(|_, _| Err(io::Error::other("synthetic write failure"))),
        )
    }
    fn commit(&self, state: &mut State, next: MemoFile) -> Result<(), Response> {
        if state.load_error {
            return Err(storage_error());
        }
        let bytes = serde_json::to_vec_pretty(&next).map_err(|_| operation_error())?;
        (self.write)(&self.root, &bytes).map_err(|_| operation_error())?;
        state.data = next;
        Ok(())
    }
    pub fn mentions(&self) -> Vec<(String, String)> {
        let Ok(state) = self.state.lock() else {
            return vec![];
        };
        if state.load_error {
            return vec![];
        }
        state
            .data
            .memos
            .iter()
            .map(|m| (m.project.clone(), m.text.clone()))
            .collect()
    }
    fn validate_images(&self, state: &State, names: &[String]) -> Result<(), Response> {
        let invalid = || Response::error(400, "bad_request", "invalid image reference");
        if names.len() > 10 {
            return Err(Response::error(
                400,
                "bad_request",
                "a memo can hold up to 10 images",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in names {
            if !valid_image_name(name)
                || !seen.insert(name)
                || state.data.memos.iter().any(|m| m.images.contains(name))
            {
                return Err(invalid());
            }
            if self
                .root
                .child_dir("memo-images", false)
                .and_then(|d| d.metadata(name))
                .is_err()
            {
                return Err(Response::error(400, "bad_request", "image not found"));
            }
        }
        Ok(())
    }
    /// The project resolver is server-owned and receives only session_id. There
    /// is deliberately no request field that can set the persisted project.
    pub fn handle_memos(
        &self,
        request: &Request,
        project_for_session: impl Fn(i64) -> String,
        now: Timestamp,
    ) -> Response {
        let mut state = match self.state.lock() {
            Ok(v) => v,
            Err(_) => return storage_error(),
        };
        if state.load_error {
            return storage_error();
        }
        let id = request
            .path
            .strip_prefix("/api/memos")
            .unwrap_or("")
            .trim_matches('/');
        let result = (|| -> Result<Response, Response> {
            match request.method.as_str() {
                "GET" => {
                    if !id.is_empty() {
                        return Err(Response::error(404, "not_found", "memo route not found"));
                    }
                    Ok(Response::json(200, &json!({"memos":state.data.memos})))
                }
                "POST" => {
                    if !id.is_empty() {
                        return Err(Response::error(
                            405,
                            "method_not_allowed",
                            "unsupported memo operation",
                        ));
                    }
                    let body = decode_json::<MemoCreate>(request)?;
                    let text = body.text.trim();
                    if (text.is_empty() && body.images.is_empty()) || text.len() > TEXT_LIMIT {
                        return Err(text_error());
                    }
                    if state.data.memos.len() >= MEMO_LIMIT {
                        return Err(Response::error(
                            400,
                            "memo_limit",
                            "maximum of 500 saved memos reached",
                        ));
                    }
                    self.validate_images(&state, &body.images)?;
                    let stamp = timestamp(now)?;
                    let item = Memo {
                        id: random_hex(16).map_err(|_| operation_error())?,
                        text: text.into(),
                        project: project_for_session(body.session_id),
                        created_at: stamp.clone(),
                        updated_at: stamp,
                        images: body.images,
                        ..Default::default()
                    };
                    let mut next = state.data.clone();
                    next.memos.push(item.clone());
                    self.commit(&mut state, next)?;
                    Ok(Response::json(200, &json!({"memo":item})))
                }
                "PATCH" => {
                    if id.is_empty() {
                        return Err(Response::error(
                            405,
                            "method_not_allowed",
                            "unsupported memo operation",
                        ));
                    }
                    let mut body = decode_json::<MemoPatch>(request)?;
                    if let Some(text) = body.text.as_mut() {
                        *text = text.trim().into();
                        if text.len() > TEXT_LIMIT {
                            return Err(text_error());
                        }
                    }
                    let mut next = state.data.clone();
                    let item = next
                        .memos
                        .iter_mut()
                        .find(|m| m.id == id)
                        .ok_or_else(|| Response::error(404, "not_found", "memo not found"))?;
                    if body.text.as_ref().is_some_and(|v| v.is_empty()) && item.images.is_empty() {
                        return Err(text_error());
                    }
                    let stamp = timestamp(now)?;
                    if let Some(text) = body.text {
                        item.text = text;
                    }
                    if let Some(done) = body.done {
                        if done && !item.done {
                            item.done_at = stamp.clone();
                        } else if !done {
                            item.done_at.clear();
                        }
                        item.done = done;
                    }
                    item.updated_at = stamp;
                    let result = item.clone();
                    self.commit(&mut state, next)?;
                    Ok(Response::json(200, &json!({"memo":result})))
                }
                "DELETE" => {
                    if id.is_empty() {
                        return Err(Response::error(
                            405,
                            "method_not_allowed",
                            "unsupported memo operation",
                        ));
                    }
                    let mut next = state.data.clone();
                    let i = next
                        .memos
                        .iter()
                        .position(|m| m.id == id)
                        .ok_or_else(|| Response::error(404, "not_found", "memo not found"))?;
                    let removed = next.memos.remove(i);
                    self.commit(&mut state, next)?;
                    if let Ok(images) = self.root.child_dir("memo-images", false) {
                        for name in removed.images {
                            if valid_image_name(&name) {
                                let _ = images.remove_file(&name);
                            }
                        }
                    }
                    Ok(Response::json(200, &json!({"ok":true})))
                }
                _ => Err(Response::error(
                    405,
                    "method_not_allowed",
                    "method not allowed",
                )),
            }
        })();
        result.unwrap_or_else(|e| e)
    }
    pub fn handle_images(&self, request: &Request) -> Response {
        let state = match self.state.lock() {
            Ok(v) => v,
            Err(_) => return storage_error(),
        };
        if state.load_error {
            return storage_error();
        }
        let name = request
            .path
            .strip_prefix("/api/memo-images")
            .unwrap_or("")
            .trim_matches('/');
        if request.method == "GET" {
            if !valid_image_name(name) {
                return Response::error(404, "not_found", "image not found");
            }
            return match self
                .root
                .child_dir("memo-images", false)
                .and_then(|d| d.read(name, IMAGE_LIMIT + 1))
            {
                Ok(data) if data.len() <= IMAGE_LIMIT => {
                    let mut r = Response::bytes(200, image_mime(name), data);
                    r.headers
                        .insert("Cache-Control".into(), "private, max-age=86400".into());
                    r
                }
                _ => Response::error(404, "not_found", "image not found"),
            };
        }
        if request.method != "POST" {
            return Response::error(405, "method_not_allowed", "method not allowed");
        }
        if !name.is_empty() {
            return Response::error(
                405,
                "method_not_allowed",
                "unsupported memo image operation",
            );
        }
        if request.body.len() > IMAGE_LIMIT {
            return Response::error(413, "image_too_large", "image must be 10 MB or smaller");
        }
        let Some(ext) = sniff_image(&request.body) else {
            return Response::error(
                415,
                "unsupported_image",
                "image/png, image/jpeg, image/gif, or image/webp required",
            );
        };
        let dir = match self.root.child_dir("memo-images", true) {
            Ok(d) => d,
            Err(_) => {
                return Response::error(
                    500,
                    "memo_operation_failed",
                    "memo image could not be saved",
                );
            }
        };
        let size = dir
            .entries()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|n| dir.metadata(&n).ok())
            .map(|m| m.len())
            .sum::<u64>();
        if size + request.body.len() as u64 > IMAGES_TOTAL {
            return Response::error(
                507,
                "memo_images_full",
                "memo images exceed 500 MB in total; delete memos you no longer need",
            );
        }
        let name = match random_hex(16) {
            Ok(id) => format!("{id}.{ext}"),
            Err(_) => {
                return Response::error(
                    500,
                    "memo_operation_failed",
                    "memo image could not be saved",
                );
            }
        };
        if dir.create_new(&name, &request.body, 0o600).is_err() {
            return Response::error(
                500,
                "memo_operation_failed",
                "memo image could not be saved",
            );
        }
        Response::json(200, &json!({"image":name}))
    }
    pub fn clean_orphan_images(&self, now: Timestamp) -> usize {
        let Ok(state) = self.state.lock() else {
            return 0;
        };
        if state.load_error {
            return 0;
        }
        let Ok(dir) = self.root.child_dir("memo-images", false) else {
            return 0;
        };
        let mut removed = 0;
        for name in dir.entries().unwrap_or_default() {
            if !valid_image_name(&name) || state.data.memos.iter().any(|m| m.images.contains(&name))
            {
                continue;
            }
            let old = dir
                .metadata(&name)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| Timestamp::from_system_time(t).ok())
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|d| d >= Duration::from_secs(86400));
            if old && dir.remove_file(&name).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}
pub fn valid_image_name(name: &str) -> bool {
    let Some((id, ext)) = name.split_once('.') else {
        return false;
    };
    id.len() == 32
        && id
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        && matches!(ext, "png" | "jpg" | "gif" | "webp")
}
fn image_mime(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("png") => "image/png",
        Some("jpg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 14 && &bytes[..4] == b"RIFF" && &bytes[8..14] == b"WEBPVP" {
        Some("webp")
    } else {
        None
    }
}
