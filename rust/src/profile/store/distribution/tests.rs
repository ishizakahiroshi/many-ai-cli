use super::*;
fn paths(temp: &tempfile::TempDir) -> RuntimePaths {
    let run = temp.path().join("runtime");
    let old = temp.path().join("installed");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::create_dir_all(&old).unwrap();
    RuntimePaths::trial(&run, 49283, &old).unwrap()
}
fn definition(name: &str) -> Definition {
    Definition {
        schema_version: 1,
        id: "fixture".into(),
        display_name: name.into(),
        launch: Some(LaunchDefinition {
            executable: "synthetic-agent".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn seed(root: &Dir, kind: &str, name: &str) -> DistributionStatus {
    let def = definition(name);
    let bundle = DistributionBundle {
        payload: DistributionPayload {
            schema_version: 1,
            catalog_version: name.into(),
            definitions: Some(vec![def.clone()]),
            digests: Some([(def.id.clone(), definition_digest(&def).unwrap())].into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let bytes = to_go_json(&bundle).unwrap();
    let digest = content_digest(&bytes);
    root.child_dir(kind, true)
        .unwrap()
        .replace(&format!("{digest}.json"), &bytes, 0o600)
        .unwrap();
    DistributionStatus {
        catalog_version: name.into(),
        digest,
        state: "accepted".into(),
        ..Default::default()
    }
}
#[test]
fn absent_status_does_not_create_root_and_corruption_is_not_absence() {
    let t = tempfile::tempdir().unwrap();
    let p = paths(&t);
    let store = DistributionStore::new(&p);
    assert_eq!(store.status().unwrap().state, "none");
    assert!(store.load_accepted().unwrap().is_none());
    assert!(!p.resource(Resource::ProviderDistributions).exists());
    let root = Dir::open_or_create_private(&p.resource(Resource::ProviderDistributions)).unwrap();
    root.replace("accepted.json", b"{", 0o600).unwrap();
    assert!(store.status().is_err());
    assert!(store.load_accepted().is_err());
}
#[test]
fn rollback_verifies_target_before_mutation_and_restore_keeps_raw_pointers() {
    let t = tempfile::tempdir().unwrap();
    let p = paths(&t);
    let root = Dir::open_or_create_private(&p.resource(Resource::ProviderDistributions)).unwrap();
    let old = seed(&root, "accepted", "old");
    let new = seed(&root, "accepted", "new");
    write_json(&root, "accepted.json", &new).unwrap();
    write_json(&root, "previous.json", &old).unwrap();
    let store = DistributionStore::new(&p);
    let original = root.read("accepted.json", usize::MAX).unwrap();
    let snapshot = store.snapshot_pointers().unwrap();
    let dir = root.child_dir("accepted", false).unwrap();
    let bundle = dir
        .read(&format!("{}.json", old.digest), usize::MAX)
        .unwrap();
    dir.replace(&format!("{}.json", old.digest), b"tampered", 0o600)
        .unwrap();
    assert!(store.rollback().is_err());
    assert_eq!(root.read("accepted.json", usize::MAX).unwrap(), original);
    assert!(root.metadata("rollback-before.json").is_err());
    dir.replace(&format!("{}.json", old.digest), &bundle, 0o600)
        .unwrap();
    assert_eq!(store.rollback().unwrap().digest, old.digest);
    assert_eq!(
        root.read("rollback-before.json", usize::MAX).unwrap(),
        original
    );
    store.restore_pointers(snapshot).unwrap();
    assert_eq!(root.read("accepted.json", usize::MAX).unwrap(), original);
    assert_eq!(
        store
            .load_accepted()
            .unwrap()
            .unwrap()
            .payload
            .catalog_version,
        "new"
    );
}
#[test]
fn downloaded_integrity_and_path_traversal_are_rejected() {
    let t = tempfile::tempdir().unwrap();
    let p = paths(&t);
    let root = Dir::open_or_create_private(&p.resource(Resource::ProviderDistributions)).unwrap();
    let status = seed(&root, "downloaded", "candidate");
    let store = DistributionStore::new(&p);
    assert_eq!(
        store
            .load_downloaded(&status.digest)
            .unwrap()
            .payload
            .catalog_version,
        "candidate"
    );
    assert!(store.load_downloaded("../accepted").is_err());
    root.child_dir("downloaded", false)
        .unwrap()
        .replace(&format!("{}.json", status.digest), b"{}", 0o600)
        .unwrap();
    assert!(store.load_downloaded(&status.digest).is_err());
}
#[test]
fn diff_preserves_override_only_change_and_three_way_conflict() {
    let a = definition("before");
    let b = definition("candidate");
    let o = definition("override");
    let diff = diff_distribution(std::slice::from_ref(&a), &[b], std::slice::from_ref(&o));
    let field = diff[0]
        .changes
        .iter()
        .find(|c| c.field == "display_name")
        .unwrap();
    assert!(field.conflict);
    assert_eq!(field.current, "before");
    assert_eq!(field.candidate, "candidate");
    assert_eq!(field.r#override, "override");
    let diff = diff_distribution(std::slice::from_ref(&a), std::slice::from_ref(&a), &[o]);
    let field = diff[0]
        .changes
        .iter()
        .find(|c| c.field == "display_name")
        .unwrap();
    assert!(!field.conflict);
    assert!(field.candidate.is_empty());
    assert_eq!(diff[0].status, "changed");
    let wire = to_go_json(field).unwrap();
    assert!(
        String::from_utf8(wire)
            .unwrap()
            .contains("\"conflict\":false")
    );
}
