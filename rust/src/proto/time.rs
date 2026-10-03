//! Go-compatible timestamp helpers. Storage/approval use local RFC3339 seconds;
//! JSON time values use RFC3339Nano. Callers must select precision explicitly.
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, Offset, Timelike, Utc};
use std::{fmt, sync::OnceLock};
mod clock;
pub use clock::{EarlierTimestamp, Timestamp, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimestampError;
impl fmt::Display for TimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("timestamp is invalid or outside the supported range")
    }
}
impl std::error::Error for TimestampError {}
pub fn utc(time: Timestamp) -> Result<DateTime<Utc>, TimestampError> {
    DateTime::from_timestamp(time.unix_seconds(), time.subsec_nanos()).ok_or(TimestampError)
}
/// Civil-time scheduling boundary; never converts through OS clock precision.
pub fn from_utc(time: DateTime<Utc>) -> Result<Timestamp, TimestampError> {
    Timestamp::from_unix(time.timestamp(), time.timestamp_subsec_nanos())
}
/// Format with the runtime's local timezone, like time.Now().Format(RFC3339).
pub fn format_rfc3339(time: Timestamp) -> Result<String, TimestampError> {
    let offset = utc(time)?
        .with_timezone(&Local)
        .offset()
        .fix()
        .local_minus_utc();
    format_with_offset(time, offset, false)
}
pub fn format_rfc3339_nano(time: Timestamp) -> Result<String, TimestampError> {
    let offset = utc(time)?
        .with_timezone(&Local)
        .offset()
        .fix()
        .local_minus_utc();
    format_with_offset(time, offset, true)
}
/// Deterministic explicit-offset formatter for provider/source timestamps and tests.
/// Zone seconds are truncated to minutes in the suffix, as in Go's RFC3339 layout.
pub fn format_with_offset(
    time: Timestamp,
    offset_seconds: i32,
    nano: bool,
) -> Result<String, TimestampError> {
    let offset = FixedOffset::east_opt(offset_seconds).ok_or(TimestampError)?;
    let date = utc(time)?.with_timezone(&offset);
    if !(0..=9999).contains(&date.year()) {
        return Err(TimestampError);
    }
    let mut out = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        date.year(),
        date.month(),
        date.day(),
        date.hour(),
        date.minute(),
        date.second()
    );
    if nano && date.nanosecond() != 0 {
        out.push('.');
        out.push_str(format!("{:09}", date.nanosecond()).trim_end_matches('0'));
    }
    if offset_seconds == 0 {
        out.push('Z');
    } else {
        let minutes = offset_seconds.unsigned_abs() / 60;
        out.push(if offset_seconds / 60 < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}:{:02}", minutes / 60, minutes % 60));
    }
    Ok(out)
}
/// Match time.Parse(RFC3339), including comma fractions, one-digit hours, and
/// its tolerated 24-hour/60-minute numeric offsets. Leap seconds are rejected.
pub fn parse_rfc3339(value: &str) -> Result<Timestamp, TimestampError> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re=RE.get_or_init(||regex::Regex::new(r"\A([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{1,2}):([0-9]{2}):([0-9]{2})(?:[.,]([0-9]+))?(Z|[+-][0-9]{2}:[0-9]{2})\z").expect("constant timestamp regex"));
    let c = re.captures(value).ok_or(TimestampError)?;
    let n = |i: usize| c[i].parse::<u32>().map_err(|_| TimestampError);
    let mut nanos = 0u32;
    if let Some(fraction) = c.get(7) {
        let s = fraction.as_str();
        for i in 0..9 {
            nanos = nanos * 10 + u32::from(s.as_bytes().get(i).copied().unwrap_or(b'0') - b'0');
        }
    }
    if n(6)? > 59 {
        return Err(TimestampError);
    }
    let date = NaiveDate::from_ymd_opt(n(1)? as i32, n(2)?, n(3)?)
        .and_then(|d| d.and_hms_nano_opt(n(4).ok()?, n(5).ok()?, n(6).ok()?, nanos))
        .ok_or(TimestampError)?;
    let zone = &c[8];
    let offset = if zone == "Z" {
        0
    } else {
        let h = zone[1..3].parse::<i64>().map_err(|_| TimestampError)?;
        let m = zone[4..6].parse::<i64>().map_err(|_| TimestampError)?;
        if h > 24 || m > 60 {
            return Err(TimestampError);
        }
        (h * 3600 + m * 60) * if zone.starts_with('-') { -1 } else { 1 }
    };
    Timestamp::from_unix(date.and_utc().timestamp() - offset, nanos)
}
