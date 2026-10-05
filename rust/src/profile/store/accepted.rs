use super::*;
use std::collections::BTreeSet;

pub const DISTRIBUTION_LIMIT: usize = 2 * 1024 * 1024;
/// Reads only bundles already accepted by Go/Rust's separate trust workflow.
/// This intentionally verifies storage integrity, not an external signature.
pub struct AcceptedDistributionStore {
    root: Root,
    gate: Mutex<()>,
}
impl AcceptedDistributionStore {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            root: Root::new(paths.resource(Resource::ProviderDistributions)),
            gate: Mutex::new(()),
        }
    }
    pub fn load_accepted(&self) -> Result<Option<DistributionBundle>> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("distribution store lock poisoned"))?;
        let root = match self.root.open(false) {
            Ok(r) => r,
            Err(e) if e.is_not_found() => return Ok(None),
            Err(e) => return Err(e),
        };
        let raw = match read_recover(&root, "accepted.json", usize::MAX) {
            Ok(r) => r,
            Err(e) if e.is_not_found() => return Ok(None),
            Err(e) => return Err(e),
        };
        let status = crate::proto::decode_wire::<DistributionStatus>(&raw).map_err(|e| {
            StoreError::Decode(format!("decode accepted distribution pointer: {e}"))
        })?;
        if status.digest.is_empty() {
            return Ok(None);
        }
        if status.digest.len() != 64
            || !status
                .digest
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(StoreError::Decode(format!(
                "invalid distribution SHA-256 digest {:?}",
                status.digest
            )));
        }
        let accepted = root.child_dir("accepted", false)?;
        let bytes = read_recover(
            &accepted,
            &format!("{}.json", status.digest),
            DISTRIBUTION_LIMIT + 1,
        )?;
        if bytes.len() > DISTRIBUTION_LIMIT {
            return Err(StoreError::Decode(format!(
                "accepted distribution bundle exceeds {DISTRIBUTION_LIMIT} bytes"
            )));
        }
        if content_digest(&bytes) != status.digest {
            return Err(StoreError::Decode(
                "accepted distribution content digest mismatch".into(),
            ));
        }
        let bundle = crate::proto::decode_wire::<DistributionBundle>(&bytes)
            .map_err(|e| StoreError::Decode(format!("decode accepted distribution: {e}")))?;
        if bundle.payload.schema_version != CURRENT_SCHEMA_VERSION
            || bundle.payload.catalog_version.trim().is_empty()
        {
            return Err(StoreError::Decode(
                "accepted distribution schema is invalid".into(),
            ));
        }
        let definitions = bundle.payload.definitions.as_deref().unwrap_or_default();
        if definitions.len() > 256 {
            return Err(StoreError::Decode(
                "accepted distribution contains too many definitions".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for definition in definitions {
            if !seen.insert(&definition.id) {
                return Err(StoreError::Decode(format!(
                    "accepted distribution contains duplicate provider {:?}",
                    definition.id
                )));
            }
            let raw = to_go_json(definition)?;
            if !validate_definition(&raw, &default_adapters())
                .is_ok_and(|(_, diagnostics)| !diagnostics.iter().any(Diagnostic::is_error))
            {
                return Err(StoreError::Decode(format!(
                    "accepted distribution definition {:?} is invalid",
                    definition.id
                )));
            }
            let digest = definition_digest(definition)?;
            if bundle
                .payload
                .digests
                .as_ref()
                .and_then(|d| d.get(&definition.id))
                .is_none_or(|d| d.is_empty() || *d != digest)
            {
                return Err(StoreError::Decode(format!(
                    "accepted distribution digest mismatch for {:?}",
                    definition.id
                )));
            }
        }
        Ok(Some(bundle))
    }
    pub fn load_definitions(&self) -> LoadedDefinitions {
        match self.load_accepted() {
            Ok(Some(bundle)) => LoadedDefinitions {
                definitions: bundle.payload.definitions.unwrap_or_default(),
                diagnostics: vec![],
            },
            Ok(None) => LoadedDefinitions::empty(),
            Err(e) => LoadedDefinitions {
                definitions: vec![],
                diagnostics: vec![diag(
                    "distribution_accepted_load_failed",
                    "error",
                    "distribution",
                    &e.to_string(),
                )],
            },
        }
    }
}
