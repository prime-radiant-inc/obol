//! Validation and ordering for the caller-supplied `as_of` pricing stamp.
//!
//! `as_of` is load-bearing: snapshot precedence (bundled floor vs on-disk
//! refresh) is decided by comparing stamps. Only two forms are accepted —
//! `YYYY-MM-DD` and `YYYY-MM-DDTHH:MM:SSZ` (UTC) — so ordering is total and
//! a stamp can never smuggle path separators into the archive filename.

use crate::error::ObolError;

/// Numeric sort key (`yyyymmddhhmmss`) for a stamp. A bare date counts as
/// midnight UTC. Errors on anything but the two canonical forms.
pub fn sort_key(s: &str) -> Result<u64, ObolError> {
    let err = || ObolError::InvalidAsOf(s.to_string());
    let b = s.as_bytes();
    if b.len() != 10 && b.len() != 20 {
        return Err(err());
    }
    let num = |range: std::ops::Range<usize>| -> Result<u64, ObolError> {
        let part = &b[range];
        if !part.iter().all(u8::is_ascii_digit) {
            return Err(err());
        }
        Ok(std::str::from_utf8(part).unwrap().parse().unwrap())
    };
    if b[4] != b'-' || b[7] != b'-' {
        return Err(err());
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return Err(err());
    }
    let (hour, minute, second) = if b.len() == 20 {
        if b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
            return Err(err());
        }
        (num(11..13)?, num(14..16)?, num(17..19)?)
    } else {
        (0, 0, 0)
    };
    if hour > 23 || minute > 59 || second > 59 {
        return Err(err());
    }
    Ok(((year * 100 + month) * 100 + day) * 1_000_000 + (hour * 100 + minute) * 100 + second)
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

/// Validate a stamp without caring about its ordering.
pub fn validate(s: &str) -> Result<(), ObolError> {
    sort_key(s).map(|_| ())
}

/// Filesystem-safe archive filename for a validated stamp
/// (`:` is not portable in filenames).
pub fn archive_file_name(s: &str) -> String {
    format!("prices-{}.json", s.replace(':', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_date() {
        assert!(sort_key("2026-06-09").is_ok());
    }

    #[test]
    fn accepts_utc_datetime() {
        assert!(sort_key("2026-06-09T18:26:46Z").is_ok());
    }

    #[test]
    fn accepts_leap_day_in_leap_year() {
        assert!(sort_key("2028-02-29").is_ok());
    }

    #[test]
    fn rejects_garbage_formats() {
        for bad in [
            "",
            "6/9/2026",
            "Apr-2027",
            "20260609",
            "2026-6-9",
            "2026-06-09T18:26:46",  // missing Z
            "2026-06-09 18:26:46Z", // space, not T
            "2026-06-09t18:26:46Z", // lowercase t
            "2026-06-09T18:26Z",    // no seconds
            "../../escape",
            "2026-06-09/evil",
            "2026-06-09\n",
        ] {
            assert!(
                matches!(sort_key(bad), Err(ObolError::InvalidAsOf(_))),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn rejects_invalid_calendar_and_clock_values() {
        for bad in [
            "2026-02-29", // 2026 is not a leap year
            "2100-02-29", // century non-leap
            "2026-13-01",
            "2026-00-10",
            "2026-06-00",
            "2026-06-31",
            "2026-06-09T24:00:00Z",
            "2026-06-09T18:60:46Z",
            "2026-06-09T18:26:60Z",
        ] {
            assert!(
                matches!(sort_key(bad), Err(ObolError::InvalidAsOf(_))),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn orders_dates_and_datetimes_together() {
        let k = |s: &str| sort_key(s).unwrap();
        assert!(k("2026-06-04") < k("2026-06-09"));
        assert_eq!(k("2026-06-09"), k("2026-06-09T00:00:00Z"));
        assert!(k("2026-06-09T08:00:00Z") < k("2026-06-09T09:30:00Z"));
        assert!(k("2026-06-09T23:59:59Z") < k("2026-06-10"));
        // The bug that motivated all this: junk no longer "wins" — it errors.
        assert!(sort_key("6/9/2026").is_err());
    }

    #[test]
    fn archive_file_name_is_filesystem_safe() {
        assert_eq!(archive_file_name("2026-06-09"), "prices-2026-06-09.json");
        assert_eq!(
            archive_file_name("2026-06-09T18:26:46Z"),
            "prices-2026-06-09T18-26-46Z.json"
        );
    }
}
