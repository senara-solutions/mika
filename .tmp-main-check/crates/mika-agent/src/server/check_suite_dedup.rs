//! In-memory dedup gate for `check_suite.completed(success)` webhook events (mika#1869).
//!
//! ## Why this exists
//!
//! A single push to a PR branch triggers up to 8 GitHub workflows. Each workflow
//! emits its own `check_suite.completed(success)` webhook, and the gateway routes
//! every one as a separate message to mika-qa. Each message walks the full
//! [`super::ci_success_handler::try_handle_ci_success`] path (several `gh` calls +
//! a merge attempt) doing *identical* work — `check_suite.completed(success)` is
//! scoped to ONE workflow, but the handler re-aggregates all required checks on
//! every invocation, so N events for the same `(repo, branch, head_sha)` produce
//! at most one state transition. The other N−1 are pure waste that saturate
//! mika-qa's mailbox and trip rate limits.
//!
//! This module provides a precise, process-global dedup keyed on
//! `(repo, branch, head_sha)`. Distinct pushes advance `head_sha`, so a genuine
//! second push to the same branch is never falsely deduped.
//!
//! ## Semantics
//!
//! - First sighting of a key within the last [`DEDUP_WINDOW`] → returns `false`
//!   and records the observation.
//! - Repeat sighting within the window → returns `true` (duplicate). The stored
//!   timestamp is **not** advanced — the window is fixed from the first sighting
//!   (matching the ticket's "60s from first workflow completion").
//! - After the window elapses, the key is treated as fresh again (overwrite +
//!   `false`).
//!
//! Monotonic [`Instant`] is used instead of `SystemTime` so the gate is immune to
//! wall-clock skew.
//!
//! ## Bounded growth
//!
//! The backing map is capped at [`DEDUP_CAP`] entries with a [`ENTRY_TTL`] TTL.
//! Eviction is amortized: on any call, if the map is at capacity, entries older
//! than the TTL are swept before the insert. No background task.
//!
//! Thread-safety comes from `DashMap`'s interior sharding; the check-and-insert
//! is made atomic via the `entry` API (shard lock held across the read+write), so
//! under a concurrent burst exactly one caller sees the key as new.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;

/// Default dedup window — a repeat within this span of the first sighting is a
/// duplicate (ticket spec: 60s from first workflow completion).
pub const DEDUP_WINDOW: Duration = Duration::from_secs(60);

/// Soft capacity of the dedup map. Eviction is triggered when the map reaches
/// this size.
const DEDUP_CAP: usize = 1000;

/// Entries older than this are dropped on the next capacity-triggered sweep.
const ENTRY_TTL: Duration = Duration::from_secs(600);

/// Process-global dedup store. Key = `"{repo}:{branch}:{head_sha}"`, value =
/// monotonic `Instant` of the first sighting within the current window.
static DEDUP_MAP: LazyLock<DashMap<String, Instant>> = LazyLock::new(DashMap::new);

/// Returns `true` if a check_suite success for this exact `(repo, branch, head_sha)`
/// was already seen within `window`. Records the observation as a side effect —
/// the first caller for a key returns `false` and registers it.
pub fn try_dedup_check_suite(repo: &str, branch: &str, head_sha: &str, window: Duration) -> bool {
    try_dedup_in(&DEDUP_MAP, repo, branch, head_sha, window, Instant::now())
}

/// Core logic, parameterized over the backing map and the notion of "now" so it
/// can be exercised deterministically in tests without real sleeps or the shared
/// process-global map.
fn try_dedup_in(
    map: &DashMap<String, Instant>,
    repo: &str,
    branch: &str,
    head_sha: &str,
    window: Duration,
    now: Instant,
) -> bool {
    let key = format!("{repo}:{branch}:{head_sha}");

    // Amortized eviction. Done BEFORE acquiring the per-key `entry` shard lock —
    // `retain` locks every shard, so running it while holding an entry lock would
    // deadlock. Eviction is best-effort; correctness lives in the `entry` block.
    if map.len() >= DEDUP_CAP {
        map.retain(|_, &mut stored| now.saturating_duration_since(stored) < ENTRY_TTL);
    }

    // Atomic check-and-insert: `entry` holds the shard lock across the read and
    // the write, so a concurrent burst on the same key yields exactly one Vacant.
    match map.entry(key) {
        Entry::Occupied(mut e) => {
            let stored = *e.get();
            if now.saturating_duration_since(stored) < window {
                // Duplicate within the window. Do not advance the timestamp —
                // the window is fixed from the first sighting.
                true
            } else {
                // Window elapsed — treat as a fresh sighting.
                e.insert(now);
                false
            }
        }
        Entry::Vacant(e) => {
            e.insert(now);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const REPO: &str = "senara-solutions/mika";
    const BRANCH: &str = "feat/dedup";
    const SHA: &str = "abc123";

    #[test]
    fn first_call_false_second_call_true() {
        let map = DashMap::new();
        let now = Instant::now();
        assert!(
            !try_dedup_in(&map, REPO, BRANCH, SHA, DEDUP_WINDOW, now),
            "first sighting must not be a duplicate"
        );
        assert!(
            try_dedup_in(&map, REPO, BRANCH, SHA, DEDUP_WINDOW, now),
            "immediate second sighting of the same key must be a duplicate"
        );
    }

    #[test]
    fn distinct_head_sha_not_deduped() {
        // The core ticket invariant: a genuine second push (different head_sha)
        // to the same (repo, branch) is never falsely deduped.
        let map = DashMap::new();
        let now = Instant::now();
        assert!(!try_dedup_in(
            &map,
            REPO,
            BRANCH,
            "sha-one",
            DEDUP_WINDOW,
            now
        ));
        assert!(
            !try_dedup_in(&map, REPO, BRANCH, "sha-two", DEDUP_WINDOW, now),
            "a different head_sha for the same (repo, branch) is a distinct push"
        );
    }

    #[test]
    fn window_expiry_returns_false() {
        let map = DashMap::new();
        let base = Instant::now();
        let window = Duration::from_secs(60);

        // First sighting registers the key at `base`.
        assert!(!try_dedup_in(&map, REPO, BRANCH, SHA, window, base));

        // Still inside the window → duplicate.
        let inside = base + Duration::from_secs(30);
        assert!(try_dedup_in(&map, REPO, BRANCH, SHA, window, inside));

        // Past the window → fresh again.
        let outside = base + Duration::from_secs(61);
        assert!(
            !try_dedup_in(&map, REPO, BRANCH, SHA, window, outside),
            "a sighting after the window elapsed must be treated as fresh"
        );

        // ...and it re-armed the window from `outside`.
        assert!(try_dedup_in(&map, REPO, BRANCH, SHA, window, outside));
    }

    #[test]
    fn concurrent_same_key_exactly_one_false() {
        // AC3: 5 concurrent callers, exactly one non-duplicate.
        let map = Arc::new(DashMap::new());
        let false_count = Arc::new(AtomicUsize::new(0));
        let now = Instant::now();

        let mut handles = Vec::new();
        for _ in 0..5 {
            let map = Arc::clone(&map);
            let false_count = Arc::clone(&false_count);
            handles.push(std::thread::spawn(move || {
                if !try_dedup_in(&map, REPO, BRANCH, SHA, DEDUP_WINDOW, now) {
                    false_count.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            false_count.load(Ordering::SeqCst),
            1,
            "exactly one of five concurrent callers must see the key as new"
        );
    }

    #[test]
    fn storm_eight_identical_events_exactly_one_false() {
        // AC5: fire 8 identical (repo, branch, head_sha) events within the window;
        // exactly one takes the downstream verdict path (≤1 merge evaluation).
        let map = DashMap::new();
        let now = Instant::now();
        let mut false_count = 0;
        for _ in 0..8 {
            if !try_dedup_in(&map, REPO, BRANCH, SHA, DEDUP_WINDOW, now) {
                false_count += 1;
            }
        }
        assert_eq!(
            false_count, 1,
            "8 identical check_suite events must yield exactly one non-duplicate"
        );
    }

    #[test]
    fn eviction_bounds_map_size() {
        let map = DashMap::new();
        let base = Instant::now();
        let window = DEDUP_WINDOW;

        // Fill to capacity with keys registered at `base`.
        for i in 0..DEDUP_CAP {
            let sha = format!("sha-{i}");
            assert!(!try_dedup_in(&map, REPO, BRANCH, &sha, window, base));
        }
        assert_eq!(map.len(), DEDUP_CAP);

        // One more insert, far enough in the future that every existing entry is
        // older than the TTL → the capacity-triggered sweep drops them all before
        // the new key lands.
        let later = base + ENTRY_TTL + Duration::from_secs(1);
        assert!(!try_dedup_in(&map, REPO, BRANCH, "fresh", window, later));
        assert!(
            map.len() <= DEDUP_CAP,
            "map must stay bounded after eviction, got {}",
            map.len()
        );
        assert!(
            map.len() < DEDUP_CAP,
            "stale entries older than the TTL must be swept, got {}",
            map.len()
        );
    }

    #[test]
    fn public_api_dedups_on_shared_map() {
        // Exercise the real process-global path. Use a unique key so parallel
        // tests sharing the global map cannot collide.
        let sha = "public-api-unique-sha-9f8e7d";
        assert!(!try_dedup_check_suite(REPO, BRANCH, sha, DEDUP_WINDOW));
        assert!(try_dedup_check_suite(REPO, BRANCH, sha, DEDUP_WINDOW));
    }
}
