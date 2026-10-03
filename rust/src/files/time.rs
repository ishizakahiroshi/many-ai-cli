//! File timestamps use the shared, Go-fixtured RFC3339 implementation. Invalid
//! filesystem timestamps are errors, never fabricated successful zero dates.
use std::time::SystemTime;
pub fn format(value: SystemTime) -> super::Result<String> {
    crate::proto::time::format_rfc3339_nano(value).map_err(|_| {
        super::err(
            500,
            "invalid_timestamp",
            "file timestamp cannot be represented as RFC3339",
        )
    })
}
pub fn format_utc(value: SystemTime) -> super::Result<String> {
    crate::proto::time::format_with_offset(value, 0, true).map_err(|_| {
        super::err(
            500,
            "invalid_timestamp",
            "file timestamp cannot be represented as RFC3339",
        )
    })
}
pub fn parse(value: &str) -> Option<SystemTime> {
    crate::proto::time::parse_rfc3339(value).ok()
}
