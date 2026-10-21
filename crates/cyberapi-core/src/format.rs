//! How values are written for people to read (F15.6).

use crate::Timestamp;
use chrono::Datelike;

/// A time, short: `MM-DD HH:mm` within the reader's current year,
/// `YYYY-MM-DD` before it. Shown in `now`'s offset — the reader's — whatever
/// offset it was stored with. An unknown time (the epoch) is empty.
pub fn short_time(at: Timestamp, now: Timestamp) -> String {
    if at == Timestamp::default() {
        return String::new();
    }
    let local = at.get().with_timezone(now.get().offset());
    if local.year() == now.get().year() {
        local.format("%m-%d %H:%M").to_string()
    } else {
        local.format("%Y-%m-%d").to_string()
    }
}

/// How long ago something was, for a list that is read at a glance: the
/// moment, minutes, hours, the days of the last week; before that its date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ago {
    JustNow,
    Minutes(i64),
    Hours(i64),
    Days(i64),
    /// `MM-DD` within the reader's current year, `YYYY-MM-DD` before it;
    /// empty for an unknown time.
    On(String),
}

pub fn ago(at: Timestamp, now: Timestamp) -> Ago {
    if at == Timestamp::default() {
        return Ago::On(String::new());
    }
    let elapsed = now.get().signed_duration_since(at.get());
    match elapsed.num_minutes() {
        // A clock behind the store's reads as now.
        ..1 => Ago::JustNow,
        minutes @ 1..60 => Ago::Minutes(minutes),
        minutes if minutes < 24 * 60 => Ago::Hours(minutes / 60),
        minutes if minutes < 7 * 24 * 60 => Ago::Days(minutes / (24 * 60)),
        _ => {
            let local = at.get().with_timezone(now.get().offset());
            let format = if local.year() == now.get().year() {
                "%m-%d"
            } else {
                "%Y-%m-%d"
            };
            Ago::On(local.format(format).to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_year_shows_the_day_and_time() {
        let now = Timestamp::parse_lenient("2026-10-20T21:00:00+08:00");
        assert_eq!(
            short_time(Timestamp::parse_lenient("2026-03-05T09:07:00+08:00"), now),
            "03-05 09:07"
        );
        // Stored in UTC, read in the reader's offset.
        assert_eq!(
            short_time(Timestamp::parse_lenient("2026-03-05T01:07:00Z"), now),
            "03-05 09:07"
        );
    }

    #[test]
    fn ago_reads_at_a_glance() {
        let now = Timestamp::parse_lenient("2026-10-20T21:00:00+08:00");
        let at = |text: &str| ago(Timestamp::parse_lenient(text), now);
        assert_eq!(at("2026-10-20T20:59:30+08:00"), Ago::JustNow);
        assert_eq!(
            at("2026-10-20T21:05:00+08:00"),
            Ago::JustNow,
            "a time ahead of the clock"
        );
        assert_eq!(at("2026-10-20T20:15:00+08:00"), Ago::Minutes(45));
        assert_eq!(at("2026-10-20T10:00:00Z"), Ago::Hours(3));
        assert_eq!(at("2026-10-14T21:00:00+08:00"), Ago::Days(6));
        assert_eq!(at("2026-10-13T20:00:00+08:00"), Ago::On("10-13".into()));
        assert_eq!(at("2023-07-15T10:00:00+08:00"), Ago::On("2023-07-15".into()));
        assert_eq!(ago(Timestamp::default(), now), Ago::On(String::new()));
    }

    #[test]
    fn earlier_years_show_the_date() {
        let now = Timestamp::parse_lenient("2026-10-20T21:00:00+08:00");
        assert_eq!(
            short_time(Timestamp::parse_lenient("2022-09-05T10:03:34+08:00"), now),
            "2022-09-05"
        );
        assert_eq!(short_time(Timestamp::default(), now), "");
    }
}
