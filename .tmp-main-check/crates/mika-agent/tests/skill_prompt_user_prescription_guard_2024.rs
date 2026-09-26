//! mika#2024 R6 — a skill prompt must not prescribe to the **user** a gesture the
//! runtime makes impossible.
//!
//! ## Why a source scan and not a behavioural test
//!
//! The regression this guards against is *rewriting the sentence in the prompt*. No
//! assertion on `run_gws` would redden: the handler would keep appending exactly the
//! remediation it is asked for, while the prompt started speaking again over it — and
//! `google-workspace` is `always_on`, so the prompt reaches the model on every turn,
//! with or without a failing tool call. The defect would not make a decision wrong;
//! it would make one inoperative. Same reasoning, and the same shape, as
//! `grooming_marker::tests::no_grooming_regex_outside_this_module` (mika#2158) and
//! `dispatcher::tests::mika2205_periodic_scans_do_not_read_the_pat_field_directly`.
//!
//! For `browser-control` this is not merely a backstop, it is the **only**
//! enforcement: that capability comes from an MCP server, not from a Rust builtin,
//! so there is no handler to condition (plan § R5, § Fire-Disposition).
//!
//! ## The predicate discriminates the ADDRESSEE, not the vocabulary
//!
//! The agent has a shell; the user of a cloud tenant does not. So "mentions a
//! command" is the wrong predicate — it would cry on three healthy prompts
//! (`mcp:49` « Run `mika mcp list` to verify », `github:41` « Run `label list` once
//! per conversation », `shell-exec:59` « Run `shellcheck <script>` »), all addressed
//! to the agent, and a guard that cries on healthy prompts is disarmed within the
//! week. Those three are **not** allowlist entries: they are outside the predicate
//! by construction, and they appear below as negative controls precisely so that
//! stays true.
//!
//! Three shapes, because the two measured violations were written two different
//! ways and a third was plausible enough to be worth covering:
//!
//! - **A — inline**: a user address and a gesture verb on one line
//!   (« Ask the user to run … », « tell the user to restart … »).
//! - **B — address then block**: an address line ending in a colon, with the gesture
//!   in the quoted block that follows (the `browser-control` install block).
//! - **C — impersonal or second-person**: « suggest running … », « from your
//!   terminal » — the founding symptom verbatim.
//!
//! ## Allowlist: none, and the vacuity is the invariant
//!
//! At land time the only lines this predicate would have caught are exactly the ones
//! mika#2024 removes. An allowlist entry would legitimize a surviving user
//! prescription — the very class this ticket closes — and would outlive the follow-up
//! meant to retire it. A new positive is a **halt**: read the occurrence, decide
//! whether it belongs to this PR or to a follow-up, and do not soften the predicate
//! to make it green (that removes the one capability the guard is shipped for).

use std::path::{Path, PathBuf};

/// Perimeter, pinned rather than merely exercised (plan § Fire-Disposition).
///
/// The guard does not count itself even though this file literally contains the
/// proscribed strings — because it enumerates `system_prompt.md` under two prompt
/// trees, not `src/` as mika#2361's T7 did, which is why that one had to exclude
/// itself. **That is a property of the perimeter, not a property of the predicate.**
/// Widening this scan to `src/` or `tests/` would silently redden the guard on its
/// own fixtures, so the perimeter is asserted here and not just its verdict.
const SCANNED_TREES: &[&str] = &["crates/mika-agent/templates/skills", "skills/bundled"];

/// Resolve the workspace root from the crate dir (`crates/mika-agent/`).
fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

/// Every `<tree>/<skill>/system_prompt.md`, i.e. the glob the constant names.
fn system_prompts_under(tree: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(tree) else {
        return found;
    };
    for entry in entries.flatten() {
        let prompt = entry.path().join("system_prompt.md");
        if prompt.is_file() {
            found.push(prompt);
        }
    }
    found.sort();
    found
}

fn all_scanned_prompts() -> Vec<PathBuf> {
    let root = workspace_root();
    SCANNED_TREES
        .iter()
        .flat_map(|tree| system_prompts_under(&root.join(tree)))
        .collect()
}

/// A phrase that makes the **user** the addressee of what follows.
const USER_ADDRESS: &[&str] = &[
    "tell the user",
    "ask the user",
    "advise the user",
    "instruct the user",
    "the user to",
    "the user should",
    "have the user",
];

/// A gesture that needs a shell, a host, or control of the service — i.e. exactly
/// what a cloud tenant does not have. Deliberately narrow: verbs like "check",
/// "confirm" or "authenticate" are NOT here, because they describe things a user can
/// plausibly do from their own surface, and widening to them is how this predicate
/// would start crying on healthy prompts.
const HOST_GESTURE: &[&str] = &[
    "run",
    "restart",
    "execute",
    "install",
    "reinstall",
    "launch",
];

/// Second-person or impersonal remediation shapes that need no explicit address.
/// `from your terminal` is the founding symptom, verbatim.
const DIRECT_PRESCRIPTION: &[&str] = &[
    "your terminal",
    "your shell",
    "your command line",
    "from a terminal",
    "open a terminal",
    "suggest running",
    "suggest restarting",
    "suggest that they run",
];

/// How many lines after a colon-terminated address still count as the block that
/// address introduces. Ten covers the measured `browser-control` shape (address,
/// blank line, a five-line blockquote, the trailing "Then restart Mika.").
const ADDRESS_BLOCK_WINDOW: usize = 10;

/// A prohibition standing immediately before a user address inverts its meaning:
/// « do not tell the user how to install … » forbids the prescription, it does not
/// make one. Both halves of this fix's own replacement prose are written that way,
/// and so is any future prompt that learns the lesson — a predicate blind to
/// negation would cry loudest on the prompts that got it right.
const NEGATION: &[&str] = &["do not", "don't", "never", "no need to", "rather than"];

/// How far back a prohibition still governs the address. Bounded so that a « do
/// not » early in a long line cannot clear a genuine prescription at its end.
const NEGATION_LOOKBACK: usize = 40;

/// Clause terminators that end the scope of an address.
///
/// The prescription shape is tight — « ask the user **to run** … », « tell the user
/// **to restart** … » — the verb governed by the address sits in the same clause.
/// Beyond a comma or a full stop the sentence has usually changed addressee, which
/// is literally the `shell-exec:40` case: « Tell the user what you're creating,
/// **then execute immediately** » instructs the agent, not the user.
const CLAUSE_TERMINATORS: &[char] = &[',', '.', ';', '!', '?'];

/// Whether `needle` occurs in `haystack` as a whole word.
///
/// Word-bounded so that "run" does not match inside "running" (shape C owns
/// « suggest running ») nor inside "runtime" — the word this fix's own replacement
/// prose uses.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
        if from >= haystack.len() {
            break;
        }
    }
    false
}

/// Every user address on this line that is not governed by a prohibition, as the
/// byte offset just past the marker.
///
/// All occurrences, not just the first: one line can carry a prohibition and a
/// prescription, and keeping only the first would let either hide the other.
fn live_user_addresses(line: &str) -> Vec<usize> {
    let mut found = Vec::new();
    for marker in USER_ADDRESS {
        let mut from = 0;
        while let Some(offset) = line[from..].find(marker) {
            let start = from + offset;
            let lookback = line[start.saturating_sub(NEGATION_LOOKBACK)..start].to_string();
            if !NEGATION.iter().any(|n| lookback.contains(n)) {
                found.push(start + marker.len());
            }
            from = start + marker.len();
            if from >= line.len() {
                break;
            }
        }
    }
    found.sort_unstable();
    found.dedup();
    found
}

/// The clause an address governs: everything after it up to the first terminator.
fn governed_clause(line: &str, after_address: usize) -> &str {
    let rest = &line[after_address..];
    match rest.find(CLAUSE_TERMINATORS) {
        Some(end) => &rest[..end],
        None => rest,
    }
}

fn names_host_gesture(text: &str) -> Option<&'static str> {
    HOST_GESTURE
        .iter()
        .find(|verb| contains_word(text, verb))
        .copied()
}

/// One offending line, with the shape that caught it.
#[derive(Debug)]
struct Violation {
    path: String,
    line_number: usize,
    line: String,
    shape: &'static str,
}

fn scan(path: &Path, body: &str) -> Vec<Violation> {
    let lines: Vec<&str> = body.lines().collect();
    let lowered: Vec<String> = lines.iter().map(|l| l.to_ascii_lowercase()).collect();
    let mut violations = Vec::new();

    for (index, line) in lowered.iter().enumerate() {
        // Shape C — no address needed: the prescription is direct.
        if let Some(marker) = DIRECT_PRESCRIPTION.iter().find(|m| line.contains(**m)) {
            violations.push(Violation {
                path: path.display().to_string(),
                line_number: index + 1,
                line: lines[index].trim().to_string(),
                shape: match *marker {
                    "suggest running" | "suggest restarting" | "suggest that they run" => {
                        "C (impersonal remediation)"
                    }
                    _ => "C (second-person terminal)",
                },
            });
            continue;
        }

        let addresses = live_user_addresses(line);
        if addresses.is_empty() {
            continue;
        }

        // Shape A — the gesture sits in the clause the address governs. Scoping to
        // that clause (rather than to the rest of the line) is what keeps
        // `shell-exec:40` out: « Tell the user what you're creating, then execute
        // immediately » changes addressee at the comma.
        let inline_hit = addresses
            .iter()
            .any(|&at| names_host_gesture(governed_clause(line, at)).is_some());
        if inline_hit {
            violations.push(Violation {
                path: path.display().to_string(),
                line_number: index + 1,
                line: lines[index].trim().to_string(),
                shape: "A (inline user prescription)",
            });
            continue;
        }

        // Shape B — the address introduces a quoted block; the gesture is in it.
        if !line.trim_end().ends_with(':') {
            continue;
        }
        let window_end = (index + 1 + ADDRESS_BLOCK_WINDOW).min(lowered.len());
        for (offset, follower) in lowered[index + 1..window_end].iter().enumerate() {
            if names_host_gesture(follower).is_some() {
                violations.push(Violation {
                    path: path.display().to_string(),
                    line_number: index + 1 + offset + 1,
                    line: lines[index + 1 + offset].trim().to_string(),
                    shape: "B (gesture inside a block addressed to the user)",
                });
                break;
            }
        }
    }

    violations
}

// ---------------------------------------------------------------------------
// Non-vacuity — the guard must be shown to be looking at something.
// ---------------------------------------------------------------------------

#[test]
fn mika2024_the_scan_perimeter_is_the_two_prompt_trees_and_is_not_empty() {
    let root = workspace_root();
    for tree in SCANNED_TREES {
        let dir = root.join(tree);
        assert!(
            dir.is_dir(),
            "scanned tree {} does not exist — the perimeter moved and this guard is \
             now looking at nothing",
            dir.display()
        );
        assert!(
            !system_prompts_under(&dir).is_empty(),
            "no system_prompt.md under {} — a guard that scans an empty set is green \
             for the wrong reason",
            dir.display()
        );
    }

    let all = all_scanned_prompts();
    // Anchors: one prompt per tree that must be in the scanned set. If either
    // disappears, the perimeter changed and the assertion below must be revisited
    // rather than deleted.
    for anchor in [
        "templates/skills/google-workspace/system_prompt.md",
        "skills/bundled/qa-review/system_prompt.md",
    ] {
        assert!(
            all.iter().any(|p| p.to_string_lossy().ends_with(anchor)),
            "{anchor} is not in the scanned set ({} files) — perimeter drift",
            all.len()
        );
    }
}

// ---------------------------------------------------------------------------
// The guard itself.
// ---------------------------------------------------------------------------

#[test]
fn mika2024_no_skill_prompt_prescribes_a_host_gesture_to_the_user() {
    let mut violations = Vec::new();
    for path in all_scanned_prompts() {
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
        violations.extend(scan(&path, &body));
    }

    assert!(
        violations.is_empty(),
        "{} skill prompt line(s) prescribe to the USER a gesture the runtime may make \
         impossible (mika#2024):\n{}\n\n\
         A cloud or family tenant has no terminal and cannot restart the service. \
         Remedy: remove the gesture from the prompt and pose the remediation at the \
         tool result, conditioned on `(ctx.deployment, ctx.tier)` — see \
         `builtin_handlers::gws_auth_remediation`. If the capability has no Rust \
         handler to condition (the `browser-control` case), describe the \
         unavailability without naming a gesture.\n\n\
         Do NOT soften this predicate and do NOT add an allowlist entry: an \
         allowlist would legitimize exactly the class this guard exists to refuse, \
         and a softened predicate is a green guard with the capability removed.",
        violations.len(),
        violations
            .iter()
            .map(|v| format!("  - {}:{} [{}]  {}", v.path, v.line_number, v.shape, v.line))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---------------------------------------------------------------------------
// Negative controls — the three legitimate occurrences of plan § M4. These are
// NOT exemptions: the predicate must not see them at all, because their addressee
// is the agent, which does have a shell.
// ---------------------------------------------------------------------------

#[test]
fn mika2024_agent_addressed_imperatives_are_outside_the_predicate() {
    let cases = [
        (
            "mcp:49",
            "- Headers not working? Run `mika mcp list` to verify header keys are shown.",
        ),
        (
            "github:41",
            "**Before applying a label** with `issue edit --add-label`, verify the label \
             exists. Run `label list` once per conversation to check.",
        ),
        (
            "shell-exec:59",
            "- Run `shellcheck <script>` after writing to catch common issues (if \
             shellcheck is available).",
        ),
    ];
    for (name, line) in cases {
        let found = scan(Path::new("negative-control"), line);
        assert!(
            found.is_empty(),
            "the predicate fired on {name}, whose addressee is the agent: {found:?}. \
             A guard that cries on healthy prompts gets disarmed within the week — \
             repair the predicate, do not exempt the file."
        );
    }
}

/// Two false-positive classes the first draft of this predicate actually produced,
/// pinned so the repair cannot regress in silence.
///
/// Both were found by running the guard, not by reasoning about it — which is the
/// argument for keeping them here as fixtures rather than as a comment.
#[test]
fn mika2024_negation_and_agent_addressed_clauses_are_outside_the_predicate() {
    let cases = [
        // (1) A prohibition inverts the address. Ironically this is how every
        // prompt that has learnt the lesson is written, so a negation-blind
        // predicate cries loudest on the healthiest files.
        (
            "prohibition / install",
            "If no browser tools are listed, do not tell the user how to install or \
             start anything.",
        ),
        (
            "prohibition / restart",
            "Say that browser automation is unavailable for now; do not ask the user \
             to restart anything.",
        ),
        (
            "prohibition / never",
            "Never tell the user to run a shell command to fix this.",
        ),
        // (2) `shell-exec:40` — the address governs its clause, and the imperative
        // after the comma is aimed at the agent.
        (
            "shell-exec:40",
            "- **Target does not exist:** Tell the user what you're creating, then \
             execute immediately.",
        ),
    ];
    for (name, line) in cases {
        let found = scan(Path::new("negative-control"), line);
        assert!(
            found.is_empty(),
            "the predicate fired on {name}, which prescribes nothing to the user: \
             {found:?}"
        );
    }
}

/// The replacement prose this PR writes must itself stay outside the predicate —
/// otherwise the fix and the guard contradict each other on the day they land.
#[test]
fn mika2024_the_replacement_prose_is_outside_the_predicate() {
    let cases = [
        "- 2: Authentication error — the stored Google credentials are expired or \
         invalid. The tool result appends the remediation that applies to this \
         runtime; relay that one.",
        "- **Browser crash or MCP disconnection** — the MCP server only reconnects \
         when this instance is restarted, which is an operator action and not \
         something the user can perform. Say that browser automation is unavailable \
         for now; do not ask the user to restart anything.",
    ];
    for line in cases {
        let found = scan(Path::new("replacement-prose"), line);
        assert!(
            found.is_empty(),
            "the predicate fires on prose mika#2024 itself wrote: {found:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Positive controls — the guard must still be able to fail. Without these, a
// predicate that matched nothing at all would pass every test above.
// ---------------------------------------------------------------------------

#[test]
fn mika2024_the_two_measured_violations_are_still_caught() {
    // Shape A + C — google-workspace, as it read before this fix.
    let gws = "- 2: Authentication error — credentials may be expired or invalid. Ask \
               the user to run `gws auth login` to re-authenticate.\n\
               - If `run_gws` reports an authentication error (exit code 2), tell the \
               user their credentials may be expired and suggest running `gws auth \
               login` to re-authenticate.";
    let found = scan(Path::new("pre-fix/google-workspace"), gws);
    assert_eq!(
        found.len(),
        2,
        "the predicate no longer catches the two measured google-workspace lines: \
         {found:?}"
    );

    // Shape B — browser-control's address-then-block form.
    let browser = "If no browser tools are listed in your available tools, tell the user:\n\
                   \n\
                   > Browser automation requires a Playwright MCP server. Run \
                   `get_documentation` with topic `browser-control` for setup \
                   instructions, or run:\n\
                   > ```\n\
                   > mika mcp add playwright --transport stdio --command npx --args -y \
                   @playwright/mcp\n\
                   > ```\n\
                   > Then restart Mika.";
    let found = scan(Path::new("pre-fix/browser-control"), browser);
    assert!(
        !found.is_empty(),
        "the predicate no longer catches the browser-control install block — the \
         address-then-block shape is half the measured population"
    );

    // Shape A — browser-control's restart line, found by this PR's own sweep and
    // absent from the plan's § M4 table.
    let restart = "- **Browser crash or MCP disconnection** — tell the user to restart \
                   Mika to reconnect the MCP server.";
    let found = scan(Path::new("pre-fix/browser-control-restart"), restart);
    assert_eq!(
        found.len(),
        1,
        "the predicate no longer catches « tell the user to restart Mika »: {found:?}"
    );

    // Shape C — the founding symptom, verbatim.
    let verbatim = "You can fix that by running `gws auth login` from your terminal.";
    let found = scan(Path::new("pre-fix/founding-symptom"), verbatim);
    assert_eq!(
        found.len(),
        1,
        "the predicate no longer catches the founding symptom verbatim: {found:?}"
    );
}
