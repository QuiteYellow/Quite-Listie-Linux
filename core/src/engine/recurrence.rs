use chrono::{DateTime, Datelike, Duration, Months, Utc, Weekday};

use crate::model::reminder::{ReminderRepeatMode, ReminderRepeatRule, ReminderRepeatUnit};

/// Calculate the next reminder date after `from` for the given rule and mode.
///
/// - `Fixed`: advance in rule increments from the original `reminder_date`
///   until the result is in the future relative to `now`.
/// - `AfterComplete`: advance by one rule interval from `from` (the completion
///   moment), skipping past occurrences until in the future.
pub fn next_reminder_date(
    from: DateTime<Utc>,
    rule: &ReminderRepeatRule,
    mode: &ReminderRepeatMode,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let mut candidate = from;
    loop {
        candidate = advance(candidate, rule);
        match mode {
            ReminderRepeatMode::Fixed => {
                if candidate > now {
                    return candidate;
                }
            }
            ReminderRepeatMode::AfterComplete => {
                // For after-complete we always want exactly one interval from `from`.
                return if candidate > now { candidate } else { advance(now, rule) };
            }
        }
    }
}

fn advance(dt: DateTime<Utc>, rule: &ReminderRepeatRule) -> DateTime<Utc> {
    let n = rule.interval as i64;
    match rule.unit {
        ReminderRepeatUnit::Day => dt + Duration::days(n),
        ReminderRepeatUnit::Week => dt + Duration::weeks(n),
        ReminderRepeatUnit::Month => dt + Months::new(rule.interval),
        ReminderRepeatUnit::Year => dt + Months::new(rule.interval * 12),
        ReminderRepeatUnit::Weekdays => next_weekday(dt),
    }
}

/// Advance by one weekday (Mon–Fri), skipping Sat and Sun.
fn next_weekday(dt: DateTime<Utc>) -> DateTime<Utc> {
    let mut next = dt + Duration::days(1);
    loop {
        match next.weekday() {
            Weekday::Sat | Weekday::Sun => next = next + Duration::days(1),
            _ => return next,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::reminder::ReminderRepeatUnit;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 9, 0, 0).unwrap()
    }

    #[test]
    fn daily_fixed_skips_past() {
        let rule = ReminderRepeatRule::daily();
        let base = utc(2026, 1, 1);
        let now = utc(2026, 1, 5);
        let next = next_reminder_date(base, &rule, &ReminderRepeatMode::Fixed, now);
        assert!(next > now);
        // Should be 2026-01-06
        assert_eq!(next.day(), 6);
    }

    #[test]
    fn weekdays_skips_weekend() {
        let rule = ReminderRepeatRule::weekdays();
        // Friday
        let base = utc(2026, 5, 8);
        let now = utc(2026, 5, 8) + Duration::hours(1);
        let next = next_reminder_date(base, &rule, &ReminderRepeatMode::Fixed, now);
        // Should be Monday the 11th
        assert_eq!(next.weekday(), Weekday::Mon);
    }
}
