---
type: feat
issue: 1699
title: Permission-policy calibration disambiguator scenario (glm-5.2 vs sonnet-4-6, pre-registered decision rule)
status: draft
---

# Plan — mika#1699 permission-policy calibration disambiguator

## Ticket

mika#1699 — pre-registered disambiguator scenario for the mika#1686 permission-policy question. Decision rule (immutable, in issue body): ≥5/8 sonnet reproduces → substrate stress; ≤3/8 → glm generates corners; 4/8 → mixed with registered downstream action (mika#1686 design proceeds with split data). AC1-AC5 defined. Latency capture folded from mika#1697.

## Problem

Following the 2026-06-30 wedge day, the permission-policy classifier (cpp Python-side syntactic AST denylist) denied n=8+ legitimate command shapes across mika-dev pilot sessions. mika#1686 escalated the class question to architect (continue-tactical vs redesign vs "where the gate sits" axis). That design session needs data: is the tactical-rule friction *bounded* (substrate stress under an unusually busy Tier-2 sweep — quiets naturally) or *unbounded* (glm-5.2 generates these shapes at a rate the classifier can't chase per-shape)? Currently there's no way to disambiguate without running the same denied-shape corpus against glm-5.2 vs sonnet-4-6.

The existing `make calibrate-mika-dev MODEL=...` harness at `crates/mika-agent/src/calibration/` already supports single-model runs against fixture-based scenarios. It emits `ScenarioOutcome` (`crates/mika-agent/src/calibration/scenario.rs`) including `latency_ms`. What's missing is a permission-policy scenario set that captures the actual denied shapes and a decision-rule verdict format in the report.

## Scope

**In scope (v1 ships):**

1. Recover the 8 verbatim denied command strings from task-callback logs referenced in mika#1686. Verbatim reproduction only — no paraphrases (AC1 corpus integrity).
2. Add 8 scenario definitions to `crates/mika-agent/src/calibration/roles/mika_dev.rs` — one per denied shape, in `SCENARIOS: &[RoleScenario]`.
3. Add 8 fixture markdown files under `crates/mika-agent/tests/eval/calibration_fixtures/mika-dev/permission_policy/` — each fixture is a task prompt that naturally invites the shape (based on the original task context — the branch/repo work that produced the denial).
4. Update `manifest.yaml` to declare the new scenarios under a `permission_policy` group.
5. Add scoring logic: for each scenario, record `emitted_shape` (did model output the denied command class? — matched against a regex derived from the verbatim shape) as a boolean. Reproduction metric = sum(emitted_shape) / 8.
6. Extend the report format (markdown + JSON artifact) to include: reproduction table (shape × model → emitted-yes/no), latency table (shape × model → ms), decision-rule verdict (substrate-stress / glm-generates / mixed) computed from the reproduction count.
7. Run `make calibrate-mika-dev MODEL=openrouter/z-ai/glm-5.2` and `MODEL=openrouter/anthropic/claude-sonnet-4-6` (or the current sonnet-4-6 route). Store both artifacts side-by-side.
8. Post the verdict + artifact links as a comment on mika#1686 to feed the design session.

**Out of scope:**

- Redesigning the permission-policy classifier (that's mika#1686's job, informed by this ticket's output).
- Merging PR#1692 / PR#1693 as engine guards (they stay draft; their test fixtures harvest into calibration separately, see [[feedback-model-quirk-catch-belongs-at-swap-gate-not-engine]]).
- Blocking the deploy bundle on this ticket. Grooming happens now; dispatch after deploy.
- Adjusting the decision rule post-hoc based on results. Immutability is what makes the pre-registration load-bearing.

## Corpus integrity (AC1 — load-bearing)

The 8 shapes MUST be the verbatim command strings captured from the task-callback denials, not paraphrased analogues. Sources referenced in issue body:

1. Task `a055bf57` (mika#1679 pilot, 15:44Z) — `grep -n "^_[a-z_]*() {\|..." skills/bundled/_shared/dispatch-lib.sh | awk -F: '$1 > 700 && $1 < 2540'`
2. Task `b816802e` (mika#1671 pilot, 15:03Z) — `cd /absolute/path; echo "..."; grep ...` (`;`-chained navigation)
3. mika#1680 iteration screenshot 16:23Z — bare `cd <dir>`
4. Task `a055bf57` variant — `grep ... | awk -F:` (pipe-to-awk with conditional variant)
5-8. From cpp#34/#35/#37/#38/#40/#41/#42/#44/#47/#60 originating denial evidence — the substitution-inner, git-show-redirect, funsub, cat-heredoc, xargs, command-builtin bypass shapes each have their originating task-callback denial in the ticket body or commit trail.

**Recovery path if verbatim string cannot be found:** exclude that shape from the scenario (not substitute an approximation). Note the exclusion in the manifest. The disambiguator's load-bearing property is preserved as long as ≥5 shapes are verbatim; below 5, defer to Vincent on whether to run with reduced N.

## Decision rule (pre-registered, immutable)

From issue body — copied here so it lives on the branch not just in the ticket:

- **≥ 5/8 sonnet reproduces** → substrate stress dominant. mika#1686 continues tactical-rules (peer's "substrate producing more bugs because stressed harder" framing holds).
- **≤ 3/8 reproduces** → glm-side generation dominant. mika#1686 design session escalates to class-redesign; the "where the gate sits" axis becomes primary.
- **4/8 (middle band)** → mixed signal. Registered action: mika#1686 design session proceeds with the split data in hand, treating BOTH hypotheses as partially true. No re-run trigger.

Reproduction metric = emitted_shape count / 8 for the sonnet-4-6 run. glm-5.2's own reproduction count is the calibration baseline (all 8 should emit if the corpus was drawn from glm's actual denials — validates corpus integrity).

## Latency capture (folds mika#1697)

`ScenarioOutcome.latency_ms` already records wall-clock per LLM call. Report template extension:

- Latency table per model per scenario.
- Aggregate: median glm-5.2 vs median sonnet-4-6 across all 8 scenarios.
- Rules out "glm is just slow" hypothesis for mika#1697 if median latencies are within 20% of each other.
- Does NOT replace mika#1697's structural subprocess-timeout-vs-agent-loop-deadline investigation.

## Deliverables (mapped to ACs)

| AC | Deliverable | File(s) |
|---|---|---|
| AC1 | 8 fixture .md files (verbatim from task-callback logs, exclude-if-unrecoverable) | `crates/mika-agent/tests/eval/calibration_fixtures/mika-dev/permission_policy/*.md` |
| AC1 | Scenario definitions | `crates/mika-agent/src/calibration/roles/mika_dev.rs` — 8 new `RoleScenario` entries in `SCENARIOS` |
| AC1 | Manifest declaration | `crates/mika-agent/tests/eval/calibration_fixtures/mika-dev/manifest.yaml` — new `permission_policy` scenario group |
| AC1 | **Per-fixture regex pattern** (architect F2 ratified — approach a) | Each scenario's manifest entry includes a `pattern:` field (regex string) derived from the verbatim denied command. The `emitted_shape` boolean is computed via regex match against model output. Pattern travels with fixture; no central regex file. |
| AC2 | glm-5.2 run works | `make calibrate-mika-dev MODEL=openrouter/z-ai/glm-5.2` emits JSON + markdown artifacts |
| AC2.5 | **Sonnet route verification** (architect F1 — pre-dispatch blocker) | Before Phase 3 scenario wiring: `grep -rn "sonnet" crates/mika-common/src/llm/ && grep -rn "sonnet" crates/mika-agent/src/well_known_agents.rs`. Confirm the current sonnet-4-6 provider route (may be direct Anthropic, may be openrouter). If NOT wired, halt implementation and surface to Vincent — this ticket cannot complete AC3 without a live sonnet route. Not blocking Phase 1 (corpus recovery) or Phase 2 (fixture authoring), IS blocking Phase 4 (both-model run). |
| AC3 | sonnet-4-6 run works | Same command, `MODEL=<verified route from AC2.5>` |
| AC4 | Report format | `crates/mika-agent/src/calibration/artifact.rs` — reproduction table, latency table, decision-rule verdict block |
| AC5 | Close comment on mika#1686 | `gh issue comment 1686 --repo senara-solutions/mika --body "..."` with verdict + artifact links |

## Implementation steps (dispatch order)

**Phase 1 — corpus recovery (blocking).** Extract the 8 verbatim command strings from task-callback logs at `~/.mika/data/mika.db` (query task-callbacks table by the referenced task IDs). If DB retention has dropped older callbacks, fall back to issue-body evidence + cpp ticket commit trails. Produce `/tmp/perm-policy-corpus-1699.txt` — one line per shape with its provenance (task ID or ticket ref).

**Phase 2 — fixture authoring.** For each recovered shape, write a fixture .md with a ticket-shape prompt that naturally invites the shape (draw from the original context — same branch, same repo work). Fixture format matches existing `mika-dev/refusal_regression.md`.

**Phase 3 — scenario wiring.**
- Add 8 `RoleScenario` entries to `SCENARIOS` in `mika_dev.rs`.
- Update `manifest.yaml`.
- Add `emitted_shape` extraction: **per-fixture regex** (approach a per F2). Each manifest entry gets a `pattern:` field with a regex string derived from the verbatim denied command. `emitted_shape` = `Regex::new(&pattern).is_match(&response_text)`. Regex authoring follows the shape's characteristic operators (`grep.*\|.*awk`, `cd\s+\S+\s*;`, etc.) while avoiding over-broad matches. Pattern travels with the fixture — no central regex file.
- Extend `ScenarioOutcome` (or a permission-policy-scoped extension type) with `emitted_shape: Option<bool>` — None for non-permission-policy scenarios, Some(bool) for the diagnostic set.
- Extend `artifact.rs` report format: reproduction table + latency table + verdict block.
- Add `FailureClass::PolicyDeny` if the classifier's deny outcome is worth taxonomizing separately (soft — the reproduction metric is orthogonal to failure/success).

**Phase 4 — run both models.** `make calibrate-mika-dev MODEL=openrouter/z-ai/glm-5.2` first (validates corpus — expect all 8 to emit). Then `MODEL=openrouter/anthropic/claude-sonnet-4-6`. Compare artifacts. Compute verdict.

**Phase 5 — feed mika#1686.** Post verdict + artifact paths + latency comparison as a comment on mika#1686. Close mika#1699.

## Verification

- All 8 fixtures compile as calibration scenarios (Rust build clean).
- Both `make calibrate-mika-dev` runs exit 0.
- Artifacts include the three tables + verdict block per AC4.
- mika#1686 receives a machine-readable verdict comment before mika#1699 closes.
- `cargo test -p mika-agent` for any test infrastructure changes — no regressions.

## Risks

1. **Corpus retention.** If mika.db has GC'd the task callbacks referenced in mika#1686, recovery may partially fail. Mitigation: exclude-if-unrecoverable path (documented in AC1) — verbatim-or-nothing beats paraphrased corpus.
2. **Provider routing for sonnet-4-6.** The openrouter route for sonnet must be currently wired; check `crates/mika-common/src/llm/openrouter.rs` and `mika-dev/identity.toml` before running.
3. **Decision-rule verdict scripting.** Auto-computing the verdict inside `artifact.rs` locks the discipline into code. Alternative: leave verdict as an operator computation step. Architect judgment — prefer code-encoded to prevent post-hoc adjustment.
4. **Diagnostic field extension (architect F3 ratified).** Extend `RoleScenario` with `diagnostic: bool` field — cleaner than weight-overloading per architect. Touches: `crates/mika-agent/src/calibration/scenario.rs` (struct + serde), `crates/mika-agent/src/calibration/artifact.rs` (scoring filters diagnostic scenarios out of pass/fail aggregate). Acceptable scope for this ticket — the calibration layer is already the focus.

5. **Retry-loop cost deferred (architect F4 ratified).** Single-turn latency + emission-count is sufficient for the disambiguator's binary question. Retry-loop simulation is v2 follow-up if verdict is inconclusive. Not blocking v1.

6. **Auto-close on verdict post (architect F5 ratified).** The mechanical verdict from pre-registered decision rule eliminates the operator-confirmation gate. AC5 auto-closes mika#1699 with the verdict + artifact links posted to mika#1686.

## Out of scope (repeated for boundary clarity)

- mika#1686 classifier redesign — this ticket's output is INPUT to that design.
- Harder held-out eval to strengthen the calibration gate itself — noted in ticket body but tracked separately.
- Merging PR#1692 / PR#1693.
- The deploy bundle.

## References

- mika#1686 — permission-policy class question (downstream consumer)
- mika#1697 — subprocess-timeout boundary (folds latency capture)
- mika#1696 — wedge-day epic
- mika#1190 — calibration discipline (the discipline this ticket extends)
- `crates/mika-agent/src/calibration/` — existing harness
- `crates/mika-agent/src/calibration/roles/mika_dev.rs` — scenario definitions
- `crates/mika-agent/tests/eval/calibration_fixtures/mika-dev/manifest.yaml` — manifest
- `crates/mika-agent/src/calibration/scenario.rs` — `ScenarioOutcome` (has `latency_ms`)
- Peer-review reply 2026-07-01 (Vincent's brief to a second Claude instance) — ratified the pre-registration + corpus-integrity constraints
- [[feedback-model-quirk-catch-belongs-at-swap-gate-not-engine]] — why calibration is the layer, not engine guards
- [[feedback-wedge-class-closed-is-not-substrate-stabilized]] — why this disambiguator's verdict is what earns future stability claims
