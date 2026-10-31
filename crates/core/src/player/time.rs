//! Conversions between WinRT time types and Unix milliseconds.
//!
//! These are plain integer math, so they live outside the `cfg(windows)`
//! module and are tested on every platform.

use std::time::{SystemTime, UNIX_EPOCH};

/// 100 ns ticks between 1601-01-01 (WinRT / FILETIME epoch) and 1970-01-01 (Unix epoch).
pub const WINRT_TO_UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// WinRT `TimeSpan` and `DateTime` both count in 100 ns ticks.
const TICKS_PER_MS: i64 = 10_000;

/// Converts `Windows.Foundation.DateTime::UniversalTime` to Unix milliseconds.
pub fn winrt_datetime_to_unix_ms(universal_time: i64) -> i64 {
    (universal_time - WINRT_TO_UNIX_EPOCH_TICKS).div_euclid(TICKS_PER_MS)
}

/// Converts `Windows.Foundation.TimeSpan::Duration` to milliseconds.
/// Negative spans (never expected from SMTC) clamp to zero.
pub fn winrt_timespan_to_ms(duration_ticks: i64) -> u64 {
    (duration_ticks.max(0) / TICKS_PER_MS) as u64
}

/// Current wall-clock time in Unix milliseconds.
pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_maps_to_zero() {
        assert_eq!(winrt_datetime_to_unix_ms(WINRT_TO_UNIX_EPOCH_TICKS), 0);
    }

    #[test]
    fn known_date_round_trips() {
        // 2026-01-01T00:00:00Z
        let unix_ms: i64 = 1_767_225_600_000;
        let ticks = unix_ms * TICKS_PER_MS + WINRT_TO_UNIX_EPOCH_TICKS;
        assert_eq!(winrt_datetime_to_unix_ms(ticks), unix_ms);
    }

    #[test]
    fn sub_millisecond_ticks_truncate() {
        let ticks = WINRT_TO_UNIX_EPOCH_TICKS + 12_345;
        assert_eq!(winrt_datetime_to_unix_ms(ticks), 1);
    }

    #[test]
    fn timespan_converts_and_clamps() {
        assert_eq!(winrt_timespan_to_ms(2_250_000_000), 225_000); // 3:45
        assert_eq!(winrt_timespan_to_ms(-5), 0);
    }
}
