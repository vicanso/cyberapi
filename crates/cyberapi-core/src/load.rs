//! A7: a load test — one API sent over and over by a number of senders at
//! once, a number of times or for a while — and what came of it: how many
//! went and how many failed, how many a second, how long they took (the
//! percentiles), and how they ended. It is only shown: nothing goes into
//! the history.

use std::time::Duration;

/// The most senders at once.
pub const MAX_SENDERS: u32 = 50;
/// The most sends.
pub const MAX_COUNT: u32 = 100_000;
/// The longest a test goes on, in seconds.
pub const MAX_SECONDS: u32 = 600;
/// The most sends a test that goes on for a while makes: each is kept to
/// be counted.
pub const MAX_SENDS: u32 = 1_000_000;

/// When a test ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// Once this many have been sent.
    Count(u32),
    /// Once this many seconds have gone by: the sends under way then end
    /// as they will.
    Seconds(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// How many send at once.
    pub senders: u32,
    pub limit: Limit,
}

impl Plan {
    /// Within the bounds: 1 to [`MAX_SENDERS`] senders, 1 to
    /// [`MAX_COUNT`] sends or [`MAX_SECONDS`] seconds.
    pub fn bounded(self) -> Self {
        Self {
            senders: self.senders.clamp(1, MAX_SENDERS),
            limit: match self.limit {
                Limit::Count(count) => Limit::Count(count.clamp(1, MAX_COUNT)),
                Limit::Seconds(seconds) => Limit::Seconds(seconds.clamp(1, MAX_SECONDS)),
            },
        }
    }

    /// Whether one more send starts, `started` having, `elapsed` into the
    /// test.
    pub fn goes_on(&self, started: u32, elapsed: Duration) -> bool {
        match self.limit {
            Limit::Count(count) => started < count,
            Limit::Seconds(seconds) => started < MAX_SENDS && elapsed < Duration::from_secs(u64::from(seconds)),
        }
    }

    /// How many senders are worth starting: no more than there are sends.
    pub fn senders_needed(&self) -> u32 {
        match self.limit {
            Limit::Count(count) => self.senders.min(count),
            Limit::Seconds(_) => self.senders,
        }
    }
}

/// One send of a test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    /// When it ended, from the test's start.
    pub at: Duration,
    /// How long it took.
    pub took: Duration,
    /// What the server answered; `None` when it did not.
    pub status: Option<u16>,
    /// Why it was not answered.
    pub error: Option<String>,
    /// The body's size.
    pub size: u64,
}

impl Sample {
    /// Not answered, or answered with an error status: there are no
    /// assertions to say otherwise.
    pub fn failed(&self) -> bool {
        self.status.is_none_or(|status| status >= 400)
    }
}

/// How a send ended.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Outcome {
    Status(u16),
    /// Not answered, and why.
    Error(String),
}

/// What came of a test so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub sent: usize,
    pub failed: usize,
    /// Sends a second, over the time the test has taken.
    pub per_second: f64,
    pub min: Duration,
    pub mean: Duration,
    pub p50: Duration,
    pub p90: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub max: Duration,
    /// The bodies' sizes, all told.
    pub bytes: u64,
    /// How the sends ended, the most common first.
    pub outcomes: Vec<(Outcome, usize)>,
}

impl Summary {
    /// The share of the sends that failed, 0 to 1.
    pub fn error_rate(&self) -> f64 {
        match self.sent {
            0 => 0.0,
            sent => self.failed as f64 / sent as f64,
        }
    }
}

/// The `percent`th percentile of `sorted`, by nearest rank: the least
/// value no fewer than that share of them are at or under.
pub fn percentile(sorted: &[Duration], percent: u32) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = (u64::from(percent.min(100)) * sorted.len() as u64).div_ceil(100).max(1);
    sorted[(rank as usize - 1).min(sorted.len() - 1)]
}

/// What `samples` came to, `elapsed` into the test.
pub fn summary(samples: &[Sample], elapsed: Duration) -> Summary {
    if samples.is_empty() {
        return Summary::default();
    }
    let mut took: Vec<Duration> = samples.iter().map(|sample| sample.took).collect();
    took.sort_unstable();
    let total: Duration = took.iter().sum();
    let mut outcomes: Vec<(Outcome, usize)> = Vec::new();
    for sample in samples {
        let outcome = match (&sample.status, &sample.error) {
            (Some(status), _) => Outcome::Status(*status),
            (None, error) => Outcome::Error(error.clone().unwrap_or_default()),
        };
        match outcomes.iter_mut().find(|(known, _)| *known == outcome) {
            Some((_, count)) => *count += 1,
            None => outcomes.push((outcome, 1)),
        }
    }
    outcomes.sort_by(|(a, a_count), (b, b_count)| b_count.cmp(a_count).then_with(|| a.cmp(b)));
    let seconds = elapsed.as_secs_f64();
    Summary {
        sent: samples.len(),
        failed: samples.iter().filter(|sample| sample.failed()).count(),
        per_second: if seconds > 0.0 {
            samples.len() as f64 / seconds
        } else {
            0.0
        },
        min: took[0],
        mean: total / took.len() as u32,
        p50: percentile(&took, 50),
        p90: percentile(&took, 90),
        p95: percentile(&took, 95),
        p99: percentile(&took, 99),
        max: took[took.len() - 1],
        bytes: samples.iter().map(|sample| sample.size).sum(),
        outcomes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn ms(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    fn sample(took: u64, status: Option<u16>) -> Sample {
        Sample {
            at: ms(took),
            took: ms(took),
            status,
            error: status.is_none().then(|| "refused".to_string()),
            size: 10,
        }
    }

    #[test]
    fn percentiles_by_nearest_rank() {
        let sorted: Vec<Duration> = (1..=100).map(ms).collect();
        assert_eq!(percentile(&sorted, 50), ms(50));
        assert_eq!(percentile(&sorted, 95), ms(95));
        assert_eq!(percentile(&sorted, 99), ms(99));
        assert_eq!(percentile(&sorted, 100), ms(100));
        assert_eq!(percentile(&sorted, 0), ms(1));
        let few = [ms(10), ms(20), ms(30)];
        assert_eq!(percentile(&few, 50), ms(20));
        assert_eq!(percentile(&few, 99), ms(30));
        assert_eq!(percentile(&[], 50), Duration::ZERO);
    }

    #[test]
    fn what_a_test_came_to() {
        let samples = [
            sample(30, Some(200)),
            sample(10, Some(200)),
            sample(20, Some(503)),
            sample(40, None),
        ];
        let summary = summary(&samples, Duration::from_secs(2));
        assert_eq!((summary.sent, summary.failed), (4, 2));
        assert_eq!(summary.error_rate(), 0.5);
        assert_eq!(summary.per_second, 2.0);
        assert_eq!((summary.min, summary.mean, summary.max), (ms(10), ms(25), ms(40)));
        assert_eq!((summary.p50, summary.p99), (ms(20), ms(40)));
        assert_eq!(summary.bytes, 40);
        assert_eq!(
            summary.outcomes,
            [
                (Outcome::Status(200), 2),
                (Outcome::Status(503), 1),
                (Outcome::Error("refused".into()), 1)
            ]
        );
        assert_eq!(super::summary(&[], ms(5)), Summary::default());
    }

    #[test]
    fn a_plan_within_bounds_and_when_it_ends() {
        let plan = Plan {
            senders: 80,
            limit: Limit::Count(0),
        }
        .bounded();
        assert_eq!(
            plan,
            Plan {
                senders: MAX_SENDERS,
                limit: Limit::Count(1)
            }
        );
        assert_eq!(plan.senders_needed(), 1, "no more senders than sends");
        assert!(plan.goes_on(0, ms(0)) && !plan.goes_on(1, ms(0)));
        let timed = Plan {
            senders: 4,
            limit: Limit::Seconds(2),
        };
        assert!(timed.goes_on(MAX_SENDS - 1, ms(1_999)) && !timed.goes_on(0, ms(2_000)));
        assert!(!timed.goes_on(MAX_SENDS, ms(0)));
        assert_eq!(timed.senders_needed(), 4);
    }
}
