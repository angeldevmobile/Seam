//! Written here rather than delegated to a date crate because the two rules
//! that matter are policy, not parsing: a `Date` is never widened into an
//! instant, and a `DateTime` without an offset is an error rather than a guess
//! about local time. Typed conversion happens in the bindings.

use crate::error::Code;

/// Strict `YYYY-MM-DD`, including month lengths and leap years.
pub fn validate_date(s: &str) -> Result<(), Code> {
    parse_date(s).map(|_| ()).ok_or(Code::InvalidDate)
}

/// RFC 3339 with a mandatory offset. Returns [`Code::MissingTimezone`] when the
/// value is well-formed but unzoned, since that is the common mistake.
pub fn validate_datetime(s: &str) -> Result<(), Code> {
    let bytes = s.as_bytes();

    let date_part = s.get(..10).ok_or(Code::InvalidDateTime)?;
    parse_date(date_part).ok_or(Code::InvalidDateTime)?;
    match bytes.get(10) {
        Some(b'T' | b't') => {}
        _ => return Err(Code::InvalidDateTime),
    }

    let time_part = s.get(11..19).ok_or(Code::InvalidDateTime)?;
    let tb = time_part.as_bytes();
    if tb.get(2) != Some(&b':') || tb.get(5) != Some(&b':') {
        return Err(Code::InvalidDateTime);
    }
    let hour = two_digits(tb, 0).ok_or(Code::InvalidDateTime)?;
    let minute = two_digits(tb, 3).ok_or(Code::InvalidDateTime)?;
    let second = two_digits(tb, 6).ok_or(Code::InvalidDateTime)?;
    // 60 is a leap second, which RFC 3339 allows.
    if hour > 23 || minute > 59 || second > 60 {
        return Err(Code::InvalidDateTime);
    }

    let mut rest = s.get(19..).ok_or(Code::InvalidDateTime)?;
    if let Some(after_dot) = rest.strip_prefix('.') {
        let digits = after_dot.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return Err(Code::InvalidDateTime);
        }
        rest = after_dot.get(digits..).ok_or(Code::InvalidDateTime)?;
    }

    if rest.is_empty() {
        return Err(Code::MissingTimezone);
    }
    if matches!(rest, "Z" | "z") {
        return Ok(());
    }

    let ob = rest.as_bytes();
    if ob.len() != 6 || !matches!(ob.first(), Some(b'+' | b'-')) || ob.get(3) != Some(&b':') {
        return Err(Code::InvalidDateTime);
    }
    let off_h = two_digits(ob, 1).ok_or(Code::InvalidDateTime)?;
    let off_m = two_digits(ob, 4).ok_or(Code::InvalidDateTime)?;
    if off_h > 23 || off_m > 59 {
        return Err(Code::InvalidDateTime);
    }

    Ok(())
}

fn parse_date(s: &str) -> Option<(u32, u32, u32)> {
    let b = s.as_bytes();
    if b.len() != 10 || b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }

    let year = four_digits(b, 0)?;
    let month = two_digits(b, 5)?;
    let day = two_digits(b, 8)?;

    if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some((year, month, day))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn two_digits(b: &[u8], at: usize) -> Option<u32> {
    let d0 = digit(*b.get(at)?)?;
    let d1 = digit(*b.get(at + 1)?)?;
    Some(d0 * 10 + d1)
}

fn four_digits(b: &[u8], at: usize) -> Option<u32> {
    let hi = two_digits(b, at)?;
    let lo = two_digits(b, at + 2)?;
    Some(hi * 100 + lo)
}

fn digit(c: u8) -> Option<u32> {
    c.is_ascii_digit().then(|| u32::from(c - b'0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_dates_pass() {
        assert!(validate_date("2026-08-29").is_ok());
        assert!(validate_date("2000-02-29").is_ok());
        assert!(validate_date("2024-02-29").is_ok());
    }

    #[test]
    fn dates_that_do_not_exist_fail() {
        assert_eq!(validate_date("2023-02-29"), Err(Code::InvalidDate));
        assert_eq!(validate_date("1900-02-29"), Err(Code::InvalidDate));
        assert_eq!(validate_date("2026-13-01"), Err(Code::InvalidDate));
        assert_eq!(validate_date("2026-04-31"), Err(Code::InvalidDate));
        assert_eq!(validate_date("2026-00-10"), Err(Code::InvalidDate));
    }

    #[test]
    fn lenient_date_spellings_are_rejected() {
        assert_eq!(validate_date("2026-8-29"), Err(Code::InvalidDate));
        assert_eq!(validate_date("2026/08/29"), Err(Code::InvalidDate));
        assert_eq!(
            validate_date("2026-08-29T00:00:00Z"),
            Err(Code::InvalidDate)
        );
        assert_eq!(validate_date(""), Err(Code::InvalidDate));
    }

    #[test]
    fn datetimes_with_an_offset_pass() {
        assert!(validate_datetime("2026-08-29T14:30:00Z").is_ok());
        assert!(validate_datetime("2026-08-29T14:30:00+02:00").is_ok());
        assert!(validate_datetime("2026-08-29T14:30:00-05:00").is_ok());
        assert!(validate_datetime("2026-08-29T14:30:00.123456Z").is_ok());
    }

    #[test]
    fn a_naive_datetime_is_an_error_not_an_assumption() {
        assert_eq!(
            validate_datetime("2026-08-29T14:30:00"),
            Err(Code::MissingTimezone)
        );
        assert_eq!(
            validate_datetime("2026-08-29T14:30:00.500"),
            Err(Code::MissingTimezone)
        );
    }

    #[test]
    fn malformed_datetimes_fail_as_malformed() {
        assert_eq!(validate_datetime("2026-08-29"), Err(Code::InvalidDateTime));
        assert_eq!(
            validate_datetime("2026-08-29 14:30:00Z"),
            Err(Code::InvalidDateTime)
        );
        assert_eq!(
            validate_datetime("2026-08-29T25:00:00Z"),
            Err(Code::InvalidDateTime)
        );
        assert_eq!(
            validate_datetime("2026-08-29T14:30:00+2:00"),
            Err(Code::InvalidDateTime)
        );
    }
}
