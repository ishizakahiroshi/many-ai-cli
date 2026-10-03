//! Civil-time routine scheduling, including the Go time.Date ambiguity choice.
use chrono::{DateTime, Datelike, Days, LocalResult, NaiveTime, Offset, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub time: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub timezone: String,
}
pub fn next(schedule: &Schedule, after: SystemTime) -> Result<Option<SystemTime>, String> {
    if schedule.kind == "manual" {
        return Ok(None);
    }
    if !matches!(schedule.kind.as_str(), "daily" | "weekdays") {
        return Err("schedule kind must be manual, daily, or weekdays".into());
    }
    let parts: Vec<_> = schedule.time.split(':').collect();
    let parsed = if parts.len() == 2
        && (1..=2).contains(&parts[0].len())
        && parts[1].len() == 2
        && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
    {
        parts[0]
            .parse::<u32>()
            .ok()
            .zip(parts[1].parse::<u32>().ok())
            .and_then(|(h, m)| NaiveTime::from_hms_opt(h, m, 0))
    } else {
        None
    };
    let clock = parsed.ok_or("schedule time must be HH:MM")?;
    if schedule.timezone.is_empty() || schedule.timezone == "Local" {
        return Err("schedule timezone is required".into());
    }
    let zone: chrono_tz::Tz = schedule
        .timezone
        .parse()
        .map_err(|_| "unknown schedule timezone")?;
    let after: DateTime<Utc> = after.into();
    let first = after.with_timezone(&zone).date_naive();
    for offset in 0..9 {
        let date = first
            .checked_add_days(Days::new(offset))
            .ok_or("no next schedule time")?;
        if schedule.kind == "weekdays" && matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            continue;
        }
        let wall = date.and_time(clock);
        let candidate = match zone.from_local_datetime(&wall) {
            LocalResult::Single(v) => v,
            LocalResult::None => continue,
            LocalResult::Ambiguous(a, b) => {
                // Go first looks up the wall-clock seconds as though they were UTC.
                // This selects early US and late European fall-back occurrences.
                let offset = zone.offset_from_utc_datetime(&wall).fix().local_minus_utc();
                if a.offset().fix().local_minus_utc() == offset {
                    a
                } else {
                    b
                }
            }
        };
        if candidate.with_timezone(&Utc) > after {
            return Ok(Some(candidate.with_timezone(&Utc).into()));
        }
    }
    Err("no next schedule time".into())
}
