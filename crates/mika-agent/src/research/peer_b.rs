//! `peer_b` — RT-005 physics pilot, brick 1/5 (mika#1887).
//!
//! A **disposable stub** standing in for a second solver ("peer B") that the
//! agent under study may consult. RT-005 estimates the *interaction* between
//! injected confidence (mika#1888) and actual peer reliability (this module) on
//! planning-token count (mika#1889). This module is the reliability arm of that
//! 2x2 design.
//!
//! Deliberately **not** an MCP server and not a transport of any kind (Vincent,
//! 2026-08-03: stub/scaffold, reversible, per coherence's YAGNI). The transport
//! is incident to the estimand. There is no I/O here — no network, no
//! filesystem, no subprocess.
//!
//! # The reliability knob
//!
//! [`Reliability::Fiable`] answers every item correctly. [`Reliability::Degradee`]
//! answers a seeded subset incorrectly. The degradation rate is the ratified
//! `2/6` — **read as a rate, not an absolute count**. mika#1887's scope requires
//! a fixture of at least 10 items while its acceptance criterion says "exactly
//! 2/6"; mika#1890 fixes the design at 10 items, all of them used, so the `6` is
//! stale from an earlier protocol draft. Applying `n * 2 / 6` in integer
//! division honours both: a 6-item fixture yields exactly 2, and the real
//! 10-item fixture yields 3.
//!
//! # Why the PRNG lives here
//!
//! Reproducibility is a protocol requirement, not a convenience: the 80-run
//! batch in mika#1890 must be replayable from its seed long after it ran. The
//! `rand` crate changes its generator algorithms across major versions, so a
//! routine dependency bump would silently re-select which items get perturbed
//! and invalidate an already-recorded batch. `SplitMix64` is pinned here
//! instead. **Do not swap it for a crate.**
//!
//! # What makes the knob measurable
//!
//! A perturbed answer is another fixture item's answer, never a corrupted
//! string. A consumer cannot route around the knob by noticing malformed
//! output, which is what the manipulation depends on. [`PeerB::perturbed_ids`]
//! reports the *realised* perturbation so a run record can carry it, and
//! [`PeerB::ground_truth`] lets the downstream analyser grade answers without
//! re-deriving truth.

use anyhow::{Result, bail};

/// Numerator of the ratified degradation rate. See the module docs on why
/// `2/6` is a rate rather than a count.
const DEGRADED_NUM: usize = 2;
/// Denominator of the ratified degradation rate.
const DEGRADED_DEN: usize = 6;

/// One fixture item with its known-correct answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    /// Stable slug. Run records in mika#1890 name items by this id.
    pub id: &'static str,
    /// The question posed to peer B.
    pub prompt: &'static str,
    /// The single correct answer. Correctness is decidable by string equality —
    /// no judge, no tolerance window.
    pub truth: &'static str,
}

/// The RT-005 ground-truth fixture: 10 items, each objectively checkable.
///
/// Items are deliberately easy. RT-005 measures how the agent *plans around* a
/// peer answer, not how hard the item is, so difficulty is not a design
/// variable. All truths are distinct, which is what lets degraded mode always
/// find a wrong-but-well-formed substitute.
pub const FIXTURE: &[Item] = &[
    Item {
        id: "rt005-01",
        prompt: "Sum of 47 and 68.",
        truth: "115",
    },
    Item {
        id: "rt005-02",
        prompt: "Reverse the string 'mika'.",
        truth: "akim",
    },
    Item {
        id: "rt005-03",
        prompt: "Number of distinct letters in 'banana'.",
        truth: "3",
    },
    Item {
        id: "rt005-04",
        prompt: "Product of 17 and 23.",
        truth: "391",
    },
    Item {
        id: "rt005-05",
        prompt: "Uppercase the string 'senara'.",
        truth: "SENARA",
    },
    Item {
        id: "rt005-06",
        prompt: "The 7th Fibonacci number, with F(1)=1 and F(2)=1.",
        truth: "13",
    },
    Item {
        id: "rt005-07",
        prompt: "Remainder of 1000 divided by 37.",
        truth: "1",
    },
    Item {
        id: "rt005-08",
        prompt: "Number of characters in 'orchestrator'.",
        truth: "12",
    },
    Item {
        id: "rt005-09",
        prompt: "Largest prime strictly below 50.",
        truth: "47",
    },
    Item {
        id: "rt005-10",
        prompt: "Join 'peer' and 'b' with a hyphen.",
        truth: "peer-b",
    },
];

/// SplitMix64 — pinned for reproducibility. See the module docs.
///
/// Reference: Steele, Lea & Flood, *Fast Splittable Pseudorandom Number
/// Generators* (OOPSLA 2014).
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform-enough draw in `0..n`. Modulo bias is immaterial at fixture
    /// scale (n <= a few dozen) and staying with modulo keeps the stream
    /// definition trivially reproducible by hand.
    fn next_below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0, "next_below requires a non-empty range");
        (self.next_u64() % n as u64) as usize
    }
}

/// The RT-005 reliability factor. Fixed at construction, immutable afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reliability {
    /// Every answer is the item's ground truth.
    Fiable,
    /// A seeded `n * 2 / 6` subset of answers is wrong.
    Degradee,
}

impl Reliability {
    /// Parse the knob from a run-configuration string, so mika#1890 does not
    /// re-implement the mapping. Accepts the protocol's French labels, with or
    /// without the accent, case-insensitively.
    pub fn from_label(label: &str) -> Result<Self> {
        match label.trim().to_lowercase().as_str() {
            "fiable" => Ok(Self::Fiable),
            "degradee" | "dégradée" | "degradée" | "dégradee" => Ok(Self::Degradee),
            other => bail!("unknown RELIABILITY value '{other}' (expected 'fiable' or 'degradee')"),
        }
    }

    /// The canonical ASCII label, for run records.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Fiable => "fiable",
            Self::Degradee => "degradee",
        }
    }
}

/// Peer B's reply to one solve request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerBResponse {
    pub item_id: String,
    /// Peer B's committed answer first, then distractors. Length is at most the
    /// requested `k`.
    pub candidates: Vec<String>,
}

impl PeerBResponse {
    /// Peer B's committed answer — the one the protocol reads at `k = 1`.
    pub fn answer(&self) -> &str {
        &self.candidates[0]
    }
}

/// A configured peer B.
///
/// Answers are computed once at construction, so results cannot depend on call
/// order. Solving is a pure read.
#[derive(Debug, Clone)]
pub struct PeerB {
    fixture: &'static [Item],
    reliability: Reliability,
    seed: u64,
    /// Parallel to `fixture`: the answer this instance will give for each item.
    answers: Vec<&'static str>,
    /// Ids whose answer differs from ground truth, sorted so the set is
    /// canonical regardless of draw order.
    perturbed_ids: Vec<&'static str>,
}

impl PeerB {
    /// Build peer B over the RT-005 fixture.
    pub fn new(reliability: Reliability, seed: u64) -> Result<Self> {
        Self::with_fixture(FIXTURE, reliability, seed)
    }

    /// Build peer B over an explicit fixture. Exists so the protocol's literal
    /// "exactly 2/6" criterion is testable against a 6-item table without
    /// shipping a second production fixture.
    ///
    /// Fails when [`Reliability::Degradee`] is requested and the fixture cannot
    /// supply a wrong-but-well-formed answer for some selected item. That is a
    /// protocol-invalidating condition — a degraded arm that silently perturbs
    /// nothing would make the whole batch measure noise — so it surfaces here
    /// rather than at analysis time.
    pub fn with_fixture(
        fixture: &'static [Item],
        reliability: Reliability,
        seed: u64,
    ) -> Result<Self> {
        if fixture.is_empty() {
            bail!("peer_b fixture is empty");
        }
        // A degraded arm that perturbs nothing would make every downstream
        // measurement noise, so refuse a fixture too small to carry the rate
        // rather than silently answering everything correctly.
        if reliability == Reliability::Degradee && perturbed_count(fixture.len()) == 0 {
            bail!(
                "peer_b fixture of {} items cannot be degraded at the {DEGRADED_NUM}/{DEGRADED_DEN} rate \
                 (needs at least {} items)",
                fixture.len(),
                DEGRADED_DEN.div_ceil(DEGRADED_NUM)
            );
        }

        let mut answers: Vec<&'static str> = fixture.iter().map(|i| i.truth).collect();
        let mut perturbed_ids = Vec::new();

        if reliability == Reliability::Degradee {
            let mut rng = SplitMix64::new(seed);
            for index in select_perturbed_indices(fixture.len(), &mut rng) {
                let donor = pick_donor(fixture, index, &mut rng).ok_or_else(|| {
                    anyhow::anyhow!(
                        "cannot degrade item '{}': no other fixture item has a different answer",
                        fixture[index].id
                    )
                })?;
                answers[index] = fixture[donor].truth;
                perturbed_ids.push(fixture[index].id);
            }
            perturbed_ids.sort_unstable();
        }

        Ok(Self {
            fixture,
            reliability,
            seed,
            answers,
            perturbed_ids,
        })
    }

    /// Ask peer B for up to `k` candidate answers to `item_id`.
    ///
    /// The head is peer B's committed answer — ground truth under `Fiable`, and
    /// under `Degradee` a wrong answer for the seeded subset. `k = 0` is
    /// clamped to 1. Distractors are deterministic and never equal the head.
    ///
    /// For a perturbed item the distractors also exclude that item's ground
    /// truth. Offering the correct answer alongside the wrong one would let a
    /// consumer recover it at any `k > 1` and collapse the manipulation the
    /// experiment rests on.
    pub fn peer_b_solve(&self, item_id: &str, k: usize) -> Result<PeerBResponse> {
        let index = self
            .fixture
            .iter()
            .position(|i| i.id == item_id)
            .ok_or_else(|| anyhow::anyhow!("unknown peer_b item id '{item_id}'"))?;

        let head = self.answers[index];
        let mut candidates = vec![head.to_string()];
        // `None` for a correctly-answered item; the item's own truth when the
        // knob perturbed it.
        let withheld_truth =
            (head != self.fixture[index].truth).then_some(self.fixture[index].truth);

        // Derived per-item so distractors do not depend on how many solves ran
        // before this one. Call order must not change any answer.
        let mut rng =
            SplitMix64::new(self.seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let wanted = k.max(1);
        let start = rng.next_below(self.fixture.len());
        for offset in 0..self.fixture.len() {
            if candidates.len() >= wanted {
                break;
            }
            let candidate = self.fixture[(start + offset) % self.fixture.len()].truth;
            if withheld_truth == Some(candidate) {
                continue;
            }
            if !candidates.iter().any(|c| c == candidate) {
                candidates.push(candidate.to_string());
            }
        }

        Ok(PeerBResponse {
            item_id: item_id.to_string(),
            candidates,
        })
    }

    /// The known-correct answer for `item_id`, for grading a run.
    pub fn ground_truth(&self, item_id: &str) -> Option<&'static str> {
        self.fixture
            .iter()
            .find(|i| i.id == item_id)
            .map(|i| i.truth)
    }

    /// The fixture this instance answers from.
    pub fn fixture(&self) -> &'static [Item] {
        self.fixture
    }

    /// Ids this instance actually answers incorrectly, sorted. Empty under
    /// `Fiable`. Run records should carry this realised set, not the nominal
    /// rate.
    pub fn perturbed_ids(&self) -> &[&'static str] {
        &self.perturbed_ids
    }

    pub fn reliability(&self) -> Reliability {
        self.reliability
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }
}

/// How many items degraded mode perturbs, for a fixture of `n` items.
fn perturbed_count(n: usize) -> usize {
    n * DEGRADED_NUM / DEGRADED_DEN
}

/// Draw `perturbed_count(n)` distinct indices via a partial Fisher-Yates
/// shuffle. Deterministic in the RNG stream.
fn select_perturbed_indices(n: usize, rng: &mut SplitMix64) -> Vec<usize> {
    let count = perturbed_count(n);
    let mut pool: Vec<usize> = (0..n).collect();
    let mut chosen = Vec::with_capacity(count);
    for slot in 0..count {
        let pick = slot + rng.next_below(n - slot);
        pool.swap(slot, pick);
        chosen.push(pool[slot]);
    }
    chosen
}

/// Pick another item whose answer is well-formed but wrong for `index`.
///
/// Draws a start offset then probes forward, so it terminates on any fixture
/// and stays deterministic. Returns `None` only when no item has a different
/// answer.
fn pick_donor(fixture: &'static [Item], index: usize, rng: &mut SplitMix64) -> Option<usize> {
    let n = fixture.len();
    let start = rng.next_below(n);
    (0..n)
        .map(|offset| (start + offset) % n)
        .find(|&donor| donor != index && fixture[donor].truth != fixture[index].truth)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six items, so the protocol's literal "exactly 2/6" criterion is
    /// exercised as written.
    const SIX_ITEM_FIXTURE: &[Item] = &[
        Item {
            id: "six-1",
            prompt: "1+1",
            truth: "2",
        },
        Item {
            id: "six-2",
            prompt: "2+2",
            truth: "4",
        },
        Item {
            id: "six-3",
            prompt: "3+3",
            truth: "6",
        },
        Item {
            id: "six-4",
            prompt: "4+4",
            truth: "8",
        },
        Item {
            id: "six-5",
            prompt: "5+5",
            truth: "10",
        },
        Item {
            id: "six-6",
            prompt: "6+6",
            truth: "12",
        },
    ];

    fn wrong_count(peer: &PeerB) -> usize {
        peer.fixture()
            .iter()
            .filter(|item| {
                let got = peer.peer_b_solve(item.id, 1).expect("fixture id resolves");
                got.answer() != item.truth
            })
            .count()
    }

    #[test]
    fn fixture_has_at_least_ten_items_with_distinct_truths() {
        assert!(
            FIXTURE.len() >= 10,
            "RT-005 requires a fixture of at least 10 items"
        );
        let mut truths: Vec<&str> = FIXTURE.iter().map(|i| i.truth).collect();
        truths.sort_unstable();
        truths.dedup();
        assert_eq!(
            truths.len(),
            FIXTURE.len(),
            "fixture truths must be distinct"
        );
    }

    #[test]
    fn fiable_returns_ground_truth_for_every_item() {
        let peer = PeerB::new(Reliability::Fiable, 42).expect("fiable peer builds");
        for item in FIXTURE {
            let got = peer.peer_b_solve(item.id, 1).expect("fixture id resolves");
            assert_eq!(
                got.answer(),
                item.truth,
                "item {} must be answered correctly",
                item.id
            );
        }
        assert!(
            peer.perturbed_ids().is_empty(),
            "fiable mode perturbs nothing"
        );
        assert_eq!(wrong_count(&peer), 0);
    }

    #[test]
    fn degradee_perturbs_exactly_three_of_ten() {
        let peer = PeerB::new(Reliability::Degradee, 42).expect("degraded peer builds");
        assert_eq!(perturbed_count(10), 3, "10 * 2 / 6 == 3");
        assert_eq!(peer.perturbed_ids().len(), 3);
        assert_eq!(wrong_count(&peer), 3);
    }

    #[test]
    fn degradee_perturbs_exactly_two_of_six() {
        let peer = PeerB::with_fixture(SIX_ITEM_FIXTURE, Reliability::Degradee, 42)
            .expect("degraded peer builds");
        assert_eq!(perturbed_count(6), 2, "6 * 2 / 6 == 2");
        assert_eq!(peer.perturbed_ids().len(), 2);
        assert_eq!(wrong_count(&peer), 2);
    }

    #[test]
    fn same_seed_same_answers_and_same_perturbed_set() {
        let a = PeerB::new(Reliability::Degradee, 7).expect("builds");
        let b = PeerB::new(Reliability::Degradee, 7).expect("builds");
        assert_eq!(a.perturbed_ids(), b.perturbed_ids());
        for item in FIXTURE {
            assert_eq!(
                a.peer_b_solve(item.id, 3).expect("resolves"),
                b.peer_b_solve(item.id, 3).expect("resolves"),
                "item {} must reproduce byte-identically",
                item.id
            );
        }
    }

    #[test]
    fn different_seed_selects_different_perturbed_set() {
        // Scan seeds rather than asserting on one: the requirement is that the
        // seed is load-bearing, not that any specific pair differs.
        let base = PeerB::new(Reliability::Degradee, 7).expect("builds");
        let differs = (0..64u64).filter(|s| *s != 7).any(|s| {
            PeerB::new(Reliability::Degradee, s)
                .expect("builds")
                .perturbed_ids()
                != base.perturbed_ids()
        });
        assert!(differs, "the seed must change which items are perturbed");
    }

    #[test]
    fn perturbed_answers_are_never_ground_truth() {
        for seed in 0..32u64 {
            let peer = PeerB::new(Reliability::Degradee, seed).expect("builds");
            for id in peer.perturbed_ids() {
                let truth = peer
                    .ground_truth(id)
                    .expect("perturbed id is in the fixture");
                let got = peer.peer_b_solve(id, 1).expect("resolves");
                assert_ne!(
                    got.answer(),
                    truth,
                    "seed {seed}: item {id} must be answered wrongly"
                );
            }
        }
    }

    #[test]
    fn perturbed_answers_are_well_formed_fixture_answers() {
        // The knob must not be detectable by output shape — a perturbed answer
        // is another item's real answer, never a corrupted string.
        for seed in 0..32u64 {
            let peer = PeerB::new(Reliability::Degradee, seed).expect("builds");
            for id in peer.perturbed_ids() {
                let got = peer.peer_b_solve(id, 1).expect("resolves");
                assert!(
                    FIXTURE.iter().any(|i| i.truth == got.answer()),
                    "seed {seed}: item {id} answer '{}' is not a well-formed fixture answer",
                    got.answer()
                );
            }
        }
    }

    #[test]
    fn unknown_item_id_is_an_error_not_a_panic() {
        let peer = PeerB::new(Reliability::Fiable, 1).expect("builds");
        let err = peer
            .peer_b_solve("no-such-item", 1)
            .expect_err("unknown id must error");
        assert!(err.to_string().contains("no-such-item"));
    }

    #[test]
    fn k_greater_than_one_returns_distinct_candidates_head_first() {
        let peer = PeerB::new(Reliability::Fiable, 3).expect("builds");
        let got = peer.peer_b_solve("rt005-04", 4).expect("resolves");
        assert_eq!(got.candidates.len(), 4);
        assert_eq!(got.answer(), "391", "the committed answer leads");
        let mut seen = got.candidates.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 4, "candidates must be distinct");

        // k = 0 is clamped to the committed answer alone.
        let clamped = peer.peer_b_solve("rt005-04", 0).expect("resolves");
        assert_eq!(clamped.candidates.len(), 1);
    }

    #[test]
    fn call_order_does_not_change_answers() {
        let peer = PeerB::new(Reliability::Degradee, 11).expect("builds");
        let forward: Vec<_> = FIXTURE
            .iter()
            .map(|i| peer.peer_b_solve(i.id, 2).expect("resolves"))
            .collect();
        let mut backward: Vec<_> = FIXTURE
            .iter()
            .rev()
            .map(|i| peer.peer_b_solve(i.id, 2).expect("resolves"))
            .collect();
        backward.reverse();
        assert_eq!(forward, backward);
    }

    #[test]
    fn reliability_labels_round_trip() {
        assert_eq!(
            Reliability::from_label("fiable").unwrap(),
            Reliability::Fiable
        );
        assert_eq!(
            Reliability::from_label("  DEGRADEE ").unwrap(),
            Reliability::Degradee
        );
        assert_eq!(
            Reliability::from_label("dégradée").unwrap(),
            Reliability::Degradee
        );
        assert_eq!(Reliability::Degradee.label(), "degradee");
        assert!(Reliability::from_label("mostly").is_err());
    }

    #[test]
    fn empty_fixture_is_rejected() {
        assert!(PeerB::with_fixture(&[], Reliability::Fiable, 1).is_err());
    }

    #[test]
    fn fixture_too_small_to_degrade_is_rejected() {
        // Under the 2/6 rate a 2-item fixture yields zero perturbations. A
        // degraded arm that degrades nothing must fail loudly, not answer
        // everything correctly while claiming to be degraded.
        const TWO: &[Item] = &[
            Item {
                id: "two-1",
                prompt: "1+1",
                truth: "2",
            },
            Item {
                id: "two-2",
                prompt: "2+2",
                truth: "4",
            },
        ];
        assert_eq!(perturbed_count(2), 0);
        assert!(PeerB::with_fixture(TWO, Reliability::Degradee, 1).is_err());
        // The same fixture is legitimate under Fiable — nothing needs perturbing.
        assert!(PeerB::with_fixture(TWO, Reliability::Fiable, 1).is_ok());
        // Three items is the smallest fixture the rate can degrade.
        assert_eq!(perturbed_count(3), 1);
    }

    #[test]
    fn distractors_never_hand_back_a_perturbed_item_truth() {
        // At k > 1 the candidate list must not contain the correct answer for
        // an item the knob perturbed, or a consumer could recover it and the
        // manipulation collapses.
        for seed in 0..32u64 {
            let peer = PeerB::new(Reliability::Degradee, seed).expect("builds");
            for id in peer.perturbed_ids() {
                let truth = peer
                    .ground_truth(id)
                    .expect("perturbed id is in the fixture");
                let got = peer.peer_b_solve(id, FIXTURE.len()).expect("resolves");
                assert!(
                    !got.candidates.iter().any(|c| c == truth),
                    "seed {seed}: item {id} leaked its ground truth '{truth}' as a distractor"
                );
            }
        }
    }

    #[test]
    fn distractors_do_include_truth_for_a_correctly_answered_item() {
        // The withholding is scoped to perturbed items only — it must not
        // quietly shrink the candidate pool everywhere.
        let peer = PeerB::new(Reliability::Fiable, 5).expect("builds");
        let got = peer
            .peer_b_solve("rt005-01", FIXTURE.len())
            .expect("resolves");
        assert_eq!(got.candidates.len(), FIXTURE.len());
    }
}
