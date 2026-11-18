use jiff::civil::DateTime;
use jiff::fmt::temporal::Pieces;
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp as JiffTimestamp, ZonedDateTime};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// A point in time with the offset it was written with.
///
/// The old database stores times as text: the front end wrote
/// `dayjs().format()` (`2022-09-05T10:03:34+08:00`), the Rust side fell back
/// to UTC RFC 3339, and a row created without one holds `''`. Comparing those
/// strings, as the old app did, orders `+08:00` and `Z` times wrongly (#24);
/// a `Timestamp` compares the instants.
#[derive(Clone, Copy)]
pub struct Timestamp {
    instant: JiffTimestamp,
    offset: Offset,
}

impl Timestamp {
    pub fn new(instant: JiffTimestamp, offset: Offset) -> Self {
        Self { instant, offset }
    }

    pub fn from_utc(instant: JiffTimestamp) -> Self {
        Self {
            instant,
            offset: Offset::UTC,
        }
    }

    pub fn from_zoned(at: &ZonedDateTime) -> Self {
        Self {
            instant: at.timestamp(),
            offset: at.offset(),
        }
    }

    /// Reads a stored time. What cannot be read — `''` included — is the Unix
    /// epoch, so a damaged value sorts first instead of failing the row.
    pub fn parse_lenient(text: &str) -> Self {
        let text = text.trim();
        let Ok(instant) = text.parse::<JiffTimestamp>() else {
            return Self::default();
        };
        let offset = Pieces::parse(text)
            .ok()
            .and_then(|pieces| pieces.to_numeric_offset())
            .unwrap_or(Offset::UTC);
        Self { instant, offset }
    }

    pub fn instant(&self) -> JiffTimestamp {
        self.instant
    }

    pub fn offset(&self) -> Offset {
        self.offset
    }

    pub fn to_datetime(&self) -> DateTime {
        self.offset.to_datetime(self.instant)
    }

    pub fn in_offset(&self, offset: Offset) -> DateTime {
        offset.to_datetime(self.instant)
    }

    pub fn in_zone(&self, tz: &TimeZone) -> ZonedDateTime {
        self.instant.to_zoned_datetime(tz.clone())
    }

    pub fn as_second(&self) -> i64 {
        self.instant.as_second()
    }

    /// Seconds since the Unix epoch, in UTC: "now" as a caller's clock gives
    /// it.
    pub fn from_unix(seconds: i64) -> Self {
        JiffTimestamp::from_second(seconds).map_or_else(|_| Self::default(), Self::from_utc)
    }

    /// RFC 3339 to the second with the original offset — the form the old
    /// app wrote, so a row written back reads the same in both.
    pub fn to_rfc3339(&self) -> String {
        let dt = self.to_datetime();
        let base = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second(),
        );
        if self.offset == Offset::UTC {
            format!("{base}Z")
        } else {
            let total = self.offset.seconds();
            let sign = if total < 0 { '-' } else { '+' };
            let abs = total.abs();
            let hours = abs / 3600;
            let minutes = (abs % 3600) / 60;
            format!("{base}{sign}{hours:02}:{minutes:02}")
        }
    }
}

impl Default for Timestamp {
    fn default() -> Self {
        Self::from_utc(JiffTimestamp::UNIX_EPOCH)
    }
}

impl PartialEq for Timestamp {
    fn eq(&self, other: &Self) -> bool {
        self.instant == other.instant
    }
}

impl Eq for Timestamp {}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        self.instant.cmp(&other.instant)
    }
}

impl Hash for Timestamp {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.instant.hash(state);
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timestamp({})", self.to_rfc3339())
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_rfc3339())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self::parse_lenient(&text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_are_compared_across_offsets() {
        // 10:00 in Beijing is 02:00 UTC, so it is the earlier of the two even
        // though its text sorts later.
        let beijing = Timestamp::parse_lenient("2022-09-05T10:00:00+08:00");
        let utc = Timestamp::parse_lenient("2022-09-05T03:00:00Z");
        assert!(beijing < utc);
        assert!("2022-09-05T10:00:00+08:00" > "2022-09-05T03:00:00Z");
    }

    #[test]
    fn unreadable_times_are_the_epoch() {
        assert_eq!(Timestamp::parse_lenient(""), Timestamp::default());
        assert_eq!(Timestamp::parse_lenient("yesterday"), Timestamp::default());
    }

    #[test]
    fn written_back_in_the_old_form() {
        let at = Timestamp::parse_lenient("2022-09-05T10:03:34+08:00");
        assert_eq!(at.to_rfc3339(), "2022-09-05T10:03:34+08:00");
        let utc = Timestamp::parse_lenient("2022-09-05T02:03:34.123456Z");
        assert_eq!(utc.to_rfc3339(), "2022-09-05T02:03:34Z");
    }
}
