//! Civil dates and RFC 3339 times from the kernel clock's milliseconds, in UTC.

use ikigai_core::Time;

const DAY_MS: u64 = 86_400_000;

/// The days since 1970-01-01 as a civil date (Howard Hinnant's `civil_from_days`).
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

/// The UTC date of `time`, `YYYY-MM-DD`.
pub(crate) fn date(time: Time) -> String {
    let (y, m, d) = civil(time.as_millis() / DAY_MS);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `time` as an `xsd:dateTime` in UTC, to the millisecond.
pub(crate) fn date_time(time: Time) -> String {
    let millis = time.as_millis();
    let of_day = millis % DAY_MS;
    let (h, min, s, ms) = (
        of_day / 3_600_000,
        of_day / 60_000 % 60,
        of_day / 1000 % 60,
        of_day % 1000,
    );
    format!("{}T{h:02}:{min:02}:{s:02}.{ms:03}Z", date(time))
}

/// Whether `text` is a date this module counts under: `YYYY-MM-DD`, a real day.
pub(crate) fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u64> {
        text.get(r)
            .filter(|t| t.bytes().all(|c| c.is_ascii_digit()))
            .and_then(|t| t.parse().ok())
    };
    let (Some(y), Some(m), Some(d)) = (digits(0..4), digits(5..7), digits(8..10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    y >= 1970 && (1..=days_in).contains(&d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_times_are_utc() {
        // 2026-11-14T18:23:56.789Z
        let t = Time::from_millis(1_794_680_636_789);
        assert_eq!(date(t), "2026-11-14");
        assert_eq!(date_time(t), "2026-11-14T18:23:56.789Z");
        assert_eq!(date(Time::from_millis(0)), "1970-01-01");
        assert_eq!(date(Time::from_millis(951_782_400_000)), "2000-02-29");
    }

    #[test]
    fn only_real_days_are_dates() {
        assert!(is_date("2026-11-14"));
        assert!(is_date("2024-02-29"));
        assert!(!is_date("2026-02-29"));
        assert!(!is_date("2026-13-01"));
        assert!(!is_date("2026-1-14"));
        assert!(!is_date("2026-11-14x"));
        assert!(!is_date("+026-11-14"));
    }
}
