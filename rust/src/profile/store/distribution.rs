//! Fixed Go distribution pointer transactions. No download or trust bypass API.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
const LIMIT: usize = accepted::DISTRIBUTION_LIMIT;
pub struct DistributionStore {
    root: Root,
    gate: Mutex<()>,
}
pub struct DistributionPointerSnapshot {
    accepted: Option<Vec<u8>>,
    previous: Option<Vec<u8>>,
}
impl DistributionStore {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            root: Root::new(paths.resource(Resource::ProviderDistributions)),
            gate: Mutex::new(()),
        }
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>> {
        self.gate
            .lock()
            .map_err(|_| StoreError::Unavailable("distribution lock poisoned"))
    }
    fn status_locked(&self) -> Result<DistributionStatus> {
        let root = match self.root.open(false) {
            Ok(root) => root,
            Err(e) if e.is_not_found() => {
                return Ok(DistributionStatus {
                    state: "none".into(),
                    ..Default::default()
                });
            }
            Err(e) => return Err(e),
        };
        let Some(raw) = optional(&root, "accepted.json")? else {
            return Ok(DistributionStatus {
                state: "none".into(),
                ..Default::default()
            });
        };
        let mut status: DistributionStatus = crate::proto::decode_wire(&raw).map_err(|e| {
            StoreError::Decode(format!("decode accepted distribution pointer: {e}"))
        })?;
        if status.state.is_empty() {
            status.state = "accepted".into();
        }
        Ok(status)
    }
    pub fn status(&self) -> Result<DistributionStatus> {
        let _g = self.lock()?;
        self.status_locked()
    }
    pub fn load_downloaded(&self, digest: &str) -> Result<DistributionBundle> {
        let _g = self.lock()?;
        self.load("downloaded", digest)
    }
    pub fn load_accepted(&self) -> Result<Option<DistributionBundle>> {
        let _g = self.lock()?;
        let status = self.status_locked()?;
        if status.digest.is_empty() {
            return Ok(None);
        };
        self.load("accepted", &status.digest).map(Some)
    }
    fn load(&self, kind: &str, digest: &str) -> Result<DistributionBundle> {
        valid_digest(digest)?;
        let dir = self.root.open(false)?.child_dir(kind, false)?;
        let raw = read_recover(&dir, &format!("{digest}.json"), LIMIT + 1)?;
        validate_bundle(&raw, digest, kind)
    }
    pub fn snapshot_pointers(&self) -> Result<DistributionPointerSnapshot> {
        let _g = self.lock()?;
        let root = match self.root.open(false) {
            Ok(root) => root,
            Err(e) if e.is_not_found() => {
                return Ok(DistributionPointerSnapshot {
                    accepted: None,
                    previous: None,
                });
            }
            Err(e) => return Err(e),
        };
        Ok(DistributionPointerSnapshot {
            accepted: optional(&root, "accepted.json")?,
            previous: optional(&root, "previous.json")?,
        })
    }
    pub fn restore_pointers(&self, snapshot: DistributionPointerSnapshot) -> Result<()> {
        let _g = self.lock()?;
        let root = self.root.open(true)?;
        restore(&root, "accepted.json", snapshot.accepted)?;
        restore(&root, "previous.json", snapshot.previous)
    }
    pub fn rollback(&self) -> Result<DistributionStatus> {
        let _g = self.lock()?;
        let root = self.root.open(false)?;
        let previous = read_recover(&root, "previous.json", usize::MAX)?;
        let mut status: DistributionStatus = crate::proto::decode_wire(&previous).map_err(|e| {
            StoreError::Decode(format!("decode previous distribution pointer: {e}"))
        })?;
        if status.digest.is_empty() {
            return Err(StoreError::Invalid(
                "previous distribution pointer has no digest".into(),
            ));
        }
        if !status.state.is_empty() && status.state != "accepted" {
            return Err(StoreError::Invalid(format!(
                "previous distribution pointer has invalid state {:?}",
                status.state
            )));
        }
        self.load("accepted", &status.digest)?;
        if let Some(current) = optional(&root, "accepted.json")? {
            root.replace("rollback-before.json", &current, 0o600)?;
        }
        root.replace("accepted.json", &previous, 0o600)?;
        if status.state.is_empty() {
            status.state = "accepted".into();
        }
        Ok(status)
    }
}
fn optional(root: &Dir, name: &str) -> Result<Option<Vec<u8>>> {
    match read_recover(root, name, usize::MAX) {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e),
    }
}
fn restore(root: &Dir, name: &str, value: Option<Vec<u8>>) -> Result<()> {
    match value {
        Some(raw) => {
            root.replace(name, &raw, 0o600)?;
        }
        None => match root.remove_file(name) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        },
    };
    Ok(())
}
fn valid_digest(digest: &str) -> Result<()> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StoreError::Invalid(format!(
            "invalid distribution SHA-256 digest {digest:?}"
        )));
    }
    Ok(())
}
fn validate_bundle(raw: &[u8], digest: &str, label: &str) -> Result<DistributionBundle> {
    if raw.len() > LIMIT {
        return Err(StoreError::Decode(format!(
            "{label} distribution bundle exceeds {LIMIT} bytes"
        )));
    }
    if content_digest(raw) != digest {
        return Err(StoreError::Decode(format!(
            "{label} distribution content digest mismatch"
        )));
    }
    let bundle: DistributionBundle = crate::proto::decode_wire(raw)
        .map_err(|e| StoreError::Decode(format!("decode {label} distribution: {e}")))?;
    if bundle.payload.schema_version != CURRENT_SCHEMA_VERSION
        || bundle.payload.catalog_version.trim().is_empty()
    {
        return Err(StoreError::Decode(format!(
            "{label} distribution schema is invalid"
        )));
    }
    let definitions = bundle.payload.definitions.as_deref().unwrap_or_default();
    if definitions.len() > 256 {
        return Err(StoreError::Decode(format!(
            "{label} distribution contains too many definitions"
        )));
    }
    let mut seen = BTreeSet::new();
    for definition in definitions {
        if !seen.insert(&definition.id) {
            return Err(StoreError::Decode(format!(
                "{label} distribution contains duplicate provider {:?}",
                definition.id
            )));
        }
        if !validate_definition(&to_go_json(definition)?, &default_adapters())
            .is_ok_and(|(_, d)| !d.iter().any(Diagnostic::is_error))
        {
            return Err(StoreError::Decode(format!(
                "{label} distribution definition {:?} is invalid",
                definition.id
            )));
        }
        let actual_digest = definition_digest(definition)?;
        if bundle
            .payload
            .digests
            .as_ref()
            .and_then(|d| d.get(&definition.id))
            .is_none_or(|d| d.is_empty() || actual_digest != *d)
        {
            return Err(StoreError::Decode(format!(
                "{label} distribution digest mismatch for {:?}",
                definition.id
            )));
        }
    }
    Ok(bundle)
}
pub fn diff_distribution(
    current: &[Definition],
    candidate: &[Definition],
    overrides: &[Definition],
) -> Vec<DistributionProviderDiff> {
    fn index(defs: &[Definition]) -> BTreeMap<String, Definition> {
        defs.iter()
            .filter(|d| !d.id.is_empty())
            .map(|d| (d.id.clone(), d.clone()))
            .collect()
    }
    let current = index(current);
    let candidate = index(candidate);
    let overrides = index(overrides);
    current
        .keys()
        .chain(candidate.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|id| {
            let old = current.get(&id);
            let new = candidate.get(&id);
            let a = flatten(old);
            let b = flatten(new);
            let o = flatten(overrides.get(&id));
            let mut changes = vec![];
            for field in a
                .keys()
                .chain(b.keys())
                .chain(o.keys())
                .cloned()
                .collect::<BTreeSet<_>>()
            {
                if field == "id" {
                    continue;
                }
                let cur = a.get(&field).cloned().unwrap_or_default();
                let cand = b.get(&field).cloned().unwrap_or_default();
                let over = o.get(&field).cloned().unwrap_or_default();
                let has = o.contains_key(&field);
                if cur == cand && (!has || over == cur) {
                    continue;
                }
                changes.push(DistributionFieldDiff {
                    field,
                    current: cur.clone(),
                    candidate: if cur == cand {
                        String::new()
                    } else {
                        cand.clone()
                    },
                    r#override: over.clone(),
                    conflict: cur != cand && has && over != cand && over != cur,
                });
            }
            let status = if old.is_none() {
                "added"
            } else if new.is_none() {
                "removed"
            } else if !changes.is_empty() {
                "changed"
            } else {
                "unchanged"
            };
            DistributionProviderDiff {
                id,
                status: status.into(),
                changes,
            }
        })
        .collect()
}
fn flatten(def: Option<&Definition>) -> BTreeMap<String, String> {
    fn walk(out: &mut BTreeMap<String, String>, prefix: String, v: serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    walk(
                        out,
                        if prefix.is_empty() {
                            k
                        } else {
                            format!("{prefix}.{k}")
                        },
                        v,
                    )
                }
            }
            serde_json::Value::String(s) => {
                out.insert(prefix, s);
            }
            v => {
                if !prefix.is_empty() {
                    out.insert(
                        prefix,
                        String::from_utf8(to_go_json(&v).expect("JSON values serialize"))
                            .expect("JSON is UTF8"),
                    );
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(
        &mut out,
        String::new(),
        serde_json::to_value(def.cloned().unwrap_or_default()).expect("provider DTO serializes"),
    );
    out
}

#[cfg(test)]
#[path = "distribution/tests.rs"]
mod tests;
