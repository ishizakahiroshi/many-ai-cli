use super::{
    FolderTrustResult,
    project::{self, Plan},
};
use crate::files::safe_fs::Dir;
use std::{
    collections::BTreeMap,
    io::{self, Read, Seek, Write},
    path::Path,
};
use toml_edit::DocumentMut;
const LIMIT: usize = 16 * 1024 * 1024;
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Codex folder trust configuration is invalid",
    )
}
fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !file.metadata()?.is_file() {
        return Err(invalid());
    }
    let mut bytes = vec![];
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > LIMIT {
        return Err(invalid());
    }
    Ok(Some(bytes))
}
fn entries(bytes: &[u8]) -> io::Result<BTreeMap<String, String>> {
    let document = std::str::from_utf8(bytes)
        .map_err(|_| invalid())?
        .parse::<DocumentMut>()
        .map_err(|_| invalid())?;
    let mut projects = BTreeMap::new();
    if let Some(table) = document.get("projects") {
        for (key, entry) in table.as_table_like().ok_or_else(invalid)?.iter() {
            let entry = entry.as_table_like().ok_or_else(invalid)?;
            let level = match entry.get("trust_level") {
                None => "",
                Some(value) => value.as_str().ok_or_else(invalid)?,
            };
            projects.insert(key.into(), level.into());
        }
    }
    Ok(projects)
}
fn result(key: &str, level: &str) -> FolderTrustResult {
    FolderTrustResult {
        key: key.into(),
        existing: match level {
            "trusted" | "untrusted" => level.into(),
            _ => "other".into(),
        },
        ..Default::default()
    }
}
pub(super) fn grant(path: &Path, plan: &Plan) -> io::Result<FolderTrustResult> {
    let original = read(path)?;
    let projects = entries(original.as_deref().unwrap_or_default())?;
    for key in &plan.lookup {
        if let Some(level) = projects.get(key) {
            return Ok(result(key, level));
        }
        if let Some((matched, level)) = projects
            .iter()
            .find(|(candidate, _)| project::normalize(candidate) == *key)
        {
            return Ok(result(matched, level));
        }
    }
    for (key, level) in &projects {
        if level == "untrusted"
            && plan
                .guard
                .iter()
                .any(|guard| guard.starts_with(project::comparable(key)))
        {
            return Ok(result(key, level));
        }
    }
    if let Some(level) = projects.get(&plan.write_key) {
        return Ok(result(&plan.write_key, level));
    }
    let target = path.canonicalize().unwrap_or_else(|_| path.into());
    let directory = Dir::open_or_create_private(target.parent().ok_or_else(invalid)?)?;
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(invalid)?;
    let key = &plan.write_key;
    let quoted = if !key.contains('\'') {
        format!("'{key}'")
    } else {
        format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
    };
    let block = format!(
        "{}[projects.{quoted}]\ntrust_level = \"trusted\"\n",
        if original.as_ref().is_none_or(Vec::is_empty) {
            ""
        } else {
            "\n"
        }
    );
    let mut file = directory.open_append(name)?;
    let before = file.seek(std::io::SeekFrom::End(0))?;
    if let Err(error) = file.write_all(block.as_bytes()) {
        let _ = file.set_len(before);
        return Err(error);
    }
    file.sync_all()?;
    drop(file);
    let verify = read(path)
        .and_then(|bytes| entries(bytes.as_deref().unwrap_or_default()))
        .and_then(|entries| {
            if entries.get(key).is_some_and(|level| level == "trusted") {
                Ok(())
            } else {
                Err(io::Error::other("Codex folder trust verification failed"))
            }
        });
    if let Err(error) = verify {
        // Never truncate a concurrent writer's bytes. Source rollback is allowed
        // only while the exact append still forms the end of this same file.
        if let Ok(Some(current)) = read(path)
            && current.len() as u64 == before + block.len() as u64
            && current.ends_with(block.as_bytes())
        {
            if original.is_none() {
                let _ = directory.remove_file(name);
            } else if let Ok(file) = directory.open_file(name, true) {
                let _ = file.set_len(before);
            }
        }
        return Err(error);
    }
    Ok(FolderTrustResult {
        written: true,
        key: key.clone(),
        ..Default::default()
    })
}
