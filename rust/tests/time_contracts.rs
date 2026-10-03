use many_ai_cli::proto::time::{format_with_offset, parse_rfc3339};
use serde_json::Value;
use std::time::{Duration, UNIX_EPOCH};
fn stamp(seconds: i64, nanos: u32) -> std::time::SystemTime {
    if seconds < 0 {
        UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs())
            + Duration::from_nanos(u64::from(nanos))
    } else {
        UNIX_EPOCH + Duration::new(seconds as u64, nanos)
    }
}
#[test]
fn go_timestamp_formatting_precision_and_parse_edges() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/foundation/time/oracle.json")).unwrap();
    let formats = corpus["formats"].as_array().unwrap();
    assert_eq!(formats.len(), 100);
    for row in formats {
        let at = stamp(
            row["seconds"].as_i64().unwrap(),
            row["nanos"].as_u64().unwrap() as u32,
        );
        let offset = row["offset"].as_i64().unwrap() as i32;
        assert_eq!(
            format_with_offset(at, offset, false).unwrap(),
            row["seconds_text"]
        );
        assert_eq!(
            format_with_offset(at, offset, true).unwrap(),
            row["nano_text"]
        );
    }
    let parses = corpus["parses"].as_array().unwrap();
    assert_eq!(parses.len(), 18);
    for row in parses {
        let result = parse_rfc3339(row["input"].as_str().unwrap());
        if row["error"] == true {
            assert!(result.is_err(), "{}", row["input"]);
        } else {
            assert_eq!(
                result.unwrap(),
                stamp(
                    row["seconds"].as_i64().unwrap(),
                    row["nanos"].as_u64().unwrap() as u32
                ),
                "{}",
                row["input"]
            );
        }
    }
}
