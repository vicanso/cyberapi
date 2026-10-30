//! How many requests are on their way through `/v1/send` at once: a few of
//! one account's, so that one reader cannot hold the server's connections
//! and memory for everyone, and so many of all accounts' (PERF-15) — each
//! may hold a 64 MB body and its answer. One past either is turned away at
//! once rather than queued: behind requests that may take minutes, a
//! queued one only seems to hang. Kept in memory — one bridge process.

use cyberapi_core::AccountId;
use cyberapi_http::SendError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// Requests on their way for one account.
pub const PER_ACCOUNT: usize = 16;

/// Requests on their way for all accounts, unless `--max-sends` says
/// otherwise.
pub const MAX_SENDS: usize = 64;

pub struct InFlight {
    counts: Mutex<Counts>,
    /// Of all accounts.
    max: usize,
}

#[derive(Default)]
struct Counts {
    accounts: HashMap<AccountId, usize>,
    all: usize,
}

/// One request on its way. It makes room again when it is dropped: when the
/// handler ends, or is dropped with the connection it answers.
pub struct Sending {
    flights: Arc<InFlight>,
    account: AccountId,
}

impl InFlight {
    /// At most `max` of all accounts' requests on their way, at least one.
    pub fn new(max: usize) -> Self {
        Self {
            counts: Mutex::default(),
            max: max.max(1),
        }
    }

    /// A place for one more of `account`'s requests; [`SendError::Busy`]
    /// when it has [`PER_ACCOUNT`] on their way, [`SendError::ServerBusy`]
    /// when all accounts have as many as the bridge takes.
    pub fn start(self: &Arc<Self>, account: &AccountId) -> Result<Sending, SendError> {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        if counts.accounts.get(account).copied().unwrap_or_default() >= PER_ACCOUNT {
            return Err(SendError::Busy { limit: PER_ACCOUNT });
        }
        if counts.all >= self.max {
            return Err(SendError::ServerBusy { limit: self.max });
        }
        *counts.accounts.entry(account.clone()).or_default() += 1;
        counts.all += 1;
        Ok(Sending {
            flights: self.clone(),
            account: account.clone(),
        })
    }

    #[cfg(test)]
    fn accounts(&self) -> usize {
        self.counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .accounts
            .len()
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        let mut counts = self.flights.counts.lock().unwrap_or_else(PoisonError::into_inner);
        counts.all -= 1;
        if let Some(count) = counts.accounts.get_mut(&self.account) {
            *count -= 1;
            if *count == 0 {
                counts.accounts.remove(&self.account);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_has_so_many_on_their_way_and_no_more() {
        let flights = Arc::new(InFlight::new(MAX_SENDS));
        let (ann, bob) = (AccountId::new("ann"), AccountId::new("bob"));
        let mut sending: Vec<_> = (0..PER_ACCOUNT).map(|_| flights.start(&ann).expect("room")).collect();
        assert_eq!(
            flights.start(&ann).err(),
            Some(SendError::Busy { limit: PER_ACCOUNT }),
            "one past the limit is turned away"
        );
        let other = flights.start(&bob).expect("another account has its own");

        sending.pop();
        let again = flights.start(&ann).expect("one back makes room");
        drop((sending, again, other));
        assert_eq!(flights.accounts(), 0, "an account with none on their way is forgotten");
    }

    /// PERF-15: all accounts together have so many on their way.
    #[test]
    fn all_accounts_have_so_many_on_their_way_and_no_more() {
        let flights = Arc::new(InFlight::new(20));
        let (ann, bob, cat) = (AccountId::new("ann"), AccountId::new("bob"), AccountId::new("cat"));
        let mut sending: Vec<_> = (0..PER_ACCOUNT).map(|_| flights.start(&ann).expect("room")).collect();
        sending.extend((0..4).map(|_| flights.start(&bob).expect("room")));
        assert_eq!(
            flights.start(&cat).err(),
            Some(SendError::ServerBusy { limit: 20 }),
            "an account with none on their way is turned away too"
        );
        assert_eq!(
            flights.start(&ann).err(),
            Some(SendError::Busy { limit: PER_ACCOUNT }),
            "an account at its own limit is told so"
        );
        drop(sending.pop());
        let room = flights.start(&cat).expect("one back makes room");
        drop((sending, room));
        assert_eq!(flights.accounts(), 0);
        assert_eq!(
            flights.counts.lock().unwrap_or_else(PoisonError::into_inner).all,
            0,
            "none on their way"
        );
    }
}
