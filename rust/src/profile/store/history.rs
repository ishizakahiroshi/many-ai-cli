use super::*;
use chrono::{Timelike, Utc};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub fn definition_digest(definition: &Definition) -> Result<String> {
    Ok(content_digest(&to_go_json(definition)?))
}
pub fn revision_id(provider: &str, parent: &str, digest: &str, created: &str) -> String {
    content_digest(format!("{provider}\n{parent}\n{digest}\n{created}").as_bytes())[..24].into()
}
fn now() -> String {
    let time = Utc::now();
    let mut out = time.format("%Y-%m-%dT%H:%M:%S").to_string();
    if time.nanosecond() != 0 {
        out.push('.');
        out.push_str(format!("{:09}", time.nanosecond()).trim_end_matches('0'));
    }
    out.push('Z');
    out
}
fn revision_component(revision: &str) -> Result<()> {
    crate::files::safe_fs::basename(revision)
        .map_err(|_| StoreError::Invalid("invalid revision".into()))
}
fn merge_object(mut base: Value, overlay: Value) -> Value {
    // The same recursive object/source replacement rule as registry::merge.
    for (key, value) in overlay.as_object().expect("typed definition object") {
        let slot = &mut base[key];
        if key != "source" && slot.is_object() && value.is_object() {
            *slot = merge_object(slot.take(), value.clone());
        } else {
            *slot = value.clone();
        }
    }
    base
}
fn merged(base: &Definition, overlay: &Definition) -> Result<Definition> {
    Ok(serde_json::from_value(merge_object(
        serde_json::to_value(base)?,
        serde_json::to_value(overlay)?,
    ))?)
}
fn validate_against(payload: &Definition, baseline: &Definition) -> Result<()> {
    let raw = to_go_json(&merged(baseline, payload)?)?;
    let (_, diagnostics) =
        validate_definition(&raw, &default_adapters()).map_err(StoreError::Decode)?;
    for d in diagnostics {
        if d.is_error() {
            return Err(StoreError::Invalid(format!(
                "override payload is invalid: {}: {}",
                d.field, d.message
            )));
        }
    }
    Ok(())
}
fn comparison(def: &Definition) -> Result<Map<String, Value>> {
    let mut def = def.clone();
    if def.enabled.is_none() {
        def.enabled = Some(true);
    }
    let mut object = serde_json::to_value(def)?
        .as_object()
        .expect("typed definition object")
        .clone();
    object.remove("source");
    Ok(object)
}
fn delta(base: &Map<String, Value>, desired: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in desired {
        if key == "id" {
            continue;
        }
        if let Some(previous) = base.get(key) {
            if let (Some(left), Some(right)) = (previous.as_object(), value.as_object()) {
                let nested = delta(left, right);
                if !nested.is_empty() {
                    out.insert(key.clone(), nested.into());
                }
                continue;
            }
            if previous == value {
                continue;
            }
        }
        out.insert(key.clone(), value.clone());
    }
    out
}
fn mismatches(
    prefix: &str,
    left: &Map<String, Value>,
    right: &Map<String, Value>,
    output: &mut Vec<String>,
) {
    let keys: BTreeSet<_> = left.keys().chain(right.keys()).collect();
    for key in keys {
        let field = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match (left.get(key), right.get(key)) {
            (Some(a), Some(b)) if a.is_object() && b.is_object() => mismatches(
                &field,
                a.as_object().unwrap(),
                b.as_object().unwrap(),
                output,
            ),
            (a, b) if a != b => output.push(field),
            _ => {}
        }
    }
}

#[derive(PartialEq, Eq)]
enum HeadStatus {
    Missing,
    Healthy,
    Broken,
}
pub struct HistoryStore {
    root: Root,
    backup: Root,
    gate: Mutex<()>,
    clock: fn() -> String,
}
impl HistoryStore {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            root: Root::new(paths.resource(Resource::ProviderOverrides)),
            backup: Root::new(paths.resource(Resource::ProviderBackups)),
            gate: Mutex::new(()),
            clock: now,
        }
    }
    pub fn current(&self, id: &str) -> Result<RevisionRecord> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        self.current_locked(id)
    }
    pub fn list_backups(&self, id: &str) -> Result<Vec<RevisionRecord>> {
        Ok(self.list_backups_nullable(id)?.unwrap_or_default())
    }
    pub fn list_backups_nullable(&self, id: &str) -> Result<Option<Vec<RevisionRecord>>> {
        validate_history_id(id)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        let dir = match self
            .backup
            .open(false)
            .and_then(|root| root.child_dir(id, false).map_err(Into::into))
        {
            Ok(dir) => dir,
            Err(error) if error.is_not_found() => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut records = vec![];
        for name in sorted_entries(&dir)? {
            if Path::new(&name)
                .extension()
                .is_some_and(|ext| ext == "json")
                && dir.child_dir(&name, false).is_err()
            {
                records.push(read_record(&dir, &name, id)?);
            }
        }
        Ok(Some(records))
    }
    pub fn verify_backup(&self, id: &str, backup: &str) -> Result<RevisionRecord> {
        validate_history_id(id)?;
        let name = backup_filename(backup)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        let dir = self.backup.open(false)?.child_dir(id, false)?;
        read_record(&dir, &name, id)
    }
    pub fn restore_backup(&self, id: &str, backup: &str, expected: &str) -> Result<RevisionRecord> {
        validate_history_id(id)?;
        let name = backup_filename(backup)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        let dir = self.backup.open(false)?.child_dir(id, false)?;
        let backup = read_record(&dir, &name, id)?;
        self.save_locked(id, backup.payload, expected, "restore")
    }
    pub fn last_verified_revision(&self, id: &str) -> Result<Option<RevisionRecord>> {
        validate_history_id(id)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        let dir = match self.root.open(false).and_then(|root| {
            root.child_dir(id, false)
                .and_then(|dir| dir.child_dir("revisions", false))
                .map_err(Into::into)
        }) {
            Ok(dir) => dir,
            Err(error) if error.is_not_found() => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut best: Option<RevisionRecord> = None;
        for name in sorted_entries(&dir)? {
            if Path::new(&name)
                .extension()
                .is_some_and(|ext| ext == "json")
                && let Ok(record) = read_record(&dir, &name, id)
                && best
                    .as_ref()
                    .is_none_or(|prior| record.created_at > prior.created_at)
            {
                best = Some(record);
            }
        }
        Ok(best)
    }
    fn head_status_locked(&self, id: &str) -> Result<HeadStatus> {
        validate_history_id(id)?;
        let dir = match self
            .root
            .open(false)
            .and_then(|root| root.child_dir(id, false).map_err(Into::into))
        {
            Ok(dir) => dir,
            Err(error) if error.is_not_found() => return Ok(HeadStatus::Missing),
            Err(error) => return Err(error),
        };
        match dir.metadata("HEAD") {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(HeadStatus::Missing);
            }
            Err(error) => return Err(error.into()),
        };
        Ok(if self.current_locked(id).is_ok() {
            HeadStatus::Healthy
        } else {
            HeadStatus::Broken
        })
    }
    pub fn needs_recovery(&self, id: &str) -> Result<bool> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        self.head_status_locked(id)
            .map(|status| status == HeadStatus::Broken)
    }
    pub fn recover_head(&self, id: &str, revision: &str) -> Result<RevisionRecord> {
        validate_history_id(id)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        match self.head_status_locked(id)? {
            HeadStatus::Missing => {
                return Err(StoreError::Invalid(
                    "override does not exist; nothing to recover".into(),
                ));
            }
            HeadStatus::Healthy => {
                return Err(StoreError::Invalid(
                    "override HEAD is readable; use Restore".into(),
                ));
            }
            HeadStatus::Broken => {}
        }
        let dir = match self
            .root
            .open(false)
            .and_then(|root| root.child_dir(id, false).map_err(Into::into))
        {
            Ok(dir) => dir,
            Err(error) if error.is_not_found() => {
                return Err(StoreError::Invalid(
                    "override does not exist; nothing to recover".into(),
                ));
            }
            Err(error) => return Err(error),
        };
        let raw = match dir.read("HEAD", usize::MAX) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::Invalid(
                    "override does not exist; nothing to recover".into(),
                ));
            }
            Err(error) => return Err(error.into()),
        };
        // Go QuarantineFile copies HEAD and intentionally leaves it in place until publication.
        let quarantine = self.backup.open(true)?.child_dir("quarantine", true)?;
        let name = format!("{}-HEAD", Utc::now().format("%Y%m%dT%H%M%S.%9fZ"));
        quarantine.create_new(&name, &raw, 0o600)?;
        let payload = if revision.is_empty() {
            Definition {
                id: id.into(),
                ..Default::default()
            }
        } else {
            self.read_revision_locked(id, revision)?.payload
        };
        let record = self.record(id, payload, "", "restore")?;
        let revisions = dir.child_dir("revisions", true)?;
        let name = format!("{}.json", record.revision);
        write_json(&revisions, &name, &record)?;
        let verified = read_record(&revisions, &name, id)?;
        if verified.content_digest != record.content_digest {
            return Err(StoreError::Decode(
                "verify revision: digest mismatch".into(),
            ));
        }
        recover(&dir, "HEAD");
        dir.replace("HEAD", format!("{}\n", record.revision).as_bytes(), 0o600)?;
        Ok(record)
    }
    pub fn reset(&self, id: &str, expected: &str) -> Result<RevisionRecord> {
        validate_history_id(id)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        self.save_locked(
            id,
            Definition {
                id: id.into(),
                ..Default::default()
            },
            expected,
            "reset",
        )
    }
    pub fn restore(&self, id: &str, revision: &str, expected: &str) -> Result<RevisionRecord> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        let selected = self.read_revision_locked(id, revision)?;
        self.save_locked(id, selected.payload, expected, "restore")
    }
    fn current_locked(&self, id: &str) -> Result<RevisionRecord> {
        validate_history_id(id)?;
        let root = self.root.open(false)?;
        let provider = root.child_dir(id, false)?;
        let head = read_recover(&provider, "HEAD", usize::MAX)?;
        let revision = String::from_utf8_lossy(&head);
        self.read_revision_locked(id, revision.trim())
    }
    pub fn get_revision(&self, id: &str, revision: &str) -> Result<RevisionRecord> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        self.read_revision_locked(id, revision)
    }
    fn read_revision_locked(&self, id: &str, revision: &str) -> Result<RevisionRecord> {
        validate_history_id(id)?;
        revision_component(revision)?;
        let dir = self
            .root
            .open(false)?
            .child_dir(id, false)?
            .child_dir("revisions", false)?;
        read_record(&dir, &format!("{revision}.json"), id)
    }
    pub fn load_overrides(&self) -> Result<LoadedDefinitions> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        let root = match self.root.open(false) {
            Ok(r) => r,
            Err(e) if e.is_not_found() => return Ok(LoadedDefinitions::empty()),
            Err(e) => return Err(e),
        };
        let mut loaded = LoadedDefinitions::empty();
        for id in sorted_entries(&root)? {
            let dir = match root.child_dir(&id, false) {
                Ok(dir) => dir,
                // os.ReadDir skips nondirectories; no raw path fallback.
                Err(e) if e.kind() == io::ErrorKind::NotADirectory => continue,
                Err(_) => continue,
            };
            if validate_history_id(&id).is_err() {
                loaded.diagnostics.push(diag(
                    "invalid_override_id",
                    "error",
                    &id,
                    "invalid override directory",
                ));
                continue;
            }
            match self.current_locked(&id) {
                Ok(mut record) => {
                    record.payload.source = SourceRef {
                        origin: ORIGIN_OVERRIDE.into(),
                        revision: record.revision,
                        digest: record.content_digest,
                        ..Default::default()
                    };
                    loaded.definitions.push(record.payload);
                }
                Err(_) => {
                    // Go preserves the bad HEAD and copies it to quarantine.
                    if let Ok(raw) = dir.read("HEAD", usize::MAX)
                        && let Ok(root) = self.backup.open(true)
                        && let Ok(quarantine) = root.child_dir("quarantine", true)
                    {
                        let name = format!("{}-HEAD", Utc::now().format("%Y%m%dT%H%M%S.%9fZ"));
                        let _ = quarantine.create_new(&name, &raw, 0o600);
                    }
                    loaded.diagnostics.push(diag(
                        "invalid_override",
                        "error",
                        &id,
                        "override HEAD could not be read",
                    ));
                }
            }
        }
        Ok(loaded)
    }
    pub fn backup_snapshot(
        &self,
        id: &str,
        payload: &Definition,
        reason: &str,
    ) -> Result<RevisionRecord> {
        validate_history_id(id)?;
        if payload.id != id {
            return Err(StoreError::Invalid(format!(
                "payload provider id does not match {id:?}"
            )));
        }
        let mut payload = payload.clone();
        payload.source = SourceRef::default();
        validate_against(&payload, &Definition::default())?;
        let record = self.record(
            id,
            payload,
            "",
            if reason.is_empty() {
                "snapshot"
            } else {
                reason
            },
        )?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        self.backup_locked(&record)?;
        Ok(record)
    }
    fn backup_locked(&self, record: &RevisionRecord) -> Result<()> {
        validate_history_id(&record.provider_id)?;
        revision_component(&record.revision)?;
        let dir = self
            .backup
            .open(true)?
            .child_dir(&record.provider_id, true)?;
        let name = format!("{}.json", record.revision);
        write_json(&dir, &name, record)?;
        let verified = read_record(&dir, &name, &record.provider_id)?;
        if verified.content_digest != record.content_digest {
            return Err(StoreError::Decode(
                "verify provider backup: digest mismatch".into(),
            ));
        }
        Ok(())
    }
    fn record(
        &self,
        id: &str,
        payload: Definition,
        parent: &str,
        reason: &str,
    ) -> Result<RevisionRecord> {
        let created_at = (self.clock)();
        let content_digest = definition_digest(&payload)?;
        Ok(RevisionRecord {
            schema_version: 1,
            provider_id: id.into(),
            revision: revision_id(id, parent, &content_digest, &created_at),
            created_at,
            reason: reason.into(),
            parent_revision: parent.into(),
            content_digest,
            payload,
        })
    }
    pub fn save_override(
        &self,
        id: &str,
        mut payload: Definition,
        baseline: &Definition,
        expected: &str,
        reason: &str,
    ) -> Result<RevisionRecord> {
        validate_history_id(id)?;
        if payload.id.is_empty() {
            payload.id = id.into();
        }
        if payload.id != id {
            return Err(StoreError::Invalid(format!(
                "payload provider id does not match {id:?}"
            )));
        }
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        let payload = match self.current_locked(id) {
            Ok(current) => merged(&current.payload, &payload)?,
            Err(e) if e.is_not_found() => payload,
            Err(e) => return Err(e),
        };
        validate_against(&payload, baseline)?;
        self.save_locked(
            id,
            payload,
            expected,
            if reason.is_empty() { "edit" } else { reason },
        )
    }
    pub fn save_effective_override(
        &self,
        id: &str,
        mut desired: Definition,
        baseline: &Definition,
        expected: &str,
        reason: &str,
    ) -> Result<RevisionRecord> {
        validate_history_id(id)?;
        if desired.id.is_empty() {
            desired.id = id.into();
        }
        if desired.id != id {
            return Err(StoreError::Invalid(format!(
                "payload provider id does not match {id:?}"
            )));
        }
        validate_against(&desired, &Definition::default())?;
        let mut base_object = serde_json::to_value(baseline)?.as_object().unwrap().clone();
        base_object.remove("source");
        let mut desired_object = serde_json::to_value(&desired)?.as_object().unwrap().clone();
        desired_object.remove("source");
        let mut object = delta(&base_object, &desired_object);
        object.insert("id".into(), id.into());
        let payload: Definition = serde_json::from_value(object.into())?;
        let mut fields = vec![];
        mismatches(
            "",
            &comparison(&merged(baseline, &payload)?)?,
            &comparison(&desired)?,
            &mut fields,
        );
        fields.sort();
        if !fields.is_empty() {
            return Err(StoreError::OverrideClearsValue(fields));
        }
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        validate_against(&payload, baseline)?;
        self.save_locked(
            id,
            payload,
            expected,
            if reason.is_empty() { "edit" } else { reason },
        )
    }
    fn save_locked(
        &self,
        id: &str,
        payload: Definition,
        expected: &str,
        reason: &str,
    ) -> Result<RevisionRecord> {
        let current = match self.current_locked(id) {
            Ok(r) => Some(r),
            Err(e) if e.is_not_found() => None,
            Err(e) => return Err(e),
        };
        let parent = current
            .as_ref()
            .map(|r| r.revision.as_str())
            .unwrap_or_default();
        if expected != parent {
            return Err(StoreError::RevisionConflict {
                expected: expected.into(),
                current: parent.into(),
            });
        }
        if let Some(current) = &current {
            self.backup_locked(current)?;
        }
        let record = self.record(id, payload, parent, reason)?;
        let dir = self.root.open(true)?.child_dir(id, true)?;
        let revisions = dir.child_dir("revisions", true)?;
        let name = format!("{}.json", record.revision);
        write_json(&revisions, &name, &record)?;
        let verified = read_record(&revisions, &name, id)?;
        if verified.content_digest != record.content_digest {
            return Err(StoreError::Decode(
                "verify revision: digest mismatch".into(),
            ));
        }
        recover(&dir, "HEAD");
        dir.replace("HEAD", format!("{}\n", record.revision).as_bytes(), 0o600)?;
        Ok(record)
    }
    pub fn list(&self, id: &str) -> Result<Option<Vec<RevisionRecord>>> {
        validate_history_id(id)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("history store lock poisoned"))?;
        let directory = self
            .root
            .open(false)
            .and_then(|root| Ok(root.child_dir(id, false)?.child_dir("revisions", false)?));
        let dir = match directory {
            Ok(d) => d,
            Err(e) if e.is_not_found() => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut records = vec![];
        for name in sorted_entries(&dir)? {
            if Path::new(&name).extension().is_some_and(|e| e == "json")
                && dir.child_dir(&name, false).is_err()
            {
                records.push(read_record(&dir, &name, id)?);
            }
        }
        Ok(Some(records))
    }
}
fn backup_filename(id: &str) -> Result<String> {
    if Path::new(id).file_name().and_then(|name| name.to_str()) != Some(id)
        || id.strip_suffix(".json").unwrap_or(id).is_empty()
        || id.contains(['/', '\\'])
    {
        return Err(StoreError::Invalid("invalid backup id".into()));
    }
    Ok(if Path::new(id).extension().is_none() {
        format!("{id}.json")
    } else {
        id.into()
    })
}
fn read_record(dir: &Dir, name: &str, expected_id: &str) -> Result<RevisionRecord> {
    let record = crate::proto::decode_wire::<RevisionRecord>(&read_recover(dir, name, usize::MAX)?)
        .map_err(|e| StoreError::Decode(format!("decode revision: {e}")))?;
    if record.schema_version != 1 || record.revision.is_empty() || record.content_digest.is_empty()
    {
        return Err(StoreError::Decode("invalid revision metadata".into()));
    }
    if record.revision != name.strip_suffix(".json").unwrap_or(name) {
        return Err(StoreError::Decode("revision id mismatch".into()));
    }
    if record.provider_id != expected_id {
        return Err(StoreError::Decode("revision provider id mismatch".into()));
    }
    if record.payload.id != expected_id {
        return Err(StoreError::Decode("revision payload id mismatch".into()));
    }
    if definition_digest(&record.payload)? != record.content_digest {
        return Err(StoreError::Decode(
            "revision content digest mismatch".into(),
        ));
    }
    let base = Definition {
        schema_version: 1,
        id: expected_id.into(),
        display_name: expected_id.into(),
        launch: Some(LaunchDefinition {
            executable: "validation-placeholder".into(),
            headless: Some(HeadlessDefinition {
                format: "validation-placeholder".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        update: Some(UpdateDefinition {
            args: vec!["validation-placeholder".into()],
            ..Default::default()
        }),
        ..Default::default()
    };
    validate_against(&record.payload, &base)
        .map_err(|e| StoreError::Decode(format!("revision payload schema is invalid: {e}")))?;
    Ok(record)
}
