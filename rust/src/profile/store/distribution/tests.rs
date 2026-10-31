use super::*;
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
