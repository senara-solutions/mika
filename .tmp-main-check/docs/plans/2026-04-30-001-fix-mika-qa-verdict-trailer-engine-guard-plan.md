---
title: "fix(mika-qa): engine-side guard validates run_gh pr review --body for VERDICT trailer"
type: fix
status: draft
date: 2026-04-30
issue: 899
---

# fix(mika-qa): engine-side guard validates `run_gh pr review --body` for VERDICT trailer

## Phase 0 — pre-implementation pinned facts

**Pin 1 — qa-review skill manifest (verified at branch HEAD):**
```toml
# skills/bundled/qa-review/skill.toml
[constraints]
required_tools = ["qa_pr_view", "run_gh", "run_shell", "build_mika"]
# NO [output] section currently exists. Verdict trailer is documented only
# in the system_prompt — no manifest-level enforcement.
```

**Pin 2 — required_suffix_lines guard locations (mika#864):**
- `crates/mika-agent/src/skills/manifest.rs` — schema definition for `[output] required_suffix_lines`
- `crates/mika-agent/src/skills/index.rs` — `collect_required_suffix_lines()` aggregator
- `crates/mika-agent/src/agent.rs` — guard #8 in EndTurn post-condition chain. **mika#864 uses literal exhaustive token lists, NOT regex.** Same precedent applies here (per architect F1).

**Pin 3 — `run_gh` dispatch call-graph (architect F4 expansion):**

Implementer Phase 0 verification, BEFORE coding:

1. Run `grep -n 'run_gh\|RunGh\|gh_runner' crates/mika-agent/src/skills/executor.rs` to enumerate the dispatch entry-point function name and exact line range.
2. Run `grep -rn 'run_gh\|RunGh' crates/mika-agent/src/` to enumerate ALL callers of run_gh dispatch across the crate.
3. Confirm the chosen validation site is upstream of every entry point (single chokepoint, not duplicated per-caller).
4. Confirm the site has access to parsed argv (gh subcommand + `--body` value) BEFORE the subprocess `spawn`. Validation must run pre-spawn — post-spawn is too late, the review is already public.
5. Pin the function name + line range + per-caller traces in this section before any code change.

**Pin 4 — verdict literal list (architect F1, mirrors mika#864 token list precedent):**

```toml
[
  "VERDICT: pass",
  "VERDICT: hold[review]",
  "VERDICT: block[ac]",
  "VERDICT: block[ci]",
  "VERDICT: block[security]",
  "VERDICT: block[pipeline]",
]
```

Closed-alphabet, exhaustive, case-sensitive on the literal `VERDICT:` prefix to match mika#864's discipline. The matcher checks that one of these literals appears as a non-empty trailing line of the body argument (mirroring mika#864's "any of last 3 trimmed lines" semantics).

**Pin 5 — empirical recurrence evidence:**
PR mika#898 mika-qa session 2026-04-29T18:27:33Z, review COMMENTED. Body contained substantive review (DIFF ANALYSIS, PLAN-AC VERIFICATION, ...) but no `VERDICT:` line. mika#896's `verdict_classification_failed` path fired. mika-qa self-diagnosis (verbatim in mika#899 body) confirmed the trailer was composed in reasoning, omitted from `--body`.

**Pin 6 — other emitters of `run_gh pr review --body` (architect F7):**

Implementer Phase 0 verification:

```bash
grep -l "run_gh.*pr.*review\|gh pr review" mika/skills/bundled/*/system_prompt.md mika/skills/bundled/*/skill.toml
```

For each skill that emits verdicts via this transport, ship its `[output] required_tool_arg_suffixes` opt-in declaration **in this PR**. mika#864 shipped opt-ins for both mika-arch skills together; same precedent. If grep returns only qa-review, document the negative result and the Out of Scope holds.

## Why

mika-qa's verdict trailer is the canonical contract for mika#896's structural verdict handler. mika#864 added an EndTurn-text required-suffix-line guard catching missing trailers in **assistant text**. It does NOT cover **tool-call argument bodies** — the LLM can render the trailer correctly in EndTurn text but emit a `run_gh pr review --body "<truncated body>"` tool call where the body string lacks the trailer. GitHub accepts the review; the verdict handler can't parse it; auto-dispatch never fires.

This is a sibling to mika#864 at a different surface (tool-call arg vs EndTurn text). Same antipattern (missing required-suffix-line) applied to a different transport.

## Goal

Before mika-qa's `run_gh pr review` tool call dispatches, the engine validates that the `--body` argument has one of the literal VERDICT trailers from Pin 4 as a non-empty trailing line. Tool calls failing the check are rejected pre-spawn with a structured corrective message; the LLM retries with a properly-formatted body. mika#896's structural handler stays the single consumer of the verdict; this fix ensures the verdict actually arrives.

## Approach

Extend the manifest schema with a flat list (architect F2 — not a nested map) declaring per-tool-arg validation. Add engine pre-dispatch validation in `executor.rs` (Pin 3 site).

### Change 1 — Schema extension in `skills/manifest.rs` (architect F2)

```rust
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OutputConstraints {
    /// Existing field from mika#864 (EndTurn text)
    #[serde(default)]
    pub required_suffix_lines: Vec<String>,

    /// New (mika#899): per-tool-name argument-level required suffixes.
    /// Flat Vec for ordering/dedup determinism (per architect F2). Each entry
    /// declares: tool name, logical argument key (mapped to argv extraction by
    /// the engine), and a literal list of accepted trailer lines (one must
    /// appear as a non-empty trailing line of the matched argument).
    #[serde(default)]
    pub required_tool_arg_suffixes: Vec<RequiredToolArgSuffix>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RequiredToolArgSuffix {
    /// Tool name (e.g., "run_gh")
    pub tool: String,
    /// Logical argument key — must be in the engine's static
    /// LOGICAL_KEY_TO_EXTRACTOR table (loud-fail at manifest load on unknown).
    pub arg: String,
    /// Literal list of accepted trailer lines. Closed alphabet.
    pub required_lines: Vec<String>,
}
```

`Vec` not `HashMap`: deterministic iteration order for log/error messages, no surprise dedup; manifest can declare the same tool/arg twice (validated at load) for clarity if multiple skills opt in.

**Matcher implementation (per architect F10):** `str::contains` against each trimmed non-empty line of the body argument — NOT regex or glob — to preserve literal `[` / `]` character semantics in token names like `block[ac]`. Bracket characters in glob/regex contexts have meta-meaning ("any character in set"); fixed-string containment is the only safe matcher for the closed-alphabet token list.

### Change 2 — qa-review skill.toml opt-in (architect F1 literal list)

```toml
[[output.required_tool_arg_suffixes]]
tool = "run_gh"
arg = "pr_review_body"
required_lines = [
  "VERDICT: pass",
  "VERDICT: hold[review]",
  "VERDICT: block[ac]",
  "VERDICT: block[ci]",
  "VERDICT: block[security]",
  "VERDICT: block[pipeline]",
]
```

### Change 3 — Static logical-key → argv-extractor mapping in `executor.rs` (architect F3)

```rust
// crates/mika-agent/src/skills/executor.rs (location pinned in Phase 0 Pin 3)

/// Static mapping from logical argument keys (declared in skill manifests)
/// to argv-extraction functions. Skills' [output.required_tool_arg_suffixes]
/// entries reference these keys; unknown keys loud-fail at manifest validation.
const LOGICAL_KEY_TO_EXTRACTOR: &[(&str, fn(&[String]) -> Option<String>)] = &[
    ("pr_review_body", extract_pr_review_body),
    // Future entries: each new logical key adds one row + one extractor fn.
];

/// Extracts the --body value from `gh pr review <pr> --body "<value>"` argv.
/// Returns None if the gh subcommand is not `pr review` or `--body` is absent.
fn extract_pr_review_body(argv: &[String]) -> Option<String> {
    let mut iter = argv.iter().peekable();
    // Walk argv: confirm subcommand is "pr" then "review", then find "--body"
    // and return next element. Tolerant of argv ordering (--body could come
    // before or after the PR number).
    let mut saw_pr = false;
    let mut saw_review = false;
    while let Some(s) = iter.next() {
        if s == "pr" { saw_pr = true; continue; }
        if saw_pr && s == "review" { saw_review = true; continue; }
        if saw_review && s == "--body" {
            return iter.next().cloned();
        }
    }
    None
}
```

**Loud-fail at manifest validation (architect F3):** in `validate_skill()` (or the manifest-load path), iterate `output.required_tool_arg_suffixes` entries and assert each `arg` value is a key in `LOGICAL_KEY_TO_EXTRACTOR`. Unknown key → return a `SkillDiagnostic::Fail` with structured message naming the unknown key and listing valid keys. Skill load fails — does NOT silently no-op the validation.

**Maintenance discipline for `LOGICAL_KEY_TO_EXTRACTOR` (per architect F9):** Adding a new logical key requires simultaneous update to `LOGICAL_KEY_TO_EXTRACTOR` AND a corresponding unit test verifying extraction from a synthetic argv fixture (parallel to mika#864's "guards land with their fixture" discipline). The `validate_skill()` loud-fail is the runtime enforcement mechanism: any skill manifest declaring an unknown logical key surfaces immediately at load time, before the table diverges silently in production.

### Change 4 — Engine validation in `executor.rs` `run_gh` dispatch path

At the Phase-0-pinned dispatch site, BEFORE subprocess spawn:

1. Collect `output.required_tool_arg_suffixes` entries from active matched skills (keyword + always-on; mirrors mika#864's `collect_required_suffix_lines`). Filter to entries where `tool == "run_gh"`.
2. For each filtered entry, look up `extract_fn` in `LOGICAL_KEY_TO_EXTRACTOR` (already validated at manifest load — guaranteed present).
3. Run `extract_fn(argv)`. If it returns `None`, the gh subcommand doesn't match the entry's logical key — skip this entry.
4. If it returns `Some(value)`, scan the value's last N (= 3) trimmed non-empty lines for an exact match against any of `required_lines`. Match → pass-through to dispatch. No match → reject with structured error.

**Rejection shape (architect F8 — distinct names for distinct surfaces):**

- Engine structured-log key: `required_tool_arg_suffix_violation` (parallel to mika#864's `required_suffix_line_violation`).
- Skill-facing retry-prompt error variant: `verdict_trailer_missing` (domain-specific in retry context — qa-review's prompt instructions reference this term).

Single retry per turn via the existing intent-precondition retry tracker pattern (architect F5):

> New retry flag `required_tool_arg_suffix_retry_done` joins the existing chain in `agent.rs` per mika#864 / #870 / #862 pattern. Single retry per turn; on second failure, ESCALATE rather than infinite loop.

### Change 5 — Test fixtures

In `crates/mika-agent/tests/eval/grounding_regressions/` (matches mika#864 sibling pattern):

- `verdict_trailer_dropped_pre_fix.rs` — synthetic mika-qa session with body lacking VERDICT line. Assert pre-fix shape demonstrates the failure (tool call dispatched, verdict missing on GitHub side via mock). Frozen fixture.
- `verdict_trailer_dropped_caught.rs` — same input, post-fix. Assert tool call rejected with `required_tool_arg_suffix_violation` log event + `verdict_trailer_missing` retry-prompt error before dispatch.
- `verdict_trailer_present_passes.rs` — body containing valid `VERDICT: block[ac]` as last non-empty line. Assert tool call dispatches normally.
- `verdict_trailer_unconstrained_skill.rs` — non-qa-review skill calling `run_gh pr review`. Assert no validation fires (only opted-in skills enforce).
- `verdict_trailer_manifest_unknown_key.rs` — synthetic skill manifest declares `arg = "nonexistent_logical_key"`. Assert `validate_skill()` returns `SkillDiagnostic::Fail` with the unknown-key error.

## Critical files

| Purpose | Path |
|---|---|
| Schema extension | `crates/mika-agent/src/skills/manifest.rs` |
| Engine validation + extractor table | `crates/mika-agent/src/skills/executor.rs` (line range pinned in Phase 0 Pin 3) |
| Manifest validation (loud-fail) | `crates/mika-agent/src/skills/index.rs` (`validate_skill()`) |
| qa-review opt-in | `skills/bundled/qa-review/skill.toml` |
| Test fixtures (new, 5 files) | `crates/mika-agent/tests/eval/grounding_regressions/verdict_trailer_*.rs` |
| Reference pattern | `crates/mika-agent/src/agent.rs` (mika#864 EndTurn guard, retry-tracker chain) |

## Out of Scope

- mika#864 itself (already covers EndTurn text — different surface).
- mika-qa skill prompt edits in `skills/bundled/qa-review/system_prompt.md` (per `feedback_prompt_enforcement_fragile` and architect F6 — defense-in-depth not needed; engine-layer enforcement sufficient).
- Other tool-call argument validations beyond the entries declared in this PR (extensibility designed in but only `pr_review_body` ships).
- Producer-side redesign of mika-qa's verdict format.
- mika#894 (asserted-unavailability) and mika#901 (mika-arch-groom-ticket findings emission) — sibling family but separate fixes.

## Acceptance Criteria

- [x] R0 (Phase 0 gate): All 6 pinned facts (Pin 1-6 above) verified before commit. Implementer halts and surfaces to operator on any disagreement. Pin 3 specifically requires function name + line range + call-graph entry points + pre-spawn confirmation + parsed-argv-access confirmation.
- [x] R1: `RequiredToolArgSuffix` struct in `skills/manifest.rs`, flat Vec field on `OutputConstraints` with serde default for backward compat.
- [x] R2: qa-review `skill.toml` declares the literal-list opt-in for `run_gh / pr_review_body` per Pin 4.
- [x] R3: `LOGICAL_KEY_TO_EXTRACTOR` static table in `executor.rs` with `pr_review_body` entry. `extract_pr_review_body()` correctly extracts the `--body` value from `gh pr review` argv (test cases for ordering, missing `--body`, wrong subcommand).
- [x] R4: Manifest validation in `validate_skill()` loud-fails on unknown logical keys with `SkillDiagnostic::Fail` and a structured message listing valid keys.
- [x] R5: Engine validation rejects `run_gh pr review --body` calls with body argument lacking any of the declared `required_lines` as a trailing non-empty line. Rejection emits `required_tool_arg_suffix_violation` structured log + `verdict_trailer_missing` retry-prompt error. Single retry per turn via `required_tool_arg_suffix_retry_done` flag joining the agent.rs retry-tracker chain.
- [x] R6: PR mika#898's mika-qa-side regression (the originating reproduction) does not recur on a fresh review of the merged PR or any other PR.
- [x] R7: Skills NOT declaring `required_tool_arg_suffixes` are unaffected — no new gate fires on their tool calls.
- [x] R8: Five fixture tests (Change 5) all pass.
- [x] R9: Existing mika#864 EndTurn guard tests still pass — this fix is additive, not modificative.

## Verification

1. **Unit tests:** `cargo test -p mika-agent skills::executor verdict_trailer` covers all 5 fixtures.
2. **Static checks:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings` pass.
3. **End-to-end smoke (post-deploy):** synthesize a mika-qa session reviewing a synthetic PR. Body lacking VERDICT line → tool call rejected with `verdict_trailer_missing`; LLM emits corrected body → tool call dispatches. Body with VERDICT line → tool call dispatches first try.
4. **Regression check (mika#864):** existing EndTurn-text guard tests still pass — this fix doesn't touch that surface.
5. **Manifest-load fail-loud check:** synthetic skill with unknown logical key → `cargo test` shows `validate_skill` returning `Fail` with the unknown-key error.

## Cross-references

- mika#864 (MERGED 2026-04-29) — EndTurn-text required-suffix-line guard. **Direct sibling**: literal-list approach, retry-tracker chain pattern, `[output]` schema location, structured-log naming.
- mika#883 — engine implementation of #864.
- mika#870 / #862 — retry-tracker chain pattern citations.
- mika#894 — asserted-unavailability sibling. Same antipattern family, different evasion vector.
- mika#896 (= mika#889, MERGED 2026-04-29) — structural verdict handler. Consumer of the verdict this fix protects.
- mika#901 — mika-arch-groom-ticket emit-findings-verbatim. Sibling structural enforcement.
- PR mika#898 — canonical reproduction.

## Sequencing & Risk

- **Risk: pre-spawn site mis-pinned in Phase 0.** Mitigated by R0 gate; implementer halts if Pin 3 verification doesn't match plan assumptions.
- **Risk: `extract_pr_review_body` argv-parsing fragility.** Mitigated by deterministic test cases (ordering variants, missing `--body`, subcommand variations).
- **Risk: false positives on whitespace.** Trailing-line check uses `trim()` matching mika#864's "any of last 3 trimmed lines" semantics. Closed alphabet on the token strings themselves.
- **Risk: scope creep into other tool-call validations.** Plan ships only `run_gh / pr_review_body` (per Pin 6 grep result). Future entries land per-skill via manifest declaration; no engine code changes required.
- **Sequencing:** Independent of mika#894, #901. Engine-side change in `crates/mika-agent/`. Can dispatch immediately after grooming.

## Grooming history

- /ce:plan (operator-drafted, well-specified ticket body) → mika-arch first-pass review (session — pinned at pass-2 submission) — ITERATE with 4 BLOCKERS + 4 sharpenings:
  - F1: switched regex → literal list per mika#864 sibling precedent (Pin 4)
  - F2: switched schema HashMap → flat Vec<RequiredToolArgSuffix> (Change 1)
  - F3: authored static LOGICAL_KEY_TO_EXTRACTOR table location + loud-fail manifest validation (Change 3)
  - F4: expanded Phase 0 Pin 3 to function name + line range + call-graph entry points + pre-spawn confirmation + parsed-argv-access confirmation
  - F5 sharpening: retry-tracker citation applied (Change 4)
  - F6 sharpening: prompt-level Out of Scope confirmed
  - F7 sharpening: Pin 6 added for other-skill grep
  - F8 sharpening: distinct error names — engine `required_tool_arg_suffix_violation`, skill-facing `verdict_trailer_missing`
- → revisions applied → mika-arch second-pass (pending GROOMED).
