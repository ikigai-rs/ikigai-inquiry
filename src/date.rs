//! An `xsd:dateTime` as milliseconds since the epoch, with no date library: enough to tell how
//! old a tuple is from the `created` its layer stamped.

/// `text` (`2026-11-14T18:02:05.123Z`, or with an offset `+01:00`, or with no fraction) as
/// milliseconds since 1970-01-01T00:00:00Z. `None` for anything else, including a time
/// without a zone, which names no instant.
pub fn millis(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let s = text.get(from..from + len)?;
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, s) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let mut i = 19;
    let mut ms = 0i64;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        let mut end = start;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
        if end == start {
            return None;
        }
        let frac = &text[start..end.min(start + 3)];
        ms = format!("{frac:0<3}").parse().ok()?;
        i = end;
    }
    let offset_min = match &text[i..] {
        "Z" => 0,
        zone if zone.len() == 6 && (zone.starts_with('+') || zone.starts_with('-')) => {
            if zone.as_bytes()[3] != b':' {
                return None;
            }
            let (oh, om) = (num(i + 1, 2)?, num(i + 4, 2)?);
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            sign * (oh * 60 + om)
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    let total = ((days * 24 + h) * 60 + mi - offset_min) * 60 + s;
    u64::try_from(total * 1000 + ms).ok()
}

/// Howard Hinnant's `days_from_civil`: a proleptic Gregorian date to days since 1970-01-01.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_a_layer_stamps() {
        assert_eq!(millis("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(millis("2026-10-03T04:00:00.123Z"), Some(1_791_000_000_123));
        assert_eq!(millis("2024-02-29T00:00:00Z"), Some(1_709_164_800_000));
        assert_eq!(millis("2025-12-31T23:59:59.999Z"), Some(1_767_225_599_999));
        assert_eq!(millis("2024-02-29T01:00:00+01:00"), Some(1_709_164_800_000));
        assert_eq!(millis("2024-02-29T00:00:00.5Z"), Some(1_709_164_800_500));
    }

    #[test]
    fn refuses_what_names_no_instant() {
        for bad in [
            "",
            "2024-02-29",
            "2024-02-29T00:00:00",
            "2024-13-01T00:00:00Z",
            "2024-02-29T00:00:00.Z",
            "2024-02-29T00:00:00+0100",
            "x024-02-29T00:00:00Z",
        ] {
            assert_eq!(millis(bad), None, "{bad}");
        }
    }
}
