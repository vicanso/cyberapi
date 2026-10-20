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
    fn earlier_years_show_the_date() {
        let now = Timestamp::parse_lenient("2026-10-20T21:00:00+08:00");
        assert_eq!(
            short_time(Timestamp::parse_lenient("2022-09-05T10:03:34+08:00"), now),
            "2022-09-05"
        );
        assert_eq!(short_time(Timestamp::default(), now), "");
    }
}
