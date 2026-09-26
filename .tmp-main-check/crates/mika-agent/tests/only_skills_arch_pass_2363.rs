//! The trap mika#2363 must not fall into, pinned (B3 / V6 / AC8).
//!
//! A mika-arch turn now declares the pass it is running, and the server keeps only
//! that skill. The obvious "simplification" that follows is to flip the three
//! `always_on = true` flags to `false` and let the declaration do the selecting.
//! **That would silently unarm every architect output contract**, and no existing
//! assertion would notice.
//!
//! Why the keyword route cannot replace the declaration (T2b): `mika-arch-groom-ticket`
//! declares the keyword `plan review` and `mika-arch-second-review` declares
//! `second pass`. Both turns of phrase occur naturally in the body of a plan — the
//! plan is the *message*, so routing on it would load the wrong skill, or both.
//! The content under review cannot select the skill that reviews it.
//!
//! Why the declaration alone cannot replace `always_on` either: `--only-skill` is
//! **strictly subtractive** (mika#2363 R2). It evicts the skills the caller did not
//! name; it never activates one. With `always_on = false` and no keyword hit, the
//! named skill is simply not active, and the turn carries **no** architect prompt:
//! no `required_suffix_lines`, no `required_finding_list_prefixes`, no
//! `required_review_anchor_prefixes` (mika#2037). The groom keeps running, the model
//! keeps answering, and `_parse_disposition` returns UNPARSED on every pass.
//!
//! A behavioural test cannot catch this class. The regression makes no decision
//! *wrong* — it makes the turn mute of contract, while every assertion about
//! parsing, guards and dispositions stays green because none of them is reached.

use std::path::{Path, PathBuf};

/// The three architect passes, exactly as `MIKA_ARCH_SKILL_ALLOWLIST`
/// (`crates/mika-agent/src/well_known_agents.rs`) lists them. Any turn runs one.
const ARCH_PASSES: &[&str] = &[
    "mika-arch-groom-ticket",
    "mika-arch-second-review",
    "mika-arch-groom-milestone",
];

fn bundled(skill: &str, file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../skills/bundled")
        .join(skill)
        .join(file)
}

fn read(skill: &str, file: &str) -> String {
    let path = bundled(skill, file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

#[test]
fn every_arch_pass_stays_always_on() {
    for skill in ARCH_PASSES {
        let toml = read(skill, "skill.toml");
        assert!(
            toml.contains("always_on = true"),
            "{skill} is no longer always_on.\n\
             \n\
             mika#2363 made a mika-arch turn declare its pass through `--only-skill`, but that \
             channel is STRICTLY SUBTRACTIVE: it evicts the skills the caller did not name and \
             can never activate one. Keyword matching cannot stand in either — this skill's \
             triggers are phrases that occur in the body of the plan under review, so routing \
             on the message would load the wrong pass or both (mika#2363 T2b).\n\
             \n\
             With this flag off, an architect turn carries NO skill prompt at all: no \
             required_suffix_lines (#864), no required_finding_list_prefixes (#901), no \
             review-anchor contract (mika#2037). Nothing fails loudly — `_parse_disposition` \
             just returns UNPARSED on every groom.\n\
             \n\
             If the additive half of the channel ever ships (mika#1727's other half, deferred \
             in mika#2363 B1.2 with its reason), revisit this pin. Until then it is load-bearing."
        );
    }
}

#[test]
fn declaring_one_pass_removes_the_other_two_prompts() {
    // AC2's arithmetic, checked against the tree rather than asserted in prose.
    // The saving is the sum of the two prompts the turn is not running; the
    // threshold is the ticket's 20 000 bytes.
    let sizes: Vec<(&str, usize)> = ARCH_PASSES
        .iter()
        .map(|s| (*s, read(s, "system_prompt.md").len()))
        .collect();
    let total: usize = sizes.iter().map(|(_, n)| n).sum();

    for (skill, own) in &sizes {
        let saved = total - own;
        assert!(
            saved >= 20_000,
            "declaring {skill} would drop only {saved} bytes of sister-pass prompt \
             (total {total}, own {own}); mika#2363 AC2 requires at least 20 000. \
             Either a sister prompt shrank a lot, or a pass was removed from the set."
        );
    }
}

#[test]
fn arch_ask_declares_exactly_the_pass_it_runs() {
    // V7's Rust-side half. The shell half (`skills/bundled/_shared/test-dispatch-lib.sh`)
    // exercises `_arch_ask`'s argv for each of the three passes; this one pins the
    // two properties that a passing argv test would not distinguish from a
    // hard-coded list.
    let lib = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/bundled/_shared/dispatch-lib.sh"),
    )
    .expect("reading dispatch-lib.sh");

    assert!(
        lib.contains(r#"--only-skill "$skill""#),
        "_arch_ask must declare its pass; without it mika-arch is back to three prompts per turn"
    );
    assert!(
        !lib.contains(r#"--enable-skill "$skill""#),
        "--enable-skill and --only-skill are mutually exclusive in clap, so keeping both would \
         make every architect call die at argument parsing. Dropping --enable-skill costs \
         nothing: it has been abandoned in transit since mika#1727."
    );
    // The pass vocabulary must not be rebuilt inside the helper. Its *callers*
    // legitimately name the pass they want — that is the declaration — but
    // `_arch_ask` itself receives it as `$1` and passes it through. A helper that
    // enumerated its sisters would hold a second copy of
    // MIKA_ARCH_SKILL_ALLOWLIST, in shell, free to drift.
    let body = arch_ask_body(&lib);
    for skill in ARCH_PASSES {
        let occurrences = body
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter(|l| l.contains(skill))
            .count();
        assert_eq!(
            occurrences, 0,
            "_arch_ask's body names {skill} literally; it must forward $1 instead"
        );
    }
}

/// Restricting the registry also removes the evicted skills' **tools**, and one
/// tool is load-bearing for every pass.
///
/// All three arch skills declare `required_fetches_for_quoted_resources = true`
/// (mika#863): the architect must fetch each resource the plan cites, and the
/// required-tools gate refuses the EndTurn otherwise. The fetching is done with
/// `gh_read`. Today each pass declares that tool itself, so declaring one pass
/// keeps it — which is the only reason mika#2363 can evict the sisters safely.
///
/// A pass that stopped declaring `gh_read` would produce a turn that is required
/// to fetch and has nothing to fetch with: the gate would refuse, re-prompt, and
/// refuse again. Before mika#2363 the sister skills' tool would have covered it,
/// so this coupling was invisible.
#[test]
fn every_arch_pass_declares_the_tool_its_own_contract_requires() {
    for skill in ARCH_PASSES {
        let toml = read(skill, "skill.toml");
        if !toml.contains("required_fetches_for_quoted_resources = true") {
            continue;
        }
        let tools = read(skill, "tools.json");
        assert!(
            tools.contains("\"gh_read\""),
            "{skill} requires fetching every quoted resource but declares no gh_read.\n\
             \n\
             Since mika#2363 a declared pass is the ONLY skill on the turn, so it no longer \
             inherits its sisters' tools. A turn required to fetch with no fetching tool is \
             refused by the required-tools gate (#863) and cannot satisfy the re-prompt either."
        );
    }
}

/// AC4 — the channel is subtractive, and the server tree is where that could
/// stop being true.
///
/// `apply_only_skills` delegates to `apply_transient_disable` and nothing else;
/// its sibling `apply_transient_always_on` would let any authenticated caller of
/// `/a2a/{agent}` force a skill into an agent's prompt. Nothing in `src/server/`
/// may reach for it. A unit test on `apply_only_skills` covers today's
/// implementation; this covers the whole surface the protocol field is exposed
/// on, including handlers that do not exist yet.
#[test]
fn no_server_path_can_force_a_skill_always_on() {
    fn walk(dir: &Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("reading server tree") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let src = std::fs::read_to_string(&path).expect("reading a server source file");
                if src.contains("apply_transient_always_on") {
                    hits.push(path.display().to_string());
                }
            }
        }
    }

    let server = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server");
    let mut hits = Vec::new();
    walk(&server, &mut hits);

    assert!(
        hits.is_empty(),
        "mika#2363 AC4: the per-turn skill channel must only ever subtract, but these server \
         files reach for the additive override: {hits:?}. A caller of /a2a/{{agent}} would be \
         able to force a skill into the agent's prompt. If the additive half is genuinely being \
         shipped, it needs its own threat model — not an allowlist entry here."
    );
}

/// Slice `_arch_ask`'s body out of the library source.
///
/// A plain `contains` over the whole file cannot answer this question: the
/// helper's three callers name their pass on purpose, and that is the correct
/// shape.
fn arch_ask_body(lib: &str) -> String {
    let start = lib
        .find("\n_arch_ask() {")
        .expect("_arch_ask must exist in dispatch-lib.sh");
    let rest = &lib[start + 1..];
    let end = rest
        .find("\n}\n")
        .expect("_arch_ask must be closed at column zero");
    rest[..end].to_string()
}
