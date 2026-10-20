use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A point in time with the offset it was written with.
///
/// The old database stores times as text: the front end wrote
/// `dayjs().format()` (`2022-09-05T10:03:34+08:00`), the Rust side fell back
/// to UTC RFC 3339, and a row created without one holds `''`. Comparing those
/// strings, as the old app did, orders `+08:00` and `Z` times wrongly (#24);
/// a `Timestamp` compares the instants.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(DateTime<FixedOffset>);

impl Timestamp {
    pub fn new(at: DateTime<FixedOffset>) -> Self {
        Self(at)
    }

    /// Reads a stored time. What cannot be read — `''` included — is the Unix
    /// epoch, so a damaged value sorts first instead of failing the row.
    pub fn parse_lenient(text: &str) -> Self {
        DateTime::parse_from_rfc3339(text.trim()).map_or_else(|_| Self::default(), Self)
    }

    pub fn get(&self) -> DateTime<FixedOffset> {
        self.0
    }

    /// RFC 3339 to the second with the original offset — the form the old
    /// app wrote, so a row written back reads the same in both.
    pub fn to_rfc3339(&self) -> String {
        self.0.to_rfc3339_opts(SecondsFormat::Secs, true)
    }
}

impl Default for Timestamp {
    fn default() -> Self {
        Self(DateTime::<Utc>::UNIX_EPOCH.fixed_offset())
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
