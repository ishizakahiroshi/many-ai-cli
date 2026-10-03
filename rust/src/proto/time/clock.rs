//! Platform-independent logical wall clock. Native clocks enter explicitly;
//! parsing and arithmetic never round through Windows FILETIME ticks.
use super::TimestampError;
use std::{
    fmt, ops,
    time::{Duration, SystemTime},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    seconds: i64,
    nanos: u32,
}
pub const UNIX_EPOCH: Timestamp = Timestamp::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EarlierTimestamp(Duration);
impl EarlierTimestamp {
    pub fn duration(self) -> Duration {
        self.0
    }
}
impl fmt::Display for EarlierTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the supplied timestamp is later than this timestamp")
    }
}
impl std::error::Error for EarlierTimestamp {}

impl Timestamp {
    pub const UNIX_EPOCH: Self = Self {
        seconds: 0,
        nanos: 0,
    };
    pub const fn from_unix(seconds: i64, nanos: u32) -> Result<Self, TimestampError> {
        if nanos >= 1_000_000_000 {
            Err(TimestampError)
        } else {
            Ok(Self { seconds, nanos })
        }
    }
    pub const fn unix_seconds(self) -> i64 {
        self.seconds
    }
    pub const fn subsec_nanos(self) -> u32 {
        self.nanos
    }
    pub const fn unix_nanos(self) -> i128 {
        self.seconds as i128 * 1_000_000_000 + self.nanos as i128
    }
    fn from_total(total: i128) -> Option<Self> {
        Some(Self {
            seconds: i64::try_from(total.div_euclid(1_000_000_000)).ok()?,
            nanos: total.rem_euclid(1_000_000_000) as u32,
        })
    }
    /// Preserve the precision actually reported by the OS; no extra precision
    /// is invented. Native i64-second/FILETIME clocks fit this representation.
    pub fn from_system_time(value: SystemTime) -> Result<Self, TimestampError> {
        let total = match value.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.as_nanos() as i128,
            Err(e) => -(e.duration().as_nanos() as i128),
        };
        Self::from_total(total).ok_or(TimestampError)
    }
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
            .expect("native wall clock exceeds signed 64-bit Unix seconds")
    }
    /// Only OS-facing callers need this boundary. Refuse lossy sub-tick values
    /// instead of silently truncating their Go-compatible logical timestamp.
    pub fn to_system_time_exact(self) -> Result<SystemTime, TimestampError> {
        let total = self.unix_nanos();
        let duration = magnitude(total);
        let native = if total < 0 {
            SystemTime::UNIX_EPOCH.checked_sub(duration)
        } else {
            SystemTime::UNIX_EPOCH.checked_add(duration)
        }
        .ok_or(TimestampError)?;
        if Self::from_system_time(native)? != self {
            return Err(TimestampError);
        }
        Ok(native)
    }
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        Self::from_total(self.unix_nanos().checked_add(duration.as_nanos() as i128)?)
    }
    pub fn checked_sub(self, duration: Duration) -> Option<Self> {
        Self::from_total(self.unix_nanos().checked_sub(duration.as_nanos() as i128)?)
    }
    pub fn duration_since(self, earlier: Self) -> Result<Duration, EarlierTimestamp> {
        let delta = self.unix_nanos() - earlier.unix_nanos();
        if delta < 0 {
            Err(EarlierTimestamp(magnitude(delta)))
        } else {
            Ok(magnitude(delta))
        }
    }
}
fn magnitude(value: i128) -> Duration {
    let magnitude = value.unsigned_abs();
    // Two normalized i64-second instants differ by at most u64::MAX seconds
    // and 999,999,999 nanoseconds. Native conversions have a smaller range.
    Duration::new(
        (magnitude / 1_000_000_000) as u64,
        (magnitude % 1_000_000_000) as u32,
    )
}
impl ops::Add<Duration> for Timestamp {
    type Output = Self;
    fn add(self, rhs: Duration) -> Self {
        self.checked_add(rhs).expect("timestamp addition overflow")
    }
}
impl ops::Sub<Duration> for Timestamp {
    type Output = Self;
    fn sub(self, rhs: Duration) -> Self {
        self.checked_sub(rhs)
            .expect("timestamp subtraction overflow")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_negative_fraction_and_exact_arithmetic() {
        let before = UNIX_EPOCH - Duration::from_nanos(1);
        assert_eq!(
            (before.unix_seconds(), before.subsec_nanos()),
            (-1, 999_999_999)
        );
        assert_eq!(before + Duration::from_nanos(1), UNIX_EPOCH);
        assert_eq!(
            before.duration_since(UNIX_EPOCH).unwrap_err().duration(),
            Duration::from_nanos(1)
        );
        assert_eq!(
            UNIX_EPOCH.duration_since(before).unwrap(),
            Duration::from_nanos(1)
        );
        assert!(Timestamp::from_unix(0, 1_000_000_000).is_err());
    }
    #[test]
    fn representational_bounds_are_checked_without_rounding() {
        let min = Timestamp::from_unix(i64::MIN, 0).unwrap();
        let max = Timestamp::from_unix(i64::MAX, 999_999_999).unwrap();
        assert!(min.checked_sub(Duration::from_nanos(1)).is_none());
        assert!(max.checked_add(Duration::from_nanos(1)).is_none());
        assert_eq!(
            max.duration_since(min).unwrap(),
            Duration::new(u64::MAX, 999_999_999)
        );
    }
    #[test]
    fn native_boundary_round_trips_or_explicitly_refuses_precision_loss() {
        for nanos in [0, 100, 123_456_700, 123_456_789] {
            let at = Timestamp::from_unix(1_700_000_000, nanos).unwrap();
            match at.to_system_time_exact() {
                Ok(native) => assert_eq!(Timestamp::from_system_time(native).unwrap(), at),
                Err(_) => {
                    #[cfg(windows)]
                    assert_ne!(nanos % 100, 0);
                    #[cfg(not(windows))]
                    panic!("native clock unexpectedly lost fixture precision");
                }
            }
        }
        #[cfg(windows)]
        assert!(
            (UNIX_EPOCH + Duration::from_nanos(1))
                .to_system_time_exact()
                .is_err()
        );
        let native = SystemTime::now();
        assert_eq!(
            Timestamp::from_system_time(native)
                .unwrap()
                .to_system_time_exact()
                .unwrap(),
            native
        );
    }
}
