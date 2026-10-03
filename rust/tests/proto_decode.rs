use many_ai_cli::proto::{self, Message};
use serde_json::Value;
#[test]
fn source_go_json_decode_cases() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/foundation/proto-decode.json")).unwrap();
    assert_eq!(cases.len(), 19);
    for (i, case) in cases.iter().enumerate() {
        let decoded = proto::decode_wire::<Message>(case["input"].as_str().unwrap().as_bytes());
        if case["error"] == true {
            assert!(decoded.is_err(), "case {i} should reject");
        } else {
            let got =
                serde_json::to_value(decoded.unwrap_or_else(|e| panic!("case {i}: {e}"))).unwrap();
            assert_eq!(got, case["value"], "case {i}");
        }
    }
}
