# Plan — fix(verdict): parser tolerates non-canonical mika-qa verdict shapes (mika#1828)

## Context

`server::verdict_handler` intercepts `pull_request_review.submitted` webhooks **before**
the LLM turn and routes on the `VERDICT:` line parsed by
`crates/mika-agent/src/server/verdict.rs::parse_verdict`. Any review body whose
`VERDICT:` line deviates from the exact canonical shape returns `Verdict::Missing`,
which the handler maps to `hold[review]` semantics: notify operator, **no auto-dispatch**.
This silently strands the review→revise→merge autonomy cycle.

**Founding evidence (2026-07-24, PR mika#1821).** `mika-platform-qa` submitted a
`CHANGES_REQUESTED` review whose body began:

```
## QA Review — mika#1821

**Verdict: REQUEST CHANGES** — one blocking finding (incomplete wiring in mika_qa.rs).
```

Two independent format deviations, either of which alone drops the review:

1. **Markdown-bold wrapper** — `**Verdict: ...**`. `VERDICT_RE` (`(?mi)^VERDICT:\s*(.+)$`)
   anchors `VERDICT` at line start; the leading `**` fails the match outright.
2. **Non-canonical class token** — `REQUEST CHANGES` is not one of
   `pass | block[ac] | block[ci] | block[security] | block[pipeline] | hold[review]`.
   Even with the `**` stripped, `parse_verdict` would fall through to `Verdict::Missing`.

Result: `handle_missing_verdict` fires, `verdict_classification_failed` is logged, the
task stays `in_progress`, and the PR sits at `REVIEW_REQUIRED` indefinitely with no
`dev-revise` dispatch.

**Priority: p1-important — loop substrate.** Every mika-qa review that drifts from the
canonical format costs one operator-manual unblock. Measured drift rate on the
routing-fixed loop = 100% (1 of 1 review we have from mika-qa).

### Current parser shape (`verdict.rs`)

- `VERDICT_RE = (?mi)^VERDICT:\s*(.+)$` — captures the post-colon remainder of the first
  line whose start is literally `VERDICT`.
- `parse_verdict` trims the captured value, matches `eq_ignore_ascii_case("pass")` →
  `Verdict::Pass`, then `BLOCK_RE`/`HOLD_RE` (`^block[...]$` / `^hold[...]$`,
  case-insensitive) → `Verdict::Block(_)`/`Verdict::Hold(_)`. Anything else →
  `Verdict::Missing { truncated }`.

### Surfaces the ACs touch

| AC | Surface | Nature |
|----|---------|--------|
| AC1 | `verdict.rs` — regex/prefix handling | markdown-bold tolerance |
| AC2 | `verdict.rs` — alias mapping | GitHub-review-state aliases → canonical |
| AC3 | `verdict.rs` `mod tests` | unit coverage for both new shapes |
| AC4 | `calibration/roles/mika_qa.rs` + fixture + `manifest.yaml` | new drift-prevention scenario |
| AC5 | `qa-review/skill.toml` | **already satisfied** — verify + justify (see Decision D3) |
| AC6 | operator / post-deploy | out of code scope — record retrigger procedure |

## Goal / Non-goals

**Goal.** Make `parse_verdict` tolerant of the two observed non-canonical shapes
(markdown-bold wrapper + GitHub-review-state alias tokens) so drifted-but-unambiguous
mika-qa reviews route correctly instead of silently degrading to `hold[review]`; add a
calibration scenario that catches the *format* drift at model-swap time; confirm the
engine-side output guard already enumerates the six canonical forms.

**Non-goals (explicit, from the ticket "Out of scope").**
- Fixing mika-qa's specific #1821 finding (`mika_qa.rs incompletely wired`, DR-5) — separate ticket.
- Widening the `DEPTH:` parser (same file, different regex) — no drift evidence there yet.
- Adding new *semantic* verdict classes — this ticket is format tolerance for existing classes.
- Reinforcing the qa-review prompt with new re-prompt exits (the ticket lists this as an
  option but the recommended combination is parser-widen + calibration + confirm-guard;
  prompt hard-failure is not pulled — see Decision D4).

## Design decisions

### D1 — Markdown-bold tolerance strategy (AC1): widen the regex, strip on the captured line

Two viable approaches:

- **(a) Regex-level** — allow an optional `**` (or `__`) before `VERDICT` and strip a
  trailing `**`/`__` from the captured tail.
- **(b) Value-level** — keep `VERDICT_RE` anchored, but first normalize the body by
  stripping leading/trailing markdown emphasis per line before matching.

**Chosen: (a) — widen `VERDICT_RE`.** Change the anchor to permit optional leading
emphasis and capture the tail, then strip a trailing emphasis run from the trimmed value
inside `parse_verdict`. Rationale: the regex is the single authority for "which line is
the verdict line"; keeping the tolerance there avoids a whole-body normalization pass and
keeps the "first VERDICT line wins" semantics (regression-tested by
`parse_verdict_multiple_lines_first_wins`). Concretely:

- New regex: `(?mi)^\s*(?:\*\*|__)?\s*VERDICT:\s*(.+?)\s*(?:\*\*|__)?\s*$`
  - `^\s*` — tolerate leading indentation (bold list items, blockquotes are handled by
    the caller's line-splitting; leading spaces are common).
  - `(?:\*\*|__)?` — optional opening emphasis run.
  - `(.+?)` — **non-greedy** capture so the trailing `(?:\*\*|__)?` can peel the closing
    emphasis instead of the greedy `.+` swallowing it.
  - trailing `\s*(?:\*\*|__)?\s*$` — optional closing emphasis + trailing space.
- Defense-in-depth: after capture, also `trim()` and strip any residual leading/trailing
  `*`/`_` run in code (covers `*Verdict:*` single-emphasis and stray asterisks the regex
  alternation misses). A small `strip_md_emphasis(&str) -> &str` helper.

**Risk & mitigation.** A non-greedy capture with an optional trailing group must not
truncate a legitimate `block[ac]` whose value contains no trailing emphasis. Covered by
keeping all existing tests green + adding explicit `**VERDICT: block[ac]**` and
`VERDICT: block[ac]` (no-emphasis) cases (AC3).

### D2 — Alias mapping (AC2): a normalize-then-match alias table, applied before Missing fallthrough

After the existing `pass` / `BLOCK_RE` / `HOLD_RE` checks fail, run the trimmed value
through an alias normalizer before returning `Missing`:

- Normalize: lowercase, collapse internal whitespace and `_`/`-` to a single space,
  trim. So `REQUEST CHANGES`, `REQUEST_CHANGES`, `CHANGES_REQUESTED`,
  `changes-requested` all normalize to `request changes` / `changes requested`.
- Alias table (deny-by-default; only these map):
  - `request changes`, `request change`, `changes requested` → `Verdict::Block("ac")`
  - `approve`, `approved` → `Verdict::Pass`
- Anything still unmatched → `Verdict::Missing { truncated }` (unchanged behavior — real
  drift still surfaces).

**Why `block[ac]` for REQUEST_CHANGES (from AC2).** `ac` is the safe default: it triggers
a bounded-retry autonomous AC-fix dispatch (max 3, then escalate), unlike
`block[security]`/`block[pipeline]` which halt with operator notify. Choosing `ac` keeps
the loop moving while staying conservative — an AC-fix dispatch on a genuinely
security-blocking review is caught by the human-review + CI gates downstream, whereas
mis-halting a routine change-request needs a manual unblock (the exact failure we are
fixing).

**Risk (from the ticket): alias mapping could mask model drift instead of surfacing it.**
Mitigation: (1) keep the alias set *minimal* — only GitHub's own review-state vocabulary,
not arbitrary synonyms; (2) emit a structured `verdict_alias_normalized` INFO log
(fields: `pr_number`, `repo`, `raw_value`, `mapped_to`) whenever an alias path fires, so
operators can measure alias-hit frequency and detect systemic drift; (3) AC4's calibration
scenario asserts the model *still* emits canonical shape, so alias tolerance is the safety
net, not a license for drift.

### D3 — AC5 is already satisfied by `required_tool_arg_suffixes`; verify, don't duplicate

AC5 asks that `qa-review/skill.toml` `[output] required_suffix_lines` enumerate the six
canonical VERDICT forms "if not already". Inspection shows the skill already enforces this
via a **more precise** mechanism — `[[output.required_tool_arg_suffixes]]` on the
`run_gh` `pr_review_body` argument (mika#899), listing all six canonical lines:

```toml
[[output.required_tool_arg_suffixes]]
tool = "run_gh"
arg = "pr_review_body"
required_lines = [
  "VERDICT: pass", "VERDICT: hold[review]", "VERDICT: block[ac]",
  "VERDICT: block[ci]", "VERDICT: block[security]", "VERDICT: block[pipeline]",
]
```

This is the correct guard for qa-review (a dispatcher-shaped skill whose verdict lands in
a `run_gh pr review --body` argument, not in the agent's final EndTurn text), and it
already validates the body **before** subprocess spawn — rejecting a non-canonical body
with a corrective error. `required_suffix_lines` (the EndTurn-text guard #8) is the wrong
mechanism here: qa-review's verdict is a tool argument, not the last line of the assistant
turn.

**Decision:** AC5 is satisfied. The implementation step is to (a) confirm the six lines
are present and exact, and (b) record in the plan/PR body that AC5 is met by
`required_tool_arg_suffixes`, not `required_suffix_lines`, with the justification above. No
skill.toml change unless the enumeration is found incomplete. This is a documented,
deliberate divergence from the AC's literal mechanism name — surfaced explicitly rather
than silently adding a redundant (and shape-wrong) guard.

### D4 — Do not pull the prompt-hard-failure option

The ticket lists "reinforce mika-qa prompt with a hard failure exit" as an option. It is
**not** pulled: (1) `feedback_prompt_enforcement_fragile` — prompt-only fixes drift under
load at the substrate layer; (2) the engine already has the structural guard (D3); (3) the
recommended combination in the ticket is parser-widen + calibration + confirm-guard. The
parser widening is the immediate structural unblock; calibration is the drift-prevention
net. Adding re-prompt exits risks the cost/latency budget the ticket itself flags.

### D5 — AC4 scenario is a *new* scenario, distinct from `verdict_format_precision`

`verdict_format_precision` already asserts exact `VERDICT: pass` casing/spacing on the
all-ACs-satisfied fixture. AC4 wants `verdict_format_canonical_shape`: a scenario asserting
the emitted body **starts** with `VERDICT:` on a line (no `**` prefix) and the first
`VERDICT:` line matches the canonical BNF exactly. This specifically targets the
markdown-bold drift the parser now tolerates — the calibration net ensures the model keeps
emitting canonical shape so alias/bold tolerance stays a safety margin, not a crutch.
Adding it takes the suite from 6 → 7 scenarios; the `scenario_count_is_six` test updates to
`scenario_count_is_seven` (value 7).

## Implementation steps

### Step 1 — Widen `VERDICT_RE` + add emphasis-strip helper (AC1)

`crates/mika-agent/src/server/verdict.rs`:

- Replace `VERDICT_RE` with the widened pattern from D1
  (`(?mi)^\s*(?:\*\*|__)?\s*VERDICT:\s*(.+?)\s*(?:\*\*|__)?\s*$`).
- Add a private `strip_md_emphasis(value: &str) -> &str` helper that trims a
  leading/trailing run of `*`/`_` characters (defense-in-depth for single-emphasis and
  stray-asterisk shapes the alternation misses), then apply it to the captured value
  before the existing `pass`/`BLOCK_RE`/`HOLD_RE` checks.
- Keep `truncated` detection and "first match wins" semantics unchanged.

### Step 2 — Add alias normalizer + table (AC2)

`crates/mika-agent/src/server/verdict.rs`:

- Add a private `normalize_alias(value: &str) -> String` (lowercase; collapse `[_\-\s]+`
  to single space; trim).
- Add `alias_to_verdict(normalized: &str) -> Option<Verdict>` with the D2 table
  (`request changes`/`request change`/`changes requested` → `Block("ac")`;
  `approve`/`approved` → `Pass`).
- In `parse_verdict`, after the `HOLD_RE` check and before the `Missing` fallthrough, run
  the (emphasis-stripped, trimmed) value through `normalize_alias` → `alias_to_verdict`;
  on `Some`, log `verdict_alias_normalized` (INFO, fields per D2) and return the mapped
  verdict. On `None`, fall through to `Missing` as today.
- `use tracing::info;` if not already imported in the module.

### Step 3 — Unit tests (AC3)

`crates/mika-agent/src/server/verdict.rs` `mod tests` — add, for each affected canonical
class, both new shapes:

- Markdown-bold: `**VERDICT: pass**`, `**Verdict: block[ac]**`, `**VERDICT: block[ci]**`,
  `**VERDICT: block[security]**`, `**VERDICT: block[pipeline]**`, `**VERDICT: hold[review]**`.
- Single-emphasis / stray: `*VERDICT: pass*`, `__VERDICT: block[ci]__`.
- Alias: `VERDICT: REQUEST CHANGES` → `Block("ac")`, `VERDICT: REQUEST_CHANGES` →
  `Block("ac")`, `VERDICT: CHANGES_REQUESTED` → `Block("ac")`,
  `VERDICT: changes-requested` → `Block("ac")`, `VERDICT: APPROVE` → `Pass`,
  `VERDICT: approved` → `Pass`.
- Combined (the #1821 shape): `**Verdict: REQUEST CHANGES**` → `Block("ac")` — the founding
  regression case; add an explicit test named for #1821.
- Negative guards (must stay `Missing`): `VERDICT: frobnicate`, `VERDICT: **block[ac]`
  (unbalanced — still parse to `block[ac]` via strip; assert intended outcome), and
  confirm no-emphasis `VERDICT: block[ac]` still parses (non-greedy-capture regression).
- Preserve all existing tests unchanged (green).

### Step 4 — Calibration scenario `verdict_format_canonical_shape` (AC4)

- `crates/mika-agent/src/calibration/roles/mika_qa.rs`:
  - Add a `RoleScenario` entry `verdict_format_canonical_shape` (tags e.g.
    `["verdict", "format", "canonical", "drift"]`, `weight` ~1.5,
    `expected_failure_classes_absent: &["ContractViolation", "EmptyResponse"]`).
  - Add the `run_scenario` match arm + `run_verdict_format_canonical_shape` async fn
    following the file's established pattern. Assertions: (a) response non-empty; (b) some
    line, after `trim()`, **starts with** `VERDICT:` (no `**`/`*`/`__` prefix) — reject a
    body whose only VERDICT line is markdown-wrapped; (c) the first such `VERDICT:` line's
    tail matches the canonical BNF exactly (`pass` | `hold[<x>]` | `block[<x>]` for the
    six known class-details) — reuse/borrow the same class validation shape as the parser.
  - Update `scenario_count_is_six` → `scenario_count_is_seven` (assert `7`).
- New fixture `tests/eval/calibration_fixtures/mika-qa/verdict_format_canonical_shape.md` —
  a PR-review input that would tempt a markdown-bold verdict (e.g. a body context heavy in
  markdown headings), with a system-prompt instruction (in the run fn) mandating canonical
  VERDICT-first shape.
- `tests/eval/calibration_fixtures/mika-qa/manifest.yaml` — add the matching scenario entry
  (id, fixture, tags, flaky, weight, description, `expected_failure_classes_absent`) to keep
  the YAML companion in parity with the `SCENARIOS` array.

### Step 5 — Confirm AC5 guard enumeration (AC5)

- Re-read `skills/bundled/qa-review/skill.toml`; confirm the
  `[[output.required_tool_arg_suffixes]]` block lists all six canonical `VERDICT:` lines
  exactly. No edit expected. Record the AC5-satisfied-by-`required_tool_arg_suffixes`
  justification (D3) in the PR body.

### Step 6 — Build, lint, test

- `cargo test -p mika-agent verdict` (unit) + full `cargo test -p mika-agent` for the
  calibration `mod tests` (scenario-count/tags/fixture parity — these run under the unit
  test tier, no API keys needed).
- `cargo clippy -p mika-agent` and `cargo fmt`.
- The real-provider calibration run (`make calibrate-mika-qa MODEL=...`) is an operator
  step (requires API keys); the new scenario's *structural* wiring (array, match arm,
  fixture presence, manifest parity, count test) is what CI verifies.

## Verification contract

- **Unit (CI, no keys):** all new `verdict.rs` tests pass; every pre-existing `verdict.rs`
  test still passes (no regression to "first match wins", truncated detection, or
  no-emphasis parsing). `mika_qa.rs` `mod tests` pass with `scenario_count_is_seven`,
  unique-ids, tags-present, and the new fixture's existence assertion (if added).
- **Static parity:** `manifest.yaml` scenario list matches the `SCENARIOS` array
  (7 entries, ids aligned).
- **Lint/format:** `cargo clippy -p mika-agent` clean; `cargo fmt` clean.
- **Behavioral (the #1821 regression):** `parse_verdict("**Verdict: REQUEST CHANGES** ...")`
  returns `Verdict::Block("ac")`, so the handler dispatches the AC-fix path instead of
  `handle_missing_verdict`.
- **Observability:** an alias-path parse emits exactly one `verdict_alias_normalized` INFO
  event with `raw_value` + `mapped_to`.

## Definition of Done

- `parse_verdict` accepts markdown-bold-wrapped VERDICT lines and the enumerated
  GitHub-review-state aliases, mapping them to the correct canonical `Verdict`, while
  genuinely-unrecognized values still return `Verdict::Missing`.
- Unit tests cover both new shapes for each affected canonical class, including the
  combined #1821 shape, plus negative guards.
- A new `verdict_format_canonical_shape` mika-qa calibration scenario exists and is wired
  into `SCENARIOS`, `run_scenario`, `manifest.yaml`, and a fixture; the scenario-count test
  reflects 7 scenarios.
- AC5 is confirmed satisfied by `required_tool_arg_suffixes` (six canonical lines present),
  with the mechanism-divergence justification recorded in the PR body.
- `cargo test -p mika-agent`, `cargo clippy -p mika-agent`, and `cargo fmt --check` all pass.
- AC6 (post-deploy retrigger of a stuck PR's review webhook) is documented as an operator
  step in the PR body — it is out of code scope and cannot be executed from the branch.

## Acceptance criteria

Transcribed verbatim from mika#1828:

- **AC1** — `parse_verdict` in `crates/mika-agent/src/server/verdict.rs` accepts
  markdown-wrapped forms (`**VERDICT: pass**`, `**Verdict: block[ac]**`, etc.) — regex
  strips leading `**` prefix and trailing `**` suffix on the matching line.
- **AC2** — `parse_verdict` maps GitHub-review-state-adjacent aliases to canonical classes:
  - `REQUEST CHANGES` / `REQUEST_CHANGES` / `CHANGES_REQUESTED` (case-insensitive,
    whitespace-tolerant) → `Verdict::Block("ac")` (default AC class, safer than
    security/pipeline)
  - `APPROVE` / `APPROVED` → `Verdict::Pass`
- **AC3** — Unit tests in verdict.rs mod tests cover both new shapes for each canonical class.
- **AC4** — mika-qa calibration scenario added: `verdict_format_canonical_shape` in
  `crates/mika-agent/src/calibration/roles/mika_qa.rs`. Asserts the emitted review body
  starts with `VERDICT:` on a line (no `**` prefix), first `VERDICT:` line matches the
  canonical BNF exactly.
- **AC5** — mika-qa `qa-review/skill.toml` `[output] required_suffix_lines` extended (if not
  already) to enumerate all six canonical VERDICT class-detail combinations — engine guard
  #8 rejects EndTurn on missing/non-canonical.
  - *Plan note (D3):* satisfied already via `[[output.required_tool_arg_suffixes]]` on the
    `pr_review_body` arg (the correct guard for a tool-argument verdict); the six canonical
    lines are present. No change unless the enumeration is found incomplete.
- **AC6** — Post-deploy: retrigger a review-requested webhook on PR #1821 (or any stuck PR)
  that produced non-canonical verdict; confirm the widened parser now classifies the
  existing review correctly and dispatches dev-revise / merges via `verdict_handler`. If AC5
  is in effect, re-triggering the fresh mika-qa review should also emit canonical shape.
  - *Plan note:* out of code scope — operator/post-deploy verification, documented in PR body.

## References

- `crates/mika-agent/src/server/verdict.rs:48-49` (`VERDICT_RE`), `:108-134` (`parse_verdict`)
- `crates/mika-agent/src/server/verdict_handler.rs` (`Verdict::Missing` → `handle_missing_verdict`)
- `crates/mika-agent/src/calibration/roles/mika_qa.rs` (mika-qa scenario suite, #1632)
- `crates/mika-agent/tests/eval/calibration_fixtures/mika-qa/manifest.yaml`
- `skills/bundled/qa-review/skill.toml` (`required_tool_arg_suffixes`, mika#899)
- PR mika#1821 review body (founding evidence)
- `feedback_prompt_enforcement_fragile` (why prompt-only substrate fixes fail — motivates D4)
- `feedback_project_mika_owned_model_dev_qa_quality_first` (mika-qa on glm-5.2 — drift source)
