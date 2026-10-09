#[test]
fn pure_observations_match_independent_pinned_go_source_corpus() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/session-observations/cases.json"
    ))
    .unwrap();
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/session-observations/go-observations.json"
    ))
    .unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 41);
    for (case, expected) in cases
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle.as_array().unwrap())
    {
        assert_eq!(case["name"], expected["name"]);
        let lines = case["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let actual = super::workflow::parse(&lines)
            .map(|p| serde_json::to_value(p).unwrap())
            .unwrap_or(serde_json::Value::Null);
        assert_eq!(actual, expected["workflow"], "workflow {}", case["name"]);
        let actual = super::cross_message::detect(&lines)
            .map(|c| serde_json::json!({"Sender":c.sender,"Line":c.line,"Signature":c.signature}))
            .unwrap_or(serde_json::Value::Null);
        assert_eq!(actual, expected["cross"], "cross message {}", case["name"]);
        let mut parser = super::journal::Parser::default();
        for byte in case["journal"].as_str().unwrap_or("").bytes() {
            parser.feed(byte)
        }
        let actual = parser
            .event()
            .map(|(kind, agent)| serde_json::json!({"type":kind,"agentId":agent}))
            .unwrap_or(serde_json::Value::Null);
        assert_eq!(actual, expected["journal"], "journal {}", case["name"]);
    }
}
