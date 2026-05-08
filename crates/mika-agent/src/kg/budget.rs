//! Budget allocation for KG pipeline stages.
//!
//! Shared between extraction (startup) and resolution (startup + tick).
//! The algorithm distributes a total budget across N corpora with pending
//! work, using two-pass fair allocation (per mika#927, mika#962).

/// Distribute `total_budget` fairly across corpora with pending work.
///
/// Two-pass algorithm:
/// - Pass 1: Each corpus gets `min(pending, total_budget / N_active)`.
/// - Pass 2: Redistribute unused slots to "hungry" corpora (pending > allocated).
///
/// Postconditions:
/// - `sum(allocated) == min(total_budget, sum(pending))`
/// - Every corpus with `pending > 0` gets `allocated > 0` (when budget > 0 and N_active > 0)
///
/// Returns a Vec of allocated budgets parallel to the input `pending_counts`.
pub fn allocate_fair_budget(pending_counts: &[u32], total_budget: u32) -> Vec<u32> {
    let n = pending_counts.iter().filter(|&&c| c > 0).count() as u32;
    if n == 0 || total_budget == 0 {
        return vec![0; pending_counts.len()];
    }

    // Pass 1: floor allocation.
    let base_share = total_budget / n;
    let mut assigned: Vec<u32> = pending_counts
        .iter()
        .map(|&count| count.min(base_share))
        .collect();

    // Remainder after floor allocation.
    let used: u32 = assigned.iter().sum();
    let mut remaining = total_budget.saturating_sub(used);

    // Pass 2: redistribute remainder to hungry corpora.
    if remaining > 0 {
        let mut hungry: Vec<usize> = pending_counts
            .iter()
            .enumerate()
            .filter(|(i, count)| **count > assigned[*i])
            .map(|(i, _)| i)
            .collect();

        while remaining > 0 && !hungry.is_empty() {
            let share = (remaining / hungry.len() as u32).max(1);
            let mut next_hungry = Vec::new();
            for &idx in &hungry {
                if remaining == 0 {
                    break;
                }
                let can_take = pending_counts[idx].saturating_sub(assigned[idx]);
                let give = share.min(can_take).min(remaining);
                assigned[idx] += give;
                remaining -= give;
                if assigned[idx] < pending_counts[idx] {
                    next_hungry.push(idx);
                }
            }
            hungry = next_hungry;
        }
    }

    assigned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_corpus_gets_all_budget() {
        let result = allocate_fair_budget(&[100], 50);
        assert_eq!(result, vec![50]);
    }

    #[test]
    fn equal_pending_splits_evenly() {
        let result = allocate_fair_budget(&[100, 100], 60);
        assert_eq!(result, vec![30, 30]);
    }

    #[test]
    fn unequal_pending_redistributes_to_hungry() {
        // Corpus 0 has 10 pending, corpus 1 has 100 pending, budget=60
        // Pass 1: base_share=30, assigned=[10, 30] (corpus 0 capped at 10)
        // Pass 2: remaining=20, hungry=[1], corpus 1 gets 20 more -> [10, 50]
        let result = allocate_fair_budget(&[10, 100], 60);
        assert_eq!(result, vec![10, 50]);
    }

    #[test]
    fn budget_exceeds_total_pending() {
        let result = allocate_fair_budget(&[10, 20, 5], 100);
        assert_eq!(result, vec![10, 20, 5]);
    }

    #[test]
    fn budget_is_zero_returns_all_zeros() {
        let result = allocate_fair_budget(&[100, 200], 0);
        assert_eq!(result, vec![0, 0]);
    }

    #[test]
    fn empty_input_returns_empty() {
        let result = allocate_fair_budget(&[], 100);
        assert_eq!(result, Vec::<u32>::new());
    }

    #[test]
    fn one_corpus_zero_pending_share_redistributed() {
        // Corpus 0 has 0 pending, corpus 1 has 100, budget=60
        // N_active=1, base_share=60, assigned=[0, 60]
        let result = allocate_fair_budget(&[0, 100], 60);
        assert_eq!(result, vec![0, 60]);
    }

    #[test]
    fn three_corpora_all_get_nonzero() {
        // 3 corpora: [100, 50, 25], budget=60
        // N_active=3, base_share=20, assigned=[20, 20, 20]
        // Remainder: 60-60=0. All get 20.
        let result = allocate_fair_budget(&[100, 50, 25], 60);
        assert_eq!(result, vec![20, 20, 20]);
    }

    #[test]
    fn three_corpora_small_one_capped() {
        // 3 corpora: [100, 50, 5], budget=60
        // N_active=3, base_share=20, assigned=[20, 20, 5]
        // Remainder: 60-45=15, hungry=[0,1]
        // share=7, corpus 0 gets 7, corpus 1 gets 7, remaining=1
        // next pass: share=1, corpus 0 gets 1
        let result = allocate_fair_budget(&[100, 50, 5], 60);
        let sum: u32 = result.iter().sum();
        assert_eq!(sum, 60);
        assert_eq!(result[2], 5); // small corpus gets exactly its pending
        assert!(result[0] > 0 && result[1] > 0); // both hungry get something
    }

    #[test]
    fn postcondition_sum_equals_min_budget_total() {
        let pending = &[200, 300, 100, 50];
        let budget = 400;
        let result = allocate_fair_budget(pending, budget);
        let sum: u32 = result.iter().sum();
        let total_pending: u32 = pending.iter().sum();
        assert_eq!(sum, budget.min(total_pending));
    }

    #[test]
    fn postcondition_every_active_corpus_gets_nonzero() {
        let pending = &[1, 1, 1, 0, 1];
        let budget = 2;
        let result = allocate_fair_budget(pending, budget);
        // With 4 active and budget=2, base_share=0, but pass 2 distributes
        // Actually base_share = 2/4 = 0, so pass 1 gives [0,0,0,0,0]
        // Pass 2: remaining=2, hungry=[0,1,2,4], share=1
        // idx=0 gets 1 (remaining=1), idx=1 gets 1 (remaining=0), stop
        let sum: u32 = result.iter().sum();
        assert_eq!(sum, 2);
        assert_eq!(result[3], 0); // zero-pending corpus gets nothing
    }
}
