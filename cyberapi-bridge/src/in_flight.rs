//! How many requests one account has on their way through `/v1/send` at
//! once: a few, so that one reader cannot hold the server's connections and
//! memory for everyone. One past them is turned away at once rather than
//! queued: behind requests that may take minutes, a queued one only seems to
//! hang. Kept in memory — one bridge process.

use cyberapi_core::AccountId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// Requests on their way for one account.
pub const PER_ACCOUNT: usize = 16;

#[derive(Default)]
pub struct InFlight(Mutex<HashMap<AccountId, usize>>);

/// One request on its way. It makes room again when it is dropped: when the
/// handler ends, or is dropped with the connection it answers.
pub struct Sending {
    flights: Arc<InFlight>,
    account: AccountId,
}

impl InFlight {
    /// A place for one more of `account`'s requests; `None` when it has
    /// [`PER_ACCOUNT`] on their way.
    pub fn start(self: &Arc<Self>, account: &AccountId) -> Option<Sending> {
        let mut counts = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let count = counts.entry(account.clone()).or_default();
        if *count >= PER_ACCOUNT {
            return None;
        }
        *count += 1;
        Some(Sending {
            flights: self.clone(),
            account: account.clone(),
        })
    }

    #[cfg(test)]
    fn accounts(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        let mut counts = self.flights.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = counts.get_mut(&self.account) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.account);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_has_so_many_on_their_way_and_no_more() {
        let flights = Arc::new(InFlight::default());
        let (ann, bob) = (AccountId::new("ann"), AccountId::new("bob"));
        let mut sending: Vec<_> = (0..PER_ACCOUNT).map(|_| flights.start(&ann).expect("room")).collect();
        assert!(flights.start(&ann).is_none(), "one past the limit is turned away");
        let other = flights.start(&bob).expect("another account has its own");

        sending.pop();
        let again = flights.start(&ann).expect("one back makes room");
        drop((sending, again, other));
        assert_eq!(flights.accounts(), 0, "an account with none on their way is forgotten");
    }
}
