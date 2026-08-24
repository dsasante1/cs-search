//! Date specifications for `--since` and `--until`.
//!
//! A cutoff is only useful if you can name it, and nobody remembers the date
//! they debugged something — they remember that it was last week. Everything
//! here resolves to the same `YYYY-MM-DD` string the filters already compare
//! against, so the engine keeps doing a string comparison and the picker's
//! state file keeps holding a plain date rather than a spec that would drift
//! under it as the session ran past midnight.

use chrono::{Duration, Local, Months, NaiveDate};

/// What a date spec may look like, for error messages and the usage text.
pub const FORMS: &str = "YYYY-MM-DD, today, yesterday, 7d, 2w, 3m, 1y, last-week, last-month";

/// Resolve a spec against today's date in the local zone.
pub fn resolve(spec: &str) -> Result<String, String> {
    against(spec, Local::now().date_naive())
}

/// The resolver proper, with "today" passed in so it can be tested without
/// waiting for the clock to move.
fn against(spec: &str, today: NaiveDate) -> Result<String, String> {
    let s = spec.trim().to_lowercase();
    let out = match s.as_str() {
        "today" => Some(today),
        "yesterday" => today.checked_sub_signed(Duration::days(1)),
        "last-week" => today.checked_sub_signed(Duration::weeks(1)),
        "last-month" => today.checked_sub_months(Months::new(1)),
        "last-year" => today.checked_sub_months(Months::new(12)),
        _ => relative(&s, today).or_else(|| absolute(&s)),
    };
    out.map(|d| d.format("%Y-%m-%d").to_string())
        .ok_or_else(|| format!("bad date '{spec}' — want {FORMS}"))
}

/// `7d`, `2w`, `3m`, `1y` — a count and a unit, counted back from today.
///
/// Months and years step by calendar rather than by a fixed number of days, so
/// `1m` on the 31st lands on the last day of the previous month instead of
/// somewhere in the middle of it.
fn relative(s: &str, today: NaiveDate) -> Option<NaiveDate> {
    let (digits, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: u32 = digits.parse().ok()?;
    match unit {
        "d" => today.checked_sub_signed(Duration::days(n.into())),
        "w" => today.checked_sub_signed(Duration::weeks(n.into())),
        "m" => today.checked_sub_months(Months::new(n)),
        "y" => today.checked_sub_months(Months::new(n.checked_mul(12)?)),
        _ => None,
    }
}

fn absolute(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// The date part of a timestamp, for comparing against a resolved cutoff.
///
/// Transcripts carry `2026-08-24T09:31:00Z` and the prompt history renders
/// `2026-08-24 09:31`; the first ten characters are the date in both.
pub fn day_of(ts: &str) -> &str {
    crate::record::take_chars(ts, 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(spec: &str) -> String {
        against(spec, NaiveDate::from_ymd_opt(2026, 8, 24).unwrap()).unwrap()
    }

    #[test]
    fn an_explicit_date_passes_through() {
        assert_eq!(day("2026-01-05"), "2026-01-05");
    }

    #[test]
    fn the_named_days_are_relative_to_today() {
        assert_eq!(day("today"), "2026-08-24");
        assert_eq!(day("yesterday"), "2026-08-23");
        assert_eq!(day("last-week"), "2026-08-17");
        assert_eq!(day("last-month"), "2026-07-24");
    }

    #[test]
    fn counted_units_step_back_from_today() {
        assert_eq!(day("7d"), "2026-08-17");
        assert_eq!(day("2w"), "2026-08-10");
        assert_eq!(day("3m"), "2026-05-24");
        assert_eq!(day("1y"), "2025-08-24");
    }

    #[test]
    fn zero_is_today_rather_than_an_error() {
        assert_eq!(day("0d"), "2026-08-24");
    }

    /// A month is a calendar step, so subtracting one from the 31st cannot land
    /// on a day September does not have.
    #[test]
    fn months_land_on_a_real_date() {
        let halloween = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert_eq!(against("1m", halloween).unwrap(), "2026-09-30");
    }

    #[test]
    fn specs_are_case_and_space_insensitive() {
        assert_eq!(day(" Yesterday "), "2026-08-23");
        assert_eq!(day("7D"), "2026-08-17");
    }

    #[test]
    fn nonsense_is_rejected_with_the_forms_it_would_accept() {
        let e = against("soonish", NaiveDate::from_ymd_opt(2026, 8, 24).unwrap()).unwrap_err();
        assert!(e.contains("soonish"), "{e}");
        assert!(e.contains("yesterday"), "the error should say what is allowed: {e}");
    }

    /// A date that looks right but names no real day is not silently accepted:
    /// as a raw string compare it would have quietly matched nothing.
    #[test]
    fn an_impossible_date_is_rejected() {
        assert!(against("2026-02-30", NaiveDate::from_ymd_opt(2026, 8, 24).unwrap()).is_err());
        assert!(against("2026-13-01", NaiveDate::from_ymd_opt(2026, 8, 24).unwrap()).is_err());
    }

    #[test]
    fn a_day_is_the_first_ten_characters_of_either_stamp_format() {
        assert_eq!(day_of("2026-08-24T09:31:00.000Z"), "2026-08-24");
        assert_eq!(day_of("2026-08-24 09:31"), "2026-08-24");
        assert_eq!(day_of(""), "");
    }
}
