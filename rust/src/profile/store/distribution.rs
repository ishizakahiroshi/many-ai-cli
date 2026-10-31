//! Three-way provider definition diff shared by the provider history API.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
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
