use many_ai_cli::proto::time::UNIX_EPOCH;
use many_ai_cli::proto::time::{format_with_offset, parse_rfc3339};
use serde_json::Value;
use std::time::Duration;
fn stamp(seconds: i64, nanos: u32) -> many_ai_cli::proto::time::Timestamp {
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

#[test]
fn parsed_sub_tick_fraction_never_rounds_through_native_clock() {
    for value in [
        "0001-01-01T00:00:00.000000001Z",
        "1969-12-31T23:59:59.999999999Z",
        "2023-11-14T22:13:20.123456789Z",
    ] {
        assert_eq!(
            format_with_offset(parse_rfc3339(value).unwrap(), 0, true).unwrap(),
            value
        );
    }
}
