use super::{
    Result,
    content::{CONTENT_MAX, text_file},
    err,
    safe_fs::Dir,
    scope::{self, Scope},
    time,
};
use crate::{
    hub::http::{Request, Response},
    proto::wire::{Field, GoWire, Schema},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
#[serde(default)]
struct Mutation {
    dir: String,
    name: String,
    path: String,
    src: String,
    #[serde(rename = "srcs", deserialize_with = "null_vec")]
    srcs: Vec<String>,
    #[serde(rename = "dstDir")]
    dst_dir: String,
    #[serde(rename = "newName")]
    new_name: String,
    content: String,
    #[serde(rename = "baseMtime")]
    base_mtime: String,
}
impl Default for Mutation {
    fn default() -> Self {
        Self {
            dir: String::new(),
            name: String::new(),
            path: String::new(),
            src: String::new(),
            srcs: Vec::new(),
            dst_dir: String::new(),
            new_name: String::new(),
            content: String::new(),
            base_mtime: "0001-01-01T00:00:00Z".into(),
        }
    }
}
fn null_vec<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(d)?.unwrap_or_default())
}
macro_rules! body_schema {
    ($name:ident,$go:literal,{$($key:literal:$kind:literal),*}) => {
        #[derive(Deserialize)] #[serde(transparent)] struct $name(Mutation);
        impl GoWire for $name { const GO_TYPE:&'static str=$go; const SCHEMAS:&'static[Schema]=&[Schema{name:$go,fields:&[$(Field{name:$key,kind:$kind}),*]}]; }
    };
}
body_schema!(CreateBody,"FilesCreate",{"dir":"string","name":"string"});
body_schema!(SaveBody,"FilesSave",{"path":"string","content":"string","baseMtime":"time.Time"});
body_schema!(RenameBody,"FilesRename",{"src":"string","newName":"string"});
body_schema!(MoveBody,"FilesMove",{"src":"string","srcs":"[]string","dstDir":"string"});
body_schema!(DeleteBody,"FilesDelete",{"src":"string"});
pub fn handle(r: &Request, s: &Scope) -> Result<Response> {
    let save = r.path == "/api/files-save";
    let response = run(r, s, save);
    if save {
        response.map_err(|mut e| {
            if let Ok(Value::Object(mut obj)) = serde_json::from_slice(&e.body) {
                obj.entry("mtime").or_insert(json!("0001-01-01T00:00:00Z"));
                e.body = serde_json::to_vec(&obj).unwrap();
                e.body.push(b'\n');
            }
            e
        })
    } else {
        response
    }
}
fn run(r: &Request, s: &Scope, save: bool) -> Result<Response> {
    if save && r.body.len() > 2 * 1024 * 1024 {
        return Err(err(413, "too_large", "request body exceeds limit"));
    }
    let body = &r.body[..r.body.len().min(1024 * 1024)];
    let q = match r.path.as_str() {
        "/api/files-save" => crate::proto::decode_wire::<SaveBody>(&r.body).map(|v| v.0),
        "/api/files-create" | "/api/files-mkdir" => {
            crate::proto::decode_http_json::<CreateBody>(body).map(|v| v.0)
        }
        "/api/files-rename" => crate::proto::decode_http_json::<RenameBody>(body).map(|v| v.0),
        "/api/files-move" => crate::proto::decode_http_json::<MoveBody>(body).map(|v| v.0),
        "/api/files-delete-dir" => crate::proto::decode_http_json::<DeleteBody>(body).map(|v| v.0),
        _ => return Err(err(404, "not_found", "not found")),
    }
    .map_err(|_| err(400, "bad_request", "invalid json"))?;
    match r.path.as_str() {
        "/api/files-save" => save_file(q, s),
        "/api/files-create" | "/api/files-mkdir" => create(q, s, r.path == "/api/files-mkdir"),
        "/api/files-rename" => rename(q, s),
        "/api/files-delete-dir" => delete(q, s),
        "/api/files-move" => move_files(q, s),
        _ => Err(err(404, "not_found", "not found")),
    }
}
fn required_abs(value: &str, name: &str) -> Result<PathBuf> {
    if value.is_empty() {
        return Err(err(400, "bad_request", &format!("{name} is required")));
    }
    let p = Path::new(value);
    if !p.is_absolute() {
        return Err(err(
            400,
            "bad_request",
            &format!("{name} must be an absolute path"),
        ));
    }
    Ok(scope::clean(p))
}
fn safe_name(name: &str, detail: &str) -> Result<()> {
    super::safe_fs::basename(name).map_err(|_| err(400, "bad_request", detail))
}
fn save_file(q: Mutation, s: &Scope) -> Result<Response> {
    let base = if q.base_mtime == "0001-01-01T00:00:00Z" {
        None
    } else {
        Some(time::parse(&q.base_mtime).ok_or_else(|| err(400, "bad_request", "invalid json"))?)
    };
    let path = required_abs(&q.path, "path")?;
    let (dir, name) = s.write_parent(&path)?;
    if !text_file(&path) {
        return Err(err(403, "forbidden", "not a previewable text file"));
    }
    if q.content.len() > CONTENT_MAX {
        return Err(err(413, "too_large", "content exceeds 1 MiB limit"));
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
        return Err(err(400, "bad_request", "path is a directory"));
    }
    let mut file = dir.open_file(&name, true).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            err(
                404,
                "not_found",
                "file not found (new file creation is not supported)",
            )
        } else {
            super::io_error(e, "open_failed")
        }
    })?;
    let meta = file
        .metadata()
        .map_err(|e| super::io_error(e, "internal_error"))?;
    let current = meta
        .modified()
        .map_err(|e| super::io_error(e, "internal_error"))?;
    if base.is_some_and(|base| base != current) {
        return Err(Response::json(
            409,
            &json!({"ok":false,"error":"conflict","detail":"file was modified by another process","mtime":time::format_utc(current)?}),
        ));
    }
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.write_all(q.content.as_bytes()))
        .and_then(|_| file.set_len(q.content.len() as u64))
        .and_then(|_| file.sync_all())
        .map_err(|e| super::io_error(e, "write_failed"))?;
    let meta = file
        .metadata()
        .map_err(|e| super::io_error(e, "stat_after_write_failed"))?;
    let mut response = json!({"ok":true,"path":path,"mtime":time::format(meta.modified().map_err(|e|super::io_error(e,"stat_after_write_failed"))?)?});
    if meta.len() != 0 {
        response["size"] = json!(meta.len());
    }
    Ok(Response::json(200, &response))
}
fn create(q: Mutation, s: &Scope, mkdir: bool) -> Result<Response> {
    if q.dir.is_empty() || q.name.is_empty() {
        return Err(err(400, "bad_request", "dir and name are required"));
    }
    let dir = required_abs(&q.dir, "dir")?;
    safe_name(
        &q.name,
        if mkdir {
            "name must be a plain directory name without path separators"
        } else {
            "name must be a plain file name without path separators"
        },
    )?;
    let path = dir.join(&q.name);
    let (cap, name) = s.write_parent(&path)?;
    if !mkdir && !text_file(&path) {
        return Err(err(403, "not_previewable", "not a previewable text file"));
    }
    let op = if mkdir {
        if cap
            .entries()
            .map_err(|e| super::io_error(e, "mkdir_failed"))?
            .contains(&name)
        {
            return Err(err(
                409,
                "already_exists",
                &format!("target already exists: {}", path.display()),
            ));
        }
        cap.mkdir_new(&name)
    } else {
        cap.create_new(&name, &[], 0o644)
    };
    op.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            err(
                409,
                "already_exists",
                &format!("target already exists: {}", path.display()),
            )
        } else {
            super::io_error(
                e,
                if mkdir {
                    "mkdir_failed"
                } else {
                    "create_failed"
                },
            )
        }
    })?;
    Ok(Response::json(200, &json!({"ok":true,"newAbs":path})))
}
fn protect_source(path: &Path, s: &Scope, verb: &str) -> Result<()> {
    if !s.allowed_write(path) {
        return Err(err(403, "forbidden", "src is outside allowed roots"));
    }
    // Missing targets cannot establish root identity; matches Go's fail-closed check.
    if std::fs::metadata(path).is_err() {
        return Err(err(
            409,
            "conflict",
            "cannot establish allowed root identity",
        ));
    }
    if s.root_itself(path) {
        return Err(err(
            409,
            "conflict",
            &format!("refusing to {verb} an allowed root directory"),
        ));
    }
    if scope::vcs_path(path) {
        return Err(err(
            403,
            "forbidden",
            &format!("refusing to {verb} a version control path"),
        ));
    }
    Ok(())
}
fn rename(q: Mutation, s: &Scope) -> Result<Response> {
    if q.src.is_empty() || q.new_name.is_empty() {
        return Err(err(400, "bad_request", "src and newName are required"));
    }
    let src = required_abs(&q.src, "src")?;
    safe_name(
        &q.new_name,
        "newName must be a plain file name without path separators",
    )?;
    protect_source(&src, s, "rename")?;
    if scope::vcs_path(Path::new(&q.new_name)) {
        return Err(err(
            403,
            "forbidden",
            "refusing to rename to a version control path",
        ));
    }
    let (cap, name) = s.write_entry_parent(&src)?;
    if name == q.new_name {
        return Err(err(409, "conflict", "newName is identical to current name"));
    }
    cap.rename_to(&name, &cap, &q.new_name)
        .map_err(|e| super::io_error(e, "rename_failed"))?;
    Ok(Response::json(
        200,
        &json!({"ok":true,"newAbs":src.with_file_name(q.new_name)}),
    ))
}
fn delete(q: Mutation, s: &Scope) -> Result<Response> {
    let src = required_abs(&q.src, "src")?;
    protect_source(&src, s, "delete")?;
    let (cap, name) = s.write_entry_parent(&src)?;
    cap.child_dir(&name, false)
        .map_err(|_| err(400, "bad_request", "src must be a directory"))?;
    cap.remove_tree(&name)
        .map_err(|e| super::io_error(e, "delete_failed"))?;
    Ok(Response::json(200, &json!({"ok":true})))
}
struct MovePlan {
    src: String,
    parent: Dir,
    name: String,
    target: PathBuf,
    is_dir: bool,
}
fn move_plan(src: &str, dst: &Path, s: &Scope) -> std::result::Result<MovePlan, String> {
    let p = required_abs(src, "src").map_err(detail)?;
    protect_source(&p, s, "move").map_err(detail)?;
    if p.parent() == Some(dst) {
        return Err("src is already in dstDir".into());
    }
    let info = std::fs::symlink_metadata(&p).map_err(|e| format!("src not found: {e}"))?;
    if info.is_dir() && dst.starts_with(&p) {
        return Err("cannot move a directory into itself or its descendant".into());
    }
    let target = dst.join(p.file_name().unwrap());
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(format!("target already exists: {}", target.display()));
    }
    let (parent, name) = s.write_entry_parent(&p).map_err(detail)?;
    Ok(MovePlan {
        src: src.into(),
        parent,
        name,
        target,
        is_dir: info.is_dir(),
    })
}
fn detail(e: Response) -> String {
    let v: Value = serde_json::from_slice(&e.body).unwrap_or_default();
    let d = v["detail"].as_str().unwrap_or("operation failed");
    if e.status == 403 {
        format!("forbidden: {d}")
    } else if e.status == 409 {
        format!("conflict: {d}")
    } else {
        d.into()
    }
}
fn move_files(q: Mutation, s: &Scope) -> Result<Response> {
    let dst = required_abs(&q.dst_dir, "dstDir")?;
    if !s.allowed_write(&dst) {
        return Err(err(403, "forbidden", "dstDir is outside allowed roots"));
    }
    if scope::vcs_path(&dst) {
        return Err(err(
            403,
            "forbidden",
            "dstDir cannot be a version control directory",
        ));
    }
    let canon = scope::canonical(&dst).map_err(|e| super::io_error(e, "not_found"))?;
    if !scope::under_roots(&canon, &s.write_roots()) || scope::vcs_path(&canon) {
        return Err(err(403, "forbidden", "dstDir is outside allowed roots"));
    }
    let target = Dir::open(&canon).map_err(|e| super::io_error(e, "open_failed"))?;
    let multi = !q.srcs.is_empty();
    let sources = if multi {
        q.srcs
    } else {
        if q.src.is_empty() {
            return Err(err(400, "bad_request", "src or srcs is required"));
        }
        vec![q.src]
    };
    let mut plans = Vec::new();
    let mut results: Vec<Value> = sources.iter().map(|src| json!({"src":src})).collect();
    let mut good = true;
    for (i, src) in sources.iter().enumerate() {
        match move_plan(src, &dst, s) {
            Ok(p) => plans.push(Some(p)),
            Err(e) => {
                results[i]["error"] = json!(e);
                plans.push(None);
                good = false;
            }
        }
    }
    for i in 0..plans.len() {
        for j in 0..i {
            if let (Some(a), Some(b)) = (&plans[i], &plans[j]) {
                let msg = if a.src == b.src {
                    "duplicate source path"
                } else if a.target == b.target {
                    "multiple sources would overwrite the same target"
                } else if (a.is_dir && Path::new(&b.src).starts_with(&a.src))
                    || (b.is_dir && Path::new(&a.src).starts_with(&b.src))
                {
                    "cannot move a directory together with one of its descendants"
                } else {
                    continue;
                };
                results[i]["error"] = json!(msg);
                results[j]["error"] = json!(msg);
                good = false;
            }
        }
    }
    if !good {
        if multi {
            return Err(Response::json(
                400,
                &json!({"ok":false,"error":"move_preflight_failed","detail":"move preflight failed","results":results}),
            ));
        }
        let msg = results[0]["error"].as_str().unwrap();
        let (status, code) = operation_error(msg);
        return Err(err(status, code, msg));
    }
    let plans: Vec<_> = plans.into_iter().flatten().collect();
    for (i, p) in plans.iter().enumerate() {
        if let Err(e) = p.parent.rename_to(&p.name, &target, &p.name) {
            results[i]["error"] = json!(format!("rename failed: {e}"));
            let mut detail = format!("rename failed: {e}");
            for (j, done) in plans[..i].iter().enumerate().rev() {
                if let Err(e) = target.rename_to(&done.name, &done.parent, &done.name) {
                    let msg = format!(
                        "rollback failed; moved data remains at {}: {e}",
                        done.target.display()
                    );
                    results[j]["error"] = json!(msg);
                    results[j]["newAbs"] = json!(done.target);
                    detail.push_str(&format!("; {msg}"));
                }
            }
            return Err(if multi {
                Response::json(
                    400,
                    &json!({"ok":false,"error":"rename_failed","detail":detail,"results":results}),
                )
            } else {
                err(500, "operation_failed", &detail)
            });
        }
    }
    if multi {
        for (r, p) in results.iter_mut().zip(&plans) {
            r["newAbs"] = json!(p.target);
        }
        Ok(Response::json(200, &json!({"ok":true,"results":results})))
    } else {
        Ok(Response::json(
            200,
            &json!({"ok":true,"newAbs":plans[0].target}),
        ))
    }
}
fn operation_error(msg: &str) -> (u16, &'static str) {
    let m = msg.to_lowercase();
    if m.contains("forbidden") {
        (403, "forbidden")
    } else if m.contains("not found") {
        (404, "not_found")
    } else if [
        "already exists",
        "already in",
        "duplicate",
        "overwrite",
        "descendant",
        "conflict",
    ]
    .iter()
    .any(|s| m.contains(s))
    {
        (409, "conflict")
    } else if m.contains("failed") {
        (500, "operation_failed")
    } else {
        (400, "bad_request")
    }
}
