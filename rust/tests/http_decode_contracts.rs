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
#[derive(Deserialize, Serialize)]
#[serde(default)]
struct Body {
    mtime: String,
    count: i64,
    nested: Option<Nested>,
    names: Option<Vec<String>>,
}
impl Default for Body {
    fn default() -> Self {
        Self {
            mtime: "0001-01-01T00:00:00Z".into(),
            count: 0,
            nested: None,
            names: None,
        }
    }
}
impl GoWire for Body {
    const GO_TYPE: &'static str = "SyntheticBody";
    const SCHEMAS: &'static [Schema] = &[
        Schema {
            name: "SyntheticBody",
            fields: &[
                Field {
                    name: "mtime",
                    kind: "time.Time",
                },
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
    assert_eq!(cases.len(), 25);
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

#[test]
fn raw_message_map_preserves_values_and_go_key_replacement() {
    use many_ai_cli::proto::wire::decode_go_json_object;
    let bytes = b"{\"future\":1e999,\"same\":1,\"same\": { \"x\":1,\"x\":2 }, \"\\ud800\":\"\\ud800\",\"bad\":\"\xff\"}";
    let map = decode_go_json_object(bytes).unwrap().unwrap();
    assert_eq!(map["future"], b"1e999");
    assert_eq!(map["same"], b"{ \"x\":1,\"x\":2 }");
    assert_eq!(map["\u{fffd}"], b"\"\\ud800\"");
    assert_eq!(map["bad"], b"\"\xff\"");
    assert!(decode_go_json_object(b"null").unwrap().is_none());
    for invalid in [b"[]".as_slice(), b"{} {}", b"{\"x\":01}", b"{\"x\":[1,]}"] {
        assert!(decode_go_json_object(invalid).is_err());
    }
}

#[test]
fn ordered_raw_members_preserve_folded_last_and_null_presence() {
    use many_ai_cli::proto::wire::{decode_go_json_members, last_go_raw_field};
    let members =
        decode_go_json_members(br#"{"content":1e999,"CONTENT":null,"content":{"x":1,"x":2}}"#)
            .unwrap()
            .unwrap();
    assert_eq!(members.len(), 3);
    assert_eq!(
        last_go_raw_field(&members, "content").unwrap(),
        br#"{"x":1,"x":2}"#
    );
    assert_eq!(
        last_go_raw_field(&members[..2], "content").unwrap(),
        b"null"
    );
    assert!(last_go_raw_field(&members, "absent").is_none());
}

#[test]
fn raw_array_and_primitive_leaves_keep_go_boundaries() {
    use many_ai_cli::proto::{decode_wire, wire::decode_go_json_array};
    let raw = b"[1e999, {\"x\":1,\"x\":2}, null, \"\\ud800\", \"\xff\"]";
    let values = decode_go_json_array(raw).unwrap().unwrap();
    assert_eq!(values.len(), 5);
    assert_eq!(values[0], b"1e999");
    assert_eq!(values[1], br#"{"x":1,"x":2}"#);
    assert_eq!(values[2], b"null");
    assert_eq!(values[4], b"\"\xff\"");
    assert_eq!(decode_wire::<String>(&values[3]).unwrap(), "\u{fffd}");
    assert_eq!(decode_wire::<String>(&values[4]).unwrap(), "\u{fffd}");
    assert_eq!(decode_wire::<String>(b"null").unwrap(), "");
    assert_eq!(decode_wire::<i64>(b"-0").unwrap(), 0);
    assert!(decode_wire::<i64>(b"1.0").is_err());
    assert!(decode_go_json_array(b"null").unwrap().is_none());
    assert!(decode_go_json_array(b"[1,]").is_err());
}

#[test]
fn generic_http_first_value_keeps_go_map_float_and_unicode_semantics() {
    use many_ai_cli::proto::decode_http_value;
    let value = decode_http_value(
        b"{\"A\":1,\"a\":2,\"A\":3,\"unicode\":\"\\ud800\",\"nil\":null} ignored",
    )
    .unwrap();
    assert_eq!(value["A"].as_f64(), Some(3.0));
    assert_eq!(value["a"].as_f64(), Some(2.0));
    assert!(value["A"].is_f64());
    assert_eq!(value["unicode"], "\u{fffd}");
    assert!(value["nil"].is_null());
    assert!(decode_http_value(br#"{"number":1e1000,"number":1}"#).is_err());
    assert!(decode_http_value(b"[1,]").is_err());
}
