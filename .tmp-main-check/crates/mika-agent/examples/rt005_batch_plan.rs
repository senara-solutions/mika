//! `rt005_batch_plan` — RT-005 physics pilot, brick 3/5 (mika#1890).
//!
//! The bash-to-`peer_b` bridge for `run-batch.sh`. It emits, as one JSON
//! document on stdout, **only what bash cannot compute**: the fixture, peer B's
//! committed answer per (reliability arm, item), and the realised perturbation.
//!
//! Everything else the orchestrator needs — run keys, the seeded permutation,
//! the design fingerprint, prompt assembly — stays in the script. Orchestration
//! logic in here would put the batch's control flow inside the product crate,
//! which is the one place mika#1890 was asked not to expand.
//!
//! # Why an example and not a subcommand
//!
//! mika#1887 put a CLI subcommand out of its own scope and named mika#1890 as
//! the owner of the orchestration surface. Cargo auto-discovers `examples/*.rs`,
//! so this file adds no `Cargo.toml` entry, no runtime surface and no API
//! surface, and it deletes with the rest of the RT-005 scaffold. It does become
//! a standing `--all-targets` build and lint target for as long as the scaffold
//! lives; that is the price of not reimplementing peer_b's answer selection in
//! bash, where the two copies could silently diverge.
//!
//! # There is deliberately no item filter
//!
//! `peer_b` draws its perturbed subset from the fixture it is handed, so
//! building it over a subset would change *which* items are perturbed and
//! therefore change the manipulation itself. The bridge always emits the whole
//! fixture at the pinned seed; selecting a few items for a manip-check is the
//! script's job, downstream of the answers.
//!
//! # The seed here is not the script's ordering seed
//!
//! [`PEER_B_SEED`] fixes *which* items the degraded arm answers wrongly. The
//! script's ordering seed only reshuffles the run order. Conflating them would
//! mean a reshuffle silently changed the content of the manipulation, so they
//! are separate values and both are recorded in the batch manifest.
//!
//! Usage: `cargo run --example rt005_batch_plan [-- --peer-seed <u64>]`

use anyhow::{Context, Result, bail};
use mika_agent::research::peer_b::{FIXTURE, PeerB, Reliability};
use serde_json::{Value, json};

/// Pinned construction seed for `peer_b`. The RT-005 Round Table ratification
/// date (2026-07-28), used as a stable arbitrary constant. Changing it changes
/// which items the degraded arm gets wrong, so it is part of the design
/// fingerprint and must not drift between a manip-check and its batch.
const PEER_B_SEED: u64 = 20_260_728;

/// The two reliability arms, in the order the manifest records them.
const ARMS: [Reliability; 2] = [Reliability::Fiable, Reliability::Degradee];

fn parse_peer_seed(args: &[String]) -> Result<u64> {
    let mut seed = PEER_B_SEED;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--peer-seed" => {
                let raw = args.get(i + 1).context("--peer-seed requires a value")?;
                seed = raw
                    .parse()
                    .with_context(|| format!("--peer-seed value '{raw}' is not a u64"))?;
                i += 2;
            }
            other => bail!("unknown argument '{other}' (expected only --peer-seed <u64>)"),
        }
    }
    Ok(seed)
}

/// Build the emitted document. Pure over the seed, so the tests below exercise
/// exactly what `main` prints.
fn build_document(peer_seed: u64) -> Result<Value> {
    let items: Vec<Value> = FIXTURE
        .iter()
        .map(|item| json!({ "id": item.id, "prompt": item.prompt, "truth": item.truth }))
        .collect();

    let mut answers = serde_json::Map::new();
    let mut perturbed = serde_json::Map::new();

    for arm in ARMS {
        let peer = PeerB::new(arm, peer_seed)
            .with_context(|| format!("building peer_b for arm '{}'", arm.label()))?;

        let mut per_item = serde_json::Map::new();
        for item in FIXTURE {
            // k = 1: the protocol reads peer B's committed answer only. Asking
            // for distractors would put alternatives the agent never sees into
            // the run record.
            let response = peer
                .peer_b_solve(item.id, 1)
                .with_context(|| format!("solving '{}' on arm '{}'", item.id, arm.label()))?;
            per_item.insert(item.id.to_string(), json!(response.answer()));
        }

        answers.insert(arm.label().to_string(), Value::Object(per_item));
        perturbed.insert(arm.label().to_string(), json!(peer.perturbed_ids()));
    }

    Ok(json!({
        "peer_b_seed": peer_seed,
        "items": items,
        "answers": answers,
        "perturbed_ids": perturbed,
    }))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let peer_seed = parse_peer_seed(&args)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&build_document(peer_seed)?)?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers_for(doc: &Value, arm: &str) -> serde_json::Map<String, Value> {
        doc["answers"][arm]
            .as_object()
            .expect("arm present")
            .clone()
    }

    fn perturbed_for(doc: &Value, arm: &str) -> Vec<String> {
        doc["perturbed_ids"][arm]
            .as_array()
            .expect("arm present")
            .iter()
            .map(|v| v.as_str().expect("id is a string").to_string())
            .collect()
    }

    /// `run-batch.sh` parses run ids positionally with `split(".")`, so an item
    /// id containing a dot would shift every field after it. The invariant lives
    /// in the fixture literal in `peer_b.rs`; this asserts the bash contract that
    /// depends on it, where a future fixture edit would trip over it.
    #[test]
    fn item_ids_carry_no_dot_because_the_orchestrator_splits_on_it() {
        for item in FIXTURE {
            assert!(
                !item.id.contains('.'),
                "item id '{}' contains a dot; run-batch.sh parses run ids with split(\".\")",
                item.id
            );
        }
    }

    #[test]
    fn document_carries_the_four_top_level_keys() {
        let doc = build_document(PEER_B_SEED).expect("builds");
        for key in ["peer_b_seed", "items", "answers", "perturbed_ids"] {
            assert!(doc.get(key).is_some(), "missing top-level key '{key}'");
        }
        assert_eq!(
            doc["items"].as_array().expect("items is an array").len(),
            FIXTURE.len()
        );
    }

    #[test]
    fn fiable_arm_answers_ground_truth_and_perturbs_nothing() {
        let doc = build_document(PEER_B_SEED).expect("builds");
        let answers = answers_for(&doc, "fiable");
        for item in FIXTURE {
            assert_eq!(
                answers[item.id],
                json!(item.truth),
                "item {} must carry ground truth on the fiable arm",
                item.id
            );
        }
        assert!(perturbed_for(&doc, "fiable").is_empty());
    }

    #[test]
    fn degradee_arm_differs_on_exactly_its_perturbed_ids() {
        let doc = build_document(PEER_B_SEED).expect("builds");
        let fiable = answers_for(&doc, "fiable");
        let degradee = answers_for(&doc, "degradee");
        let perturbed = perturbed_for(&doc, "degradee");

        // n * 2 / 6 over the ten-item fixture. This is the dilution the batch
        // manifest has to state: only three of ten items differ between arms.
        assert_eq!(perturbed.len(), 3, "10 * 2 / 6 == 3");

        let differing: Vec<String> = FIXTURE
            .iter()
            .filter(|item| fiable[item.id] != degradee[item.id])
            .map(|item| item.id.to_string())
            .collect();
        assert_eq!(
            differing, perturbed,
            "the arms must differ on exactly the realised perturbed set"
        );
    }

    #[test]
    fn peer_seed_is_the_only_thing_that_moves_the_manipulation() {
        let base = build_document(PEER_B_SEED).expect("builds");
        let same = build_document(PEER_B_SEED).expect("builds");
        assert_eq!(base, same, "same peer seed must reproduce byte-identically");

        // Scan rather than assert on one pair: the requirement is that the seed
        // is load-bearing, not that any specific alternative differs.
        let moved = (0..64u64).filter(|s| *s != PEER_B_SEED).any(|s| {
            let other = build_document(s).expect("builds");
            perturbed_for(&other, "degradee") != perturbed_for(&base, "degradee")
        });
        assert!(moved, "the peer seed must change which items are perturbed");
    }

    #[test]
    fn perturbed_answers_are_well_formed_and_never_the_truth() {
        for seed in 0..16u64 {
            let doc = build_document(seed).expect("builds");
            let degradee = answers_for(&doc, "degradee");
            for id in perturbed_for(&doc, "degradee") {
                let item = FIXTURE
                    .iter()
                    .find(|i| i.id == id)
                    .expect("perturbed id is in the fixture");
                let got = degradee[&id].as_str().expect("answer is a string");
                assert_ne!(
                    got, item.truth,
                    "seed {seed}: {id} must be answered wrongly"
                );
                assert!(
                    FIXTURE.iter().any(|i| i.truth == got),
                    "seed {seed}: {id} answer '{got}' is not a well-formed fixture answer"
                );
            }
        }
    }

    #[test]
    fn peer_seed_argument_parses_and_rejects_junk() {
        assert_eq!(parse_peer_seed(&[]).expect("default"), PEER_B_SEED);
        assert_eq!(
            parse_peer_seed(&["--peer-seed".into(), "7".into()]).expect("parses"),
            7
        );
        assert!(parse_peer_seed(&["--peer-seed".into()]).is_err());
        assert!(parse_peer_seed(&["--peer-seed".into(), "x".into()]).is_err());
        // No item filter exists by design — building peer_b over a subset would
        // change which items are perturbed.
        assert!(parse_peer_seed(&["--items".into(), "rt005-01".into()]).is_err());
    }
}
