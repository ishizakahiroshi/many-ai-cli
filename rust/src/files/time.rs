//! File timestamps use the shared, Go-fixtured RFC3339 implementation. Invalid
//! filesystem timestamps are errors, never fabricated successful zero dates.
use std::time::SystemTime;
pub fn format(value: SystemTime) -> super::Result<String> {
    crate::proto::time::Timestamp::from_system_time(value)
        .and_then(crate::proto::time::format_rfc3339_nano)
        .map_err(|_| {
            super::err(
                500,
                "invalid_timestamp",
                "file timestamp cannot be represented as RFC3339",
            )
        })
}
pub fn format_utc(value: SystemTime) -> super::Result<String> {
    crate::proto::time::Timestamp::from_system_time(value)
        .and_then(|time| crate::proto::time::format_with_offset(time, 0, true))
        .map_err(|_| {
            super::err(
                500,
                "invalid_timestamp",
                "file timestamp cannot be represented as RFC3339",
            )
        })
}
pub fn parse(value: &str) -> Option<crate::proto::time::Timestamp> {
    crate::proto::time::parse_rfc3339(value).ok()
}
