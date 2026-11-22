//! F1: Stable Top-5 Frecency ordering and eviction for workspace open tabs.
//!
//! Each API carries an exponentially decayed usage score (`5`-day half-life)
//! paired with a `30`-minute short-term recency boost when ranking tabs.
//! Opening or switching to a tab adds [`OPEN_WEIGHT`]; sending a request adds
//! [`SEND_WEIGHT`].
//!
//! To preserve spatial muscle memory across the most-used tabs, the top
//! [`TOP_STABLE`] slots form a stable window: existing Top-5 tabs keep their
//! relative positions even when their internal scores overtake each other. Only
//! when a tab outside the Top-5 outscores the lowest Top-5 member does it
//! promote (into slot #5, right after the surviving Top-4), demoting that
//! lowest member to #6+.

use crate::ItemId;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};

/// Number of leading tab slots whose relative order stays frozen while all
/// remain in the top group.
pub const TOP_STABLE: usize = 5;

/// Maximum per-project usage entries retained in [`crate::prefs::WorkspacePrefs`].
pub const MAX_USAGE_RECORDS: usize = 200;

/// Score added when an API is opened or switched to.
pub const OPEN_WEIGHT: f64 = 1.0;

/// Score added when a request is sent from an API.
pub const SEND_WEIGHT: f64 = 3.0;

/// Long-term half-life (5 days in seconds).
const LONG_HALF_LIFE_SECS: f64 = 5.0 * 86_400.0;

/// Short-term recency boost half-life (30 minutes in seconds).
const SHORT_HALF_LIFE_SECS: f64 = 30.0 * 60.0;

/// Initial magnitude of the short-term recency boost so a newly opened tab
/// remains near the front viewport without overtaking established core tabs.
const SHORT_BOOST: f64 = 6.0;

/// Fixed-point milli-unit multiplier so scores serialize cleanly as integers
/// while preserving `Eq` on `WorkspacePrefs`.
const SCALE: f64 = 1000.0;

/// Recorded usage state of one API in a workspace's tab strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TabUsage {
    /// Decayed cumulative frequency in milli-units (`1.0` == `1000`),
    /// stored as of `last_used`.
    pub score_millis: u64,
    /// Unix timestamp in seconds when the API was last opened, switched to, or sent.
    pub last_used: i64,
}

fn decay(value: f64, age_secs: i64, half_life_secs: f64) -> f64 {
    if value <= 0.0 {
        return 0.0;
    }
    if age_secs <= 0 {
        return value;
    }
    value * 0.5_f64.powf(age_secs as f64 / half_life_secs)
}

fn to_millis(score: f64) -> u64 {
    if !score.is_finite() || score <= 0.0 {
        return 0;
    }
    score.round().clamp(0.0, u64::MAX as f64) as u64
}

/// Records an interaction (`weight`) on `id` at Unix timestamp `now` (seconds).
pub fn record(usage: &mut BTreeMap<ItemId, TabUsage>, id: &ItemId, weight: f64, now: i64) {
    let prev = usage.get(id).copied().unwrap_or_default();
    let base = if prev.last_used == 0 {
        prev.score_millis as f64 / SCALE
    } else {
        decay(
            prev.score_millis as f64 / SCALE,
            now.saturating_sub(prev.last_used),
            LONG_HALF_LIFE_SECS,
        )
    };
    let next = (base + weight).max(0.0);
    let entry = usage.entry(id.clone()).or_default();
    entry.score_millis = to_millis(next * SCALE);
    entry.last_used = if prev.last_used == 0 {
        now
    } else {
        now.max(prev.last_used)
    };
}

/// Effective Frecency score of `id` at `now` (long-term decayed score plus
/// short-term recency boost). Returns `0.0` when `id` has never been recorded.
pub fn score(usage: &BTreeMap<ItemId, TabUsage>, id: &ItemId, now: i64) -> f64 {
    let Some(entry) = usage.get(id) else {
        return 0.0;
    };
    let age = now.saturating_sub(entry.last_used);
    let long_term = decay(entry.score_millis as f64 / SCALE, age, LONG_HALF_LIFE_SECS);
    let recency = decay(SHORT_BOOST, age, SHORT_HALF_LIFE_SECS);
    long_term + recency
}

/// Integer comparison key `(score_millis, last_used)` for ranking and eviction.
fn rank_key(usage: &BTreeMap<ItemId, TabUsage>, id: &ItemId, now: i64) -> (u64, i64) {
    let pts = to_millis(score(usage, id, now) * SCALE);
    let last_used = usage.get(id).map_or(i64::MIN, |u| u.last_used);
    (pts, last_used)
}

/// Reorders `open` using Frecency while keeping the first `top_k` (`5`) tabs in
/// a stable order:
///
/// 1. Select the top `top_k` tabs by Frecency score (ties keep current Top-K
///    incumbents to avoid churn).
/// 2. Surviving Top-K incumbents stay in their original relative order at the front.
/// 3. Newly promoted tabs (from index `>= top_k`) follow immediately, sorted by
///    score descending (so a single newcomer overtaking the weakest Top-5 tab
///    lands cleanly in slot #5).
/// 4. Remaining tabs follow sorted by score descending, then `last_used`
///    descending, then prior order.
pub fn reorder(open: &[ItemId], usage: &BTreeMap<ItemId, TabUsage>, now: i64, top_k: usize) -> Vec<ItemId> {
    if open.len() <= 1 || usage.is_empty() {
        return open.to_vec();
    }
    let effective_top = top_k.min(open.len());

    let scored: Vec<(usize, u64, i64)> = open
        .iter()
        .enumerate()
        .map(|(idx, id)| {
            let (pts, last_used) = rank_key(usage, id, now);
            (idx, pts, last_used)
        })
        .collect();

    if effective_top == 0 {
        let mut all = scored;
        all.sort_by_key(|&(idx, pts, last_used)| (Reverse(pts), Reverse(last_used), idx));
        return all.into_iter().map(|(idx, ..)| open[idx].clone()).collect();
    }

    let mut ranked = scored.clone();
    ranked.sort_by_key(|&(idx, pts, last_used)| (Reverse(pts), Reverse(idx < effective_top), Reverse(last_used), idx));

    let new_top: HashSet<usize> = ranked.iter().take(effective_top).map(|&(idx, ..)| idx).collect();

    let mut result = Vec::with_capacity(open.len());

    // 1. Existing Top-K survivors retain their exact prior relative order.
    for &(idx, ..) in &scored[..effective_top] {
        if new_top.contains(&idx) {
            result.push(open[idx].clone());
        }
    }

    // 2. Promoted entrants from outside Top-K land right after survivors.
    let mut promoted: Vec<(usize, u64, i64)> = scored[effective_top..]
        .iter()
        .copied()
        .filter(|(idx, ..)| new_top.contains(idx))
        .collect();
    promoted.sort_by_key(|&(idx, pts, last_used)| (Reverse(pts), Reverse(last_used), idx));
    for (idx, ..) in promoted {
        result.push(open[idx].clone());
    }

    // 3. Remaining tabs sorted by Frecency descending.
    let mut rest: Vec<(usize, u64, i64)> = scored.into_iter().filter(|(idx, ..)| !new_top.contains(idx)).collect();
    rest.sort_by_key(|&(idx, pts, last_used)| (Reverse(pts), Reverse(last_used), idx));
    for (idx, ..) in rest {
        result.push(open[idx].clone());
    }

    result
}

/// Picks the open tab to close when `open` exceeds `MAX_TABS`, choosing the
/// lowest-Frecency unprotected tab (breaking ties by oldest `last_used`, then
/// smallest `recency_rank`).
pub fn pick_eviction<'a>(
    open: &'a [ItemId],
    usage: &BTreeMap<ItemId, TabUsage>,
    now: i64,
    protected: impl Fn(&ItemId) -> bool,
    recency_rank: impl Fn(&ItemId) -> usize,
) -> Option<&'a ItemId> {
    open.iter().filter(|id| !protected(id)).min_by_key(|id| {
        let (pts, last_used) = rank_key(usage, id, now);
        (pts, last_used, recency_rank(id))
    })
}

/// Removes dead API IDs from `usage` and caps total entries to `max_entries`
/// by dropping the lowest-Frecency entries.
pub fn prune_usage(
    usage: &mut BTreeMap<ItemId, TabUsage>,
    alive: impl Fn(&ItemId) -> bool,
    now: i64,
    max_entries: usize,
) {
    usage.retain(|id, _| alive(id));
    if usage.len() <= max_entries {
        return;
    }
    let mut entries: Vec<(u64, i64, ItemId)> = usage
        .keys()
        .map(|id| {
            let (pts, last_used) = rank_key(usage, id, now);
            (pts, last_used, id.clone())
        })
        .collect();
    entries.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    let drop_count = entries.len().saturating_sub(max_entries);
    for (_, _, id) in entries.into_iter().take(drop_count) {
        usage.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn id(name: &str) -> ItemId {
        ItemId::new(name)
    }

    fn seed(entries: &[(&str, u64, i64)]) -> BTreeMap<ItemId, TabUsage> {
        entries
            .iter()
            .map(|(name, score_millis, last_used)| {
                (
                    id(name),
                    TabUsage {
                        score_millis: *score_millis,
                        last_used: *last_used,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn top_five_keep_relative_order_when_internal_scores_change() {
        let now = 1_000_000;
        let open = [id("a"), id("b"), id("c"), id("d"), id("e"), id("f")];
        // Even when `c` and `e` have much higher scores than `a` and `b`,
        // all 5 remain the top 5 so their relative order never jitters.
        let mut usage = seed(&[
            ("a", 20_000, now - 10),
            ("b", 15_000, now - 20),
            ("c", 10_000, now - 30),
            ("d", 12_000, now - 40),
            ("e", 11_000, now - 50),
            ("f", 2_000, now - 60),
        ]);
        record(&mut usage, &id("c"), SEND_WEIGHT, now);
        record(&mut usage, &id("c"), SEND_WEIGHT, now);
        record(&mut usage, &id("e"), SEND_WEIGHT, now);

        let ordered = reorder(&open, &usage, now, TOP_STABLE);
        assert_eq!(ordered, open);
    }

    #[test]
    fn sixth_tab_overtaking_weakest_top5_promotes_to_slot_five() {
        let now = 1_000_000;
        let open = [id("a"), id("b"), id("c"), id("d"), id("e"), id("f")];
        // `b` is the weakest member in the current Top-5 (slot #2).
        let usage = seed(&[
            ("a", 25_000, now - 10),
            ("b", 5_000, now - 10),
            ("c", 20_000, now - 10),
            ("d", 18_000, now - 10),
            ("e", 15_000, now - 10),
            ("f", 12_000, now - 10),
        ]);

        let ordered = reorder(&open, &usage, now, TOP_STABLE);
        assert_eq!(ordered, [id("a"), id("c"), id("d"), id("e"), id("f"), id("b")]);
    }

    #[test]
    fn exact_score_tie_keeps_incumbent_top_five() {
        let now = 1_000_000;
        let open = [id("a"), id("b"), id("c"), id("d"), id("e"), id("f")];
        let usage = seed(&[
            ("a", 10_000, now),
            ("b", 10_000, now),
            ("c", 10_000, now),
            ("d", 10_000, now),
            ("e", 10_000, now),
            ("f", 10_000, now),
        ]);
        assert_eq!(reorder(&open, &usage, now, TOP_STABLE), open);
    }

    #[test]
    fn fewer_than_five_tabs_stay_stable_for_newcomers() {
        let now = 1_000_000;
        let mut open = vec![id("a"), id("b"), id("c")];
        let mut usage = BTreeMap::new();
        record(&mut usage, &id("a"), OPEN_WEIGHT, now);
        record(&mut usage, &id("a"), SEND_WEIGHT, now);
        record(&mut usage, &id("b"), OPEN_WEIGHT, now);
        record(&mut usage, &id("c"), OPEN_WEIGHT, now);

        open.push(id("d"));
        record(&mut usage, &id("d"), OPEN_WEIGHT, now);

        assert_eq!(
            reorder(&open, &usage, now, TOP_STABLE),
            [id("a"), id("b"), id("c"), id("d")]
        );
    }

    #[test]
    fn newly_opened_tab_lands_at_slot_six_over_older_tail_and_sinks_after_boost_decays() {
        let now = 1_000_000;
        let mut usage = seed(&[
            ("t1", 30_000, now - 120),
            ("t2", 28_000, now - 120),
            ("t3", 25_000, now - 120),
            ("t4", 22_000, now - 120),
            ("t5", 20_000, now - 120),
            ("old6", 4_000, now - 7_200),
            ("old7", 3_000, now - 7_200),
        ]);
        record(&mut usage, &id("fresh"), OPEN_WEIGHT, now);

        let open = [
            id("t1"),
            id("t2"),
            id("t3"),
            id("t4"),
            id("t5"),
            id("old6"),
            id("old7"),
            id("fresh"),
        ];
        // Short-term boost (+6.0) puts `fresh` (~7.0) ahead of `old6` (~4.375)
        // at slot #6 without disturbing the Top-5.
        let ordered = reorder(&open, &usage, now, TOP_STABLE);
        assert_eq!(
            ordered,
            [
                id("t1"),
                id("t2"),
                id("t3"),
                id("t4"),
                id("t5"),
                id("fresh"),
                id("old6"),
                id("old7"),
            ]
        );

        // Two hours later the short-term boost has decayed away (~0.375), so
        // `old6` (long-term 4.0) overtakes `fresh` (long-term 1.0).
        let later = now + 7_200;
        let after = reorder(&ordered, &usage, later, TOP_STABLE);
        assert_eq!(
            after,
            [
                id("t1"),
                id("t2"),
                id("t3"),
                id("t4"),
                id("t5"),
                id("old6"),
                id("old7"),
                id("fresh"),
            ]
        );
    }

    #[test]
    fn long_term_half_life_allows_active_tab_to_overtake_stale_high_score() {
        let now = 10_000_000;
        let fifteen_days = 15 * 86_400;
        // `stale` had score 40.0 fifteen days ago -> decayed by 2^-3 = 5.0 now.
        let usage = seed(&[("stale", 40_000, now - fifteen_days), ("active", 8_000, now - 3_600)]);
        assert!(score(&usage, &id("active"), now) > score(&usage, &id("stale"), now));
    }

    #[test]
    fn legacy_empty_usage_preserves_existing_order() {
        let open = [id("a"), id("b"), id("c"), id("d"), id("e"), id("f")];
        assert_eq!(reorder(&open, &BTreeMap::new(), 1_000_000, TOP_STABLE), open);
    }

    #[test]
    fn pick_eviction_prefers_lowest_score_and_breaks_ties_by_recency_rank() {
        let now = 1_000_000;
        let open = [id("a"), id("b"), id("c")];
        let usage = seed(&[("a", 5_000, now), ("b", 1_000, now), ("c", 1_000, now)]);
        let picked = pick_eviction(
            &open,
            &usage,
            now,
            |item| *item == id("b"),
            |item| open.iter().position(|x| x == item).unwrap_or(usize::MAX),
        );
        assert_eq!(picked, Some(&id("c")));

        let tied = seed(&[("a", 1_000, now), ("b", 1_000, now), ("c", 2_000, now)]);
        let oldest_first = pick_eviction(
            &open,
            &tied,
            now,
            |_| false,
            |item| match item.as_str() {
                "a" => 1,
                "b" => 0,
                _ => 2,
            },
        );
        assert_eq!(oldest_first, Some(&id("b")));
    }

    #[test]
    fn prune_usage_drops_dead_ids_and_enforces_cap() {
        let now = 1_000_000;
        let mut usage = seed(&[("dead", 99_000, now), ("low", 1_000, now - 100), ("high", 10_000, now)]);
        prune_usage(&mut usage, |item| *item != id("dead"), now, 1);
        assert_eq!(usage.len(), 1);
        assert!(usage.contains_key(&id("high")));
    }
}
