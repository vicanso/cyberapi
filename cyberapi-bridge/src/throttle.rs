//! Failed sign-ins, to slow guessing down: a name, an address or the setup
//! token with too many failures in the last [`WINDOW`] is turned away until
//! the oldest of them is old enough. Kept in memory — one bridge process —
//! and forgotten on restart.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Failures in a window before an account name is turned away. Its owner is
/// too, for that long: the price of not letting a name be guessed at.
const PER_NAME: usize = 5;
/// More for an address, which a whole office may share behind one NAT.
const PER_ADDRESS: usize = 30;
const SETUP: usize = 10;
/// Past this many keys, the ones with nothing left in their window go.
const PRUNE_ABOVE: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// An account name as typed, ignoring case.
    Name(String),
    Address(IpAddr),
    /// Tries at the setup token.
    Setup,
}

impl Key {
    pub fn name(name: &str) -> Self {
        Self::Name(name.trim().to_lowercase())
    }

    fn limit(&self) -> usize {
        match self {
            Self::Name(_) => PER_NAME,
            Self::Address(_) => PER_ADDRESS,
            Self::Setup => SETUP,
        }
    }
}

#[derive(Default)]
pub struct Throttle {
    failures: Mutex<HashMap<Key, VecDeque<Instant>>>,
}

fn forget_old(times: &mut VecDeque<Instant>, now: Instant) {
    while times
        .front()
        .is_some_and(|at| now.saturating_duration_since(*at) >= WINDOW)
    {
        times.pop_front();
    }
}

impl Throttle {
    /// How long until any of `keys` may try again; `None` when all may now.
    pub fn wait(&self, keys: &[Key], now: Instant) -> Option<Duration> {
        let mut failures = self.failures.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        keys.iter()
            .filter_map(|key| {
                let times = failures.get_mut(key)?;
                forget_old(times, now);
                let oldest = *times.front()?;
                (times.len() >= key.limit()).then(|| WINDOW.saturating_sub(now.saturating_duration_since(oldest)))
            })
            .max()
    }

    pub fn failed(&self, keys: &[Key], now: Instant) {
        let mut failures = self.failures.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for key in keys {
            let times = failures.entry(key.clone()).or_default();
            forget_old(times, now);
            times.push_back(now);
        }
        if failures.len() > PRUNE_ABOVE {
            failures.retain(|_, times| {
                forget_old(times, now);
                !times.is_empty()
            });
        }
    }

    /// A sign-in that worked clears its name; the address keeps its count.
    pub fn succeeded(&self, key: &Key) {
        let mut failures = self.failures.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        failures.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_waits_once_it_failed_too_often() {
        let throttle = Throttle::default();
        let start = Instant::now();
        let keys = [Key::name("Alice"), Key::Address([10, 0, 0, 1].into())];
        for minute in 0..PER_NAME as u64 {
            assert_eq!(throttle.wait(&keys, start), None, "attempt {minute}");
            throttle.failed(&keys, start + Duration::from_secs(minute * 60));
        }
        let now = start + Duration::from_secs(5 * 60);
        assert_eq!(
            throttle.wait(&[Key::name(" alice ")], now),
            Some(WINDOW - Duration::from_secs(5 * 60)),
            "until the first failure is a window old, whatever the case"
        );
        assert_eq!(throttle.wait(&[Key::name("bob")], now), None, "another name may");
        assert_eq!(
            throttle.wait(&[Key::Address([10, 0, 0, 1].into())], now),
            None,
            "the address has more to go"
        );
        // A window after the first failure, one more try is allowed.
        assert_eq!(throttle.wait(&keys, start + WINDOW), None);
    }

    #[test]
    fn success_clears_the_name_only() {
        let throttle = Throttle::default();
        let now = Instant::now();
        let address = Key::Address([10, 0, 0, 2].into());
        for _ in 0..PER_ADDRESS {
            throttle.failed(&[Key::name("carol"), address.clone()], now);
        }
        throttle.succeeded(&Key::name("carol"));
        assert_eq!(throttle.wait(&[Key::name("carol")], now), None);
        assert!(throttle.wait(&[address], now).is_some());
    }
}
