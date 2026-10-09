use super::*;
fn invalid() -> io::Error {
    io::Error::other("invalid model catalog")
}
fn humanize(id: &str) -> String {
    let id = id.trim().strip_prefix("opencode/").unwrap_or(id.trim());
    if id.is_empty() {
        return "OpenCode".into();
    }
    id.split(['-', '_', '/'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            let first = chars.next().unwrap().to_uppercase().to_string();
            first + &chars.as_str().to_lowercase()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub(super) fn native(bytes: &[u8], provider: &str) -> io::Result<Vec<Model>> {
    if provider == "opencode" {
        return opencode(bytes);
    }
    let ansi = regex::Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").unwrap();
    let valid = regex::Regex::new(if provider == "grok" {
        r"(?i)^grok-[a-z0-9][a-z0-9._-]*$"
    } else {
        r"(?i)^(?:auto|[a-z0-9]+(?:[._-][a-z0-9][a-z0-9._-]*)*)$"
    })
    .unwrap();
    let plain = ansi
        .replace_all(&String::from_utf8_lossy(bytes), "")
        .into_owned();
    let mut models = Vec::new();
    let mut seen = BTreeSet::new();
    for line in plain.lines() {
        let line = line
            .trim()
            .trim_start_matches([' ', '\t', '>', '*', '•', '○', '●', '✓']);
        let (id, label) = if let Some((id, label)) = line.split_once(" - ") {
            (id.trim(), label.trim())
        } else if line.split_whitespace().count() == 1 {
            (line, "")
        } else {
            continue;
        };
        if !valid.is_match(id) || !seen.insert(id.to_owned()) {
            continue;
        }
        let label = if !label.is_empty() {
            label.into()
        } else if provider == "grok" {
            format!("Grok {}", id.to_lowercase().strip_prefix("grok-").unwrap())
        } else {
            humanize(id)
        };
        models.push(Model {
            id: id.into(),
            label,
            ..Default::default()
        });
    }
    if models.is_empty() {
        Err(invalid())
    } else {
        Ok(models)
    }
}
fn flush(models: &mut Vec<Model>, seen: &mut BTreeSet<String>, pending: &mut String) {
    if pending.is_empty() {
        return;
    }
    let full = if pending.contains('/') {
        pending.clone()
    } else {
        format!("opencode/{pending}")
    };
    if seen.insert(full.clone()) {
        models.push(Model {
            id: full,
            label: humanize(pending),
            ..Default::default()
        });
    }
    pending.clear();
}
fn opencode(bytes: &[u8]) -> io::Result<Vec<Model>> {
    let plain = String::from_utf8_lossy(bytes);
    let lines: Vec<_> = plain.lines().collect();
    let mut models = Vec::new();
    let mut seen = BTreeSet::new();
    let mut pending = String::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        index += 1;
        if let Some(id) = line.strip_prefix("opencode/") {
            flush(&mut models, &mut seen, &mut pending);
            pending = id.trim().into();
            continue;
        }
        if !line.starts_with('{') {
            continue;
        }
        let mut object = line.to_owned();
        let mut depth = line.matches('{').count() as isize - line.matches('}').count() as isize;
        while depth > 0 && index < lines.len() {
            let next = lines[index].trim();
            index += 1;
            object.push('\n');
            object.push_str(next);
            depth += next.matches('{').count() as isize - next.matches('}').count() as isize;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&object) else {
            flush(&mut models, &mut seen, &mut pending);
            continue;
        };
        if !value.is_null() && !value.is_object() {
            flush(&mut models, &mut seen, &mut pending);
            continue;
        }
        if ["id", "providerID", "name", "status"]
            .iter()
            .any(|key| !value[*key].is_null() && !value[*key].is_string())
        {
            flush(&mut models, &mut seen, &mut pending);
            continue;
        }
        let id = value["id"].as_str().unwrap_or("").trim();
        let id = if id.is_empty() { pending.as_str() } else { id };
        if id.is_empty() {
            continue;
        }
        let full = if id.contains('/') {
            id.into()
        } else {
            format!("opencode/{id}")
        };
        if seen.contains(&full) {
            pending.clear();
            continue;
        }
        let status = value["status"].as_str().unwrap_or("").trim();
        if !status.is_empty() && !status.eq_ignore_ascii_case("active") {
            pending.clear();
            continue;
        }
        let label = value["name"].as_str().unwrap_or("").trim();
        let label = if label.is_empty() {
            humanize(&pending)
        } else {
            label.into()
        };
        seen.insert(full.clone());
        models.push(Model {
            id: full,
            label,
            ..Default::default()
        });
        pending.clear();
    }
    flush(&mut models, &mut seen, &mut pending);
    Ok(models)
}
pub(super) fn nvidia(bytes: &[u8]) -> io::Result<Vec<Model>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let items = value["data"].as_array().ok_or_else(invalid)?;
    let mut models = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        let id = item["id"].as_str().ok_or_else(invalid)?.trim();
        if id.is_empty()
            || id.len() > 256
            || id.split('/').any(|part| {
                part.is_empty()
                    || matches!(part, "." | "..")
                    || !part.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
                    })
            })
            || !seen.insert(id.to_owned())
        {
            return Err(invalid());
        }
        models.push(Model {
            id: format!("nvidia/{id}"),
            label: id.into(),
            ..Default::default()
        });
    }
    Ok(models)
}
pub(super) fn defaults(bytes: &[u8]) -> io::Result<BTreeMap<String, Vec<Model>>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if value.is_null() {
        return Ok(BTreeMap::new());
    }
    let mut out = BTreeMap::new();
    for (key, items) in value.as_object().ok_or_else(invalid)? {
        let mut models = Vec::new();
        if !items.is_null() {
            for item in items.as_array().ok_or_else(invalid)? {
                if !item.is_null() && !item.is_object() {
                    return Err(invalid());
                }
                let mut model = Model::default();
                for (field, destination) in [("id", &mut model.id), ("label", &mut model.label)] {
                    if !item[field].is_null() {
                        *destination = item[field].as_str().ok_or_else(invalid)?.into();
                    }
                }
                models.push(model);
            }
        }
        out.insert(key.clone(), models);
    }
    Ok(out)
}
