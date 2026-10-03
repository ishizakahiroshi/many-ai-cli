use many_ai_cli::proto::{
    decode_http_json,
    wire::{Field, GoWire, Schema},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Nested {
    enabled: bool,
    text: String,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Body {
    count: i64,
    nested: Option<Nested>,
    names: Option<Vec<String>>,
}
impl GoWire for Body {
    const GO_TYPE: &'static str = "SyntheticBody";
    const SCHEMAS: &'static [Schema] = &[
        Schema {
            name: "SyntheticBody",
            fields: &[
                Field {
                    name: "count",
                    kind: "int",
                },
                Field {
                    name: "nested",
                    kind: "*SyntheticNested",
                },
                Field {
                    name: "names",
                    kind: "[]string",
                },
            ],
        },
        Schema {
            name: "SyntheticNested",
            fields: &[
                Field {
                    name: "enabled",
                    kind: "bool",
                },
                Field {
                    name: "text",
                    kind: "string",
                },
            ],
        },
    ];
}
#[test]
fn first_value_and_explicit_service_schemas_match_go_decoder() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/foundation/http-decode.json")).unwrap();
    assert_eq!(cases.len(), 20);
    for (i, c) in cases.iter().enumerate() {
        let result = decode_http_json::<Body>(c["input"].as_str().unwrap().as_bytes());
        if c["error"] == true {
            assert!(result.is_err(), "case {i}");
        } else {
            assert_eq!(
                serde_json::to_value(result.unwrap_or_else(|e| panic!("case {i}: {e}"))).unwrap(),
                c["value"],
                "case {i}"
            );
        }
    }
}
