use base64::{Engine, engine::general_purpose::STANDARD};
use many_ai_cli::proto::{self, Message};
use serde_json::Value;
#[test]
fn source_go_json_decode_cases() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/foundation/proto-decode.json")).unwrap();
    assert_eq!(cases.len(), 63);
    for (i, case) in cases.iter().enumerate() {
        let input = STANDARD
            .decode(case["input_base64"].as_str().unwrap())
            .unwrap();
        let decoded = proto::decode_wire::<Message>(&input);
        if case["error"] == true {
            assert!(decoded.is_err(), "case {i} should reject");
        } else {
            let got =
                serde_json::to_value(decoded.unwrap_or_else(|e| panic!("case {i}: {e}"))).unwrap();
            assert_eq!(got, case["value"], "case {i}");
        }
    }
}
